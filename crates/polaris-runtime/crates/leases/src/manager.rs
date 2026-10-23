//! Lease state, FIFO admission, TTL cleanup, OCC tickets, and churn revocation.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

#[cfg(all(feature = "loom", test))]
use loom::sync::{Mutex as StateMutex, MutexGuard as StateMutexGuard};
#[cfg(not(all(feature = "loom", test)))]
use std::sync::{Mutex as StateMutex, MutexGuard as StateMutexGuard};

use arc_swap::ArcSwap;
use polaris_sdk::{
    error::{ErrorKind, HasKind},
    ids::{Cid, FrameId},
};
use tokio::sync::{broadcast, watch};

use super::{
    key::LeaseKey,
    scope::{LeaseScope, scopes_conflict, scopes_overlap},
};

const MAX_REQUESTS_PER_ACQUIRE: usize = 256;
const MAX_COMPONENTS_PER_REQUEST: usize = 128;
const INVALIDATION_CAPACITY: usize = 256;
const MAX_CHURN_RECORDS: usize = 65_536;
const MAX_SUBTASK_LEN: usize = 256;
const HARD_MAX_ACTIVE: usize = 16_384;
const HARD_MAX_WAITERS: usize = 65_536;

/// Opaque identifier for one granted lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LeaseId(u64);

impl LeaseId {
    /// Returns the stable identifier within this manager.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why a lease's write authority was revoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokeReason {
    /// The owner repeatedly edited one file in the same subtask.
    ChurnLimit,
    /// The owning frame was explicitly revoked.
    FrameRevoked,
    /// The lease reached its configured TTL.
    Expired,
}

/// Bounded behavior for a lease manager.
#[derive(Debug, Clone)]
pub struct LeaseConfig {
    /// Maximum number of simultaneously granted leases.
    pub max_active_leases: usize,
    /// Maximum number of queued acquire operations.
    pub max_waiters: usize,
    /// Maximum wait for FIFO admission.
    pub acquire_timeout: Duration,
    /// Maximum time a granted lease remains valid without renewal.
    pub lease_ttl: Duration,
    /// Number of edits permitted before a later edit revokes the writer.
    pub edits_before_revoke: u32,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            max_active_leases: 1024,
            max_waiters: 4096,
            acquire_timeout: Duration::from_secs(30),
            lease_ttl: Duration::from_secs(300),
            edits_before_revoke: 3,
        }
    }
}

/// Request for one file or AST-symbol lease.
#[derive(Debug, Clone)]
pub struct LeaseRequest {
    /// Frame that will own the grant.
    pub agent: FrameId,
    /// Canonical workspace-relative key.
    pub key: LeaseKey,
    /// Requested exclusive scope.
    pub scope: LeaseScope,
    /// BLAKE3 CAS id of the content the edit is based on; writes require this value.
    pub base: Option<Cid>,
}

/// A fresh-content notice emitted when an OCC check fails, a lease expires, or a frame is revoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalidation {
    /// Affected lease owner.
    pub agent: FrameId,
    /// Affected file key.
    pub key: LeaseKey,
    /// Affected lease id.
    pub lease: LeaseId,
    /// Current content id if supplied by the caller.
    pub current: Option<Cid>,
    /// Revocation reason, if this was not an OCC mismatch.
    pub reason: Option<RevokeReason>,
}

/// Failure to acquire or validate a lease.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    /// Conflicting lease or queued request; `holder` is absent when only queued work blocks it.
    #[error("lease is busy")]
    Busy { holder: Option<FrameId> },
    /// A write's observed base differs from the current file content.
    #[error("stale base content id")]
    StaleBase { expected: Cid, actual: Cid },
    /// The owning frame or lease has been revoked.
    #[error("lease was revoked: {0:?}")]
    Revoked(RevokeReason),
    /// The acquire operation exceeded its configured wait duration.
    #[error("lease acquisition timed out")]
    Timeout,
    /// Path string is not a safe workspace-relative key.
    #[error("invalid workspace-relative path")]
    InvalidPath,
    /// Scope components are malformed or excessive.
    #[error("invalid AST scope")]
    InvalidScope,
    /// A configured or hard bound was exceeded.
    #[error("lease limit exceeded")]
    LimitExceeded,
    /// Multi-acquire contains duplicate or overlapping requests.
    #[error("multi-acquire contains overlapping requests")]
    OverlappingRequests,
    /// No base content id was supplied for this write lease.
    #[error("write lease has no base content id")]
    MissingBase,
    /// Guard does not belong to this manager or is no longer active.
    #[error("lease guard is not active in this manager")]
    InvalidGuard,
    /// Manager identifier space exhausted.
    #[error("lease identifier space exhausted")]
    IdExhausted,
}

impl HasKind for LeaseError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Busy { .. } | Self::Timeout => ErrorKind::Transient,
            Self::StaleBase { .. } | Self::Revoked(_) => ErrorKind::Policy,
            Self::InvalidGuard | Self::IdExhausted => ErrorKind::Fatal,
            Self::InvalidPath
            | Self::InvalidScope
            | Self::LimitExceeded
            | Self::OverlappingRequests
            | Self::MissingBase => ErrorKind::Permanent,
        }
    }
}

/// Outcome from recording an edit against a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChurnVerdict {
    /// Edit count remains below the warning threshold.
    Ok,
    /// The writer is at the configured limit but remains allowed for this edit.
    Warn { edits: u32 },
    /// The edit crossed the limit; write rights were revoked and escalation is required.
    Revoke { edits: u32, escalate_to_s2: bool },
}

/// Unforgeable write authorization tied to manager, owner, lease, and base Cid.
#[derive(Debug, Clone)]
pub struct WriteTicket {
    manager: Weak<()>,
    lease: LeaseId,
    agent: FrameId,
    base: Cid,
}

impl WriteTicket {
    /// Returns the lease this ticket authorizes.
    #[must_use]
    pub const fn lease(&self) -> LeaseId {
        self.lease
    }
    /// Returns the frame authorized to apply this write.
    #[must_use]
    pub const fn agent(&self) -> FrameId {
        self.agent
    }
    /// Returns the exact content id against which the write was checked.
    #[must_use]
    pub const fn base(&self) -> Cid {
        self.base
    }
}

#[derive(Clone)]
struct Holder {
    id: LeaseId,
    agent: FrameId,
    key: LeaseKey,
    scope: LeaseScope,
    expires: Instant,
}

struct LeaseRecord {
    holder: Holder,
    base: Option<Cid>,
    expires: Instant,
    write_allowed: bool,
}

struct Waiter {
    requests: Vec<LeaseRequest>,
    granted: Option<Vec<LeaseId>>,
}

#[derive(Default)]
struct State {
    active: Vec<LeaseRecord>,
    order: VecDeque<u64>,
    waiters: HashMap<u64, Waiter>,
    churn: HashMap<(FrameId, LeaseKey, String), u32>,
    next_ticket: u64,
    next_lease: u64,
}

struct Inner {
    config: LeaseConfig,
    state: StateMutex<State>,
    changed: watch::Sender<u64>,
    holders: ArcSwap<HolderSnapshot>,
    invalidations: broadcast::Sender<Invalidation>,
    ticket_token: Arc<()>,
}

#[derive(Default)]
struct HolderSnapshot {
    holders: Vec<Holder>,
    ranges: HashMap<LeaseKey, std::ops::Range<usize>>,
}

/// Shared lease manager. `holder_of` reads a published immutable snapshot without taking the
/// state mutex or allocating; mutations serialize under the state mutex.
#[derive(Clone)]
pub struct LeaseManager {
    inner: Arc<Inner>,
}

impl LeaseManager {
    /// Creates a manager with bounded queues and an invalidation broadcast channel.
    #[must_use]
    pub fn new(config: LeaseConfig) -> Self {
        let (invalidations, _) = broadcast::channel(INVALIDATION_CAPACITY);
        let (changed, _) = watch::channel(0_u64);
        let config = LeaseConfig {
            max_active_leases: config.max_active_leases.min(HARD_MAX_ACTIVE),
            max_waiters: config.max_waiters.min(HARD_MAX_WAITERS),
            acquire_timeout: config.acquire_timeout.min(Duration::from_secs(3_600)),
            lease_ttl: config.lease_ttl.min(Duration::from_secs(86_400)),
            edits_before_revoke: config.edits_before_revoke,
        };
        Self {
            inner: Arc::new(Inner {
                config,
                state: StateMutex::new(State::default()),
                changed,
                holders: ArcSwap::from_pointee(HolderSnapshot::default()),
                invalidations,
                ticket_token: Arc::new(()),
            }),
        }
    }

    /// Subscribes to stale-base, expiry, churn, and explicit-revocation notices.
    #[must_use]
    pub fn subscribe_invalidations(&self) -> broadcast::Receiver<Invalidation> {
        self.inner.invalidations.subscribe()
    }

    /// Acquires one lease in the shared FIFO queue.
    ///
    /// Cancellation and timeout remove the queued request; no partially granted lease survives.
    ///
    /// # Errors
    /// Returns a typed validation, capacity, timeout, or admission error.
    pub async fn acquire(&self, request: LeaseRequest) -> Result<LeaseGuard, LeaseError> {
        let guards = self.acquire_many(vec![request]).await?;
        guards.into_iter().next().ok_or(LeaseError::InvalidGuard)
    }

    /// Acquires a sorted set of leases atomically; overlapping requests are rejected.
    ///
    /// # Errors
    /// Returns a typed validation, capacity, timeout, or admission error.
    pub async fn acquire_many(
        &self,
        mut requests: Vec<LeaseRequest>,
    ) -> Result<Vec<LeaseGuard>, LeaseError> {
        validate_requests(&mut requests)?;
        if requests.len() > self.inner.config.max_active_leases {
            return Err(LeaseError::LimitExceeded);
        }
        let mut changed = self.inner.changed.subscribe();
        let ticket = self.enqueue(requests)?;
        let _cleanup = QueueCleanup {
            inner: Arc::clone(&self.inner),
            ticket,
        };
        let wait = async {
            loop {
                let notified = changed.changed();
                if let Some(ids) = take_grant(&self.inner, ticket)? {
                    return Ok(ids);
                }
                let expires = {
                    let state = lock_state(&self.inner.state);
                    state.active.iter().map(|record| record.expires).min()
                };
                match expires {
                    Some(expires) => {
                        tokio::select! {
                            result = notified => { result.map_err(|_| LeaseError::InvalidGuard)?; }
                            () = tokio::time::sleep_until(tokio::time::Instant::from_std(expires)) => {
                                self.reap_expired();
                            }
                        }
                    }
                    None => {
                        notified.await.map_err(|_| LeaseError::InvalidGuard)?;
                    }
                }
            }
        };
        let ids = tokio::time::timeout(self.inner.config.acquire_timeout, wait)
            .await
            .map_err(|_| LeaseError::Timeout)??;
        signal(&self.inner);
        Ok(ids
            .into_iter()
            .map(|id| LeaseGuard {
                inner: Arc::clone(&self.inner),
                id,
            })
            .collect())
    }

    /// Attempts immediate admission without bypassing an earlier conflicting waiter.
    ///
    /// # Errors
    /// Returns [`LeaseError::Busy`] if a conflicting queued or active request has priority.
    pub fn try_acquire(&self, request: LeaseRequest) -> Result<LeaseGuard, LeaseError> {
        let mut requests = vec![request];
        validate_requests(&mut requests)?;
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now());
        if queue_blocks(&state, &requests) {
            return Err(busy_holder(&state, &requests));
        }
        let ids = grant_locked(&self.inner, &mut state, requests)?;
        publish_holders(&self.inner, &state);
        let id = ids.into_iter().next().ok_or(LeaseError::InvalidGuard)?;
        drop(state);
        signal(&self.inner);
        Ok(LeaseGuard {
            inner: Arc::clone(&self.inner),
            id,
        })
    }

    /// Returns the number of retained lease records for diagnostics. This cold-path query
    /// takes the state lock. Expired records are included until the next cleanup operation;
    /// use `holder_of` when checking current write authority.
    #[must_use]
    pub fn active_count(&self) -> usize {
        lock_state(&self.inner.state).active.len()
    }

    /// Lock-free snapshot lookup for the current conflicting holder.
    #[must_use]
    pub fn holder_of(&self, key: &LeaseKey, scope: &LeaseScope) -> Option<(FrameId, LeaseId)> {
        let snapshot = self.inner.holders.load();
        let range = snapshot.ranges.get(key)?;
        let holders = snapshot.holders.get(range.clone())?;
        let now = Instant::now();
        holders
            .iter()
            .find(|holder| holder.expires > now && scopes_overlap(scope, &holder.scope))
            .map(|holder| (holder.agent, holder.id))
    }

    /// Validates the guard and optimistic base, producing an owner-bound write ticket.
    ///
    /// On mismatch a notice includes the actual current Cid so the caller can refresh its view.
    ///
    /// # Errors
    /// Returns [`LeaseError::StaleBase`] when the caller's content has changed.
    pub fn check_write(&self, guard: &LeaseGuard, current: Cid) -> Result<WriteTicket, LeaseError> {
        if !Arc::ptr_eq(&self.inner, &guard.inner) {
            return Err(LeaseError::InvalidGuard);
        }
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now());
        let record = state
            .active
            .iter_mut()
            .find(|record| record.holder.id == guard.id)
            .ok_or(LeaseError::InvalidGuard)?;
        if !record.write_allowed {
            return Err(LeaseError::Revoked(RevokeReason::ChurnLimit));
        }
        let expected = record.base.ok_or(LeaseError::MissingBase)?;
        if expected != current {
            let event = Invalidation {
                agent: record.holder.agent,
                key: record.holder.key.clone(),
                lease: record.holder.id,
                current: Some(current),
                reason: None,
            };
            let _ = self.inner.invalidations.send(event);
            return Err(LeaseError::StaleBase {
                expected,
                actual: current,
            });
        }
        Ok(WriteTicket {
            manager: Arc::downgrade(&self.inner.ticket_token),
            lease: record.holder.id,
            agent: record.holder.agent,
            base: expected,
        })
    }

    /// Revalidates a ticket immediately before applying its write.
    ///
    /// The applier must call this after resolving the latest content so expiry or revocation
    /// cannot turn an old ticket into authority for a later write.
    ///
    /// # Errors
    /// Returns [`LeaseError::InvalidGuard`], [`LeaseError::Revoked`], or [`LeaseError::StaleBase`].
    pub fn validate_write_ticket(
        &self,
        ticket: WriteTicket,
        current: Cid,
    ) -> Result<(), LeaseError> {
        let manager = ticket.manager.upgrade().ok_or(LeaseError::InvalidGuard)?;
        if !Arc::ptr_eq(&manager, &self.inner.ticket_token) {
            return Err(LeaseError::InvalidGuard);
        }
        if !Arc::ptr_eq(&manager, &self.inner.ticket_token) {
            return Err(LeaseError::InvalidGuard);
        }
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now());
        let record = state
            .active
            .iter()
            .find(|record| record.holder.id == ticket.lease && record.holder.agent == ticket.agent)
            .ok_or(LeaseError::InvalidGuard)?;
        if !record.write_allowed {
            return Err(LeaseError::Revoked(RevokeReason::ChurnLimit));
        }
        let expected = record.base.ok_or(LeaseError::MissingBase)?;
        if expected != ticket.base {
            return Err(LeaseError::InvalidGuard);
        }
        if current != ticket.base {
            let event = Invalidation {
                agent: record.holder.agent,
                key: record.holder.key.clone(),
                lease: record.holder.id,
                current: Some(current),
                reason: None,
            };
            let _ = self.inner.invalidations.send(event);
            return Err(LeaseError::StaleBase {
                expected: ticket.base,
                actual: current,
            });
        }
        Ok(())
    }

    /// Records a file edit and revokes write rights after the configured number of edits.
    pub fn note_edit(
        &self,
        agent: FrameId,
        key: &LeaseKey,
        subtask: &str,
    ) -> Result<ChurnVerdict, LeaseError> {
        if subtask.is_empty() || subtask.len() > MAX_SUBTASK_LEN {
            return Err(LeaseError::LimitExceeded);
        }
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now());
        if !state
            .active
            .iter()
            .any(|record| record.holder.agent == agent && &record.holder.key == key)
        {
            return Err(LeaseError::InvalidGuard);
        }
        let churn_key = (agent, key.clone(), subtask.to_owned());
        if !state.churn.contains_key(&churn_key) && state.churn.len() >= MAX_CHURN_RECORDS {
            return Err(LeaseError::LimitExceeded);
        }
        let count = state.churn.entry(churn_key).or_default();
        *count = count.saturating_add(1);
        let edits = *count;
        let threshold = self.inner.config.edits_before_revoke;
        if edits > threshold {
            let mut events = Vec::new();
            for record in &mut state.active {
                if record.holder.agent == agent && &record.holder.key == key {
                    record.write_allowed = false;
                    events.push(Invalidation {
                        agent,
                        key: key.clone(),
                        lease: record.holder.id,
                        current: None,
                        reason: Some(RevokeReason::ChurnLimit),
                    });
                }
            }
            publish_holders(&self.inner, &state);
            drop(state);
            for event in events {
                let _ = self.inner.invalidations.send(event);
            }
            Ok(ChurnVerdict::Revoke {
                edits,
                escalate_to_s2: true,
            })
        } else if edits == threshold {
            Ok(ChurnVerdict::Warn { edits })
        } else {
            Ok(ChurnVerdict::Ok)
        }
    }

    /// Revokes every active lease held by a frame and wakes FIFO waiters.
    pub async fn revoke_all(&self, agent: FrameId, why: RevokeReason) {
        let mut state = lock_state(&self.inner.state);
        let mut events = Vec::new();
        state.active.retain(|record| {
            if record.holder.agent == agent {
                events.push(Invalidation {
                    agent,
                    key: record.holder.key.clone(),
                    lease: record.holder.id,
                    current: None,
                    reason: Some(why.clone()),
                });
                false
            } else {
                true
            }
        });
        publish_holders(&self.inner, &state);
        service_queue(&self.inner, &mut state);
        drop(state);
        for event in events {
            let _ = self.inner.invalidations.send(event);
        }
        signal(&self.inner);
    }

    /// Reaps all expired leases and returns the number removed.
    pub fn reap_expired(&self) -> usize {
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now())
    }

    fn enqueue(&self, requests: Vec<LeaseRequest>) -> Result<u64, LeaseError> {
        let mut state = lock_state(&self.inner.state);
        reap_locked(&self.inner, &mut state, Instant::now());
        if state.order.len() >= self.inner.config.max_waiters {
            return Err(LeaseError::LimitExceeded);
        }
        let id = state.next_ticket;
        state.next_ticket = id.checked_add(1).ok_or(LeaseError::IdExhausted)?;
        state.order.push_back(id);
        state.waiters.insert(
            id,
            Waiter {
                requests,
                granted: None,
            },
        );
        service_queue(&self.inner, &mut state);
        publish_holders(&self.inner, &state);
        drop(state);
        signal(&self.inner);
        Ok(id)
    }
}

/// RAII lease guard. Dropping it releases the lease synchronously.
pub struct LeaseGuard {
    inner: Arc<Inner>,
    id: LeaseId,
}

impl LeaseGuard {
    /// Returns this guard's lease id.
    #[must_use]
    pub const fn id(&self) -> LeaseId {
        self.id
    }

    /// Returns the owner bound to this lease, if it remains active.
    #[must_use]
    pub fn owner(&self) -> Option<FrameId> {
        let snapshot = self.inner.holders.load();
        snapshot
            .holders
            .iter()
            .find(|holder| holder.id == self.id && holder.expires > Instant::now())
            .map(|holder| holder.agent)
    }

    /// Releases now rather than waiting for drop.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        let mut state = lock_state(&self.inner.state);
        state.active.retain(|record| record.holder.id != self.id);
        publish_holders(&self.inner, &state);
        service_queue(&self.inner, &mut state);
        drop(state);
        signal(&self.inner);
    }
}

struct QueueCleanup {
    inner: Arc<Inner>,
    ticket: u64,
}
impl Drop for QueueCleanup {
    fn drop(&mut self) {
        let mut state = lock_state(&self.inner.state);
        state.order.retain(|id| *id != self.ticket);
        if let Some(waiter) = state.waiters.remove(&self.ticket)
            && let Some(ids) = waiter.granted
        {
            state
                .active
                .retain(|record| !ids.contains(&record.holder.id));
        }
        service_queue(&self.inner, &mut state);
        publish_holders(&self.inner, &state);
        drop(state);
        signal(&self.inner);
    }
}

fn validate_requests(requests: &mut [LeaseRequest]) -> Result<(), LeaseError> {
    if requests.is_empty() || requests.len() > MAX_REQUESTS_PER_ACQUIRE {
        return Err(LeaseError::LimitExceeded);
    }
    requests.sort_by(|a, b| a.key.cmp(&b.key));
    for req in requests.iter() {
        if let LeaseScope::AstNode(path) = &req.scope
            && path.len() > MAX_COMPONENTS_PER_REQUEST
        {
            return Err(LeaseError::LimitExceeded);
        }
    }
    for (index, left) in requests.iter().enumerate() {
        for right in requests.iter().skip(index.saturating_add(1)) {
            if scopes_conflict(&left.key, &left.scope, &right.key, &right.scope) {
                return Err(LeaseError::OverlappingRequests);
            }
        }
    }
    Ok(())
}
fn take_grant(inner: &Arc<Inner>, ticket: u64) -> Result<Option<Vec<LeaseId>>, LeaseError> {
    let mut state = lock_state(&inner.state);
    let granted = state
        .waiters
        .get(&ticket)
        .and_then(|waiter| waiter.granted.clone());
    if granted.is_some() {
        state.waiters.remove(&ticket);
    }
    Ok(granted)
}

fn service_queue(inner: &Arc<Inner>, state: &mut State) {
    let mut index = 0;
    while index < state.order.len() {
        let Some(ticket) = state.order.get(index).copied() else {
            break;
        };
        let Some(waiter) = state.waiters.get(&ticket) else {
            let Some(_removed_ticket) = state.order.remove(index) else {
                break;
            };
            continue;
        };
        if waiter.granted.is_some() {
            let Some(_removed_ticket) = state.order.remove(index) else {
                break;
            };
            continue;
        }
        let requests = waiter.requests.clone();
        let blocked_by_earlier_conflict = state
            .order
            .iter()
            .take(index)
            .filter_map(|earlier| state.waiters.get(earlier))
            .any(|earlier| {
                earlier.granted.is_none() && requests_conflict(&requests, &earlier.requests)
            });
        if blocked_by_earlier_conflict || !can_grant(state, &requests) {
            index = index.saturating_add(1);
            continue;
        }
        // Capacity exhaustion can block a multi-key waiter without blocking unrelated keys.
        // Fairness is per conflicting request; keep scanning so disjoint work can use any
        // remaining slots. The normal bounded queue and acquire timeout cap waiter lifetime.
        if state.active.len().saturating_add(requests.len()) > inner.config.max_active_leases {
            index = index.saturating_add(1);
            continue;
        }
        match grant_locked(inner, state, requests) {
            Ok(ids) => {
                if let Some(waiter) = state.waiters.get_mut(&ticket) {
                    waiter.granted = Some(ids);
                }
                let Some(_removed_ticket) = state.order.remove(index) else {
                    break;
                };
            }
            Err(_) => break,
        }
    }
}

fn queue_blocks(state: &State, requests: &[LeaseRequest]) -> bool {
    state.order.iter().any(|ticket| {
        state.waiters.get(ticket).is_some_and(|waiter| {
            waiter.granted.is_none() && requests_conflict(requests, &waiter.requests)
        })
    })
}

fn requests_conflict(left: &[LeaseRequest], right: &[LeaseRequest]) -> bool {
    left.iter().any(|left| {
        right
            .iter()
            .any(|right| scopes_conflict(&left.key, &left.scope, &right.key, &right.scope))
    })
}

fn can_grant(state: &State, requests: &[LeaseRequest]) -> bool {
    requests.iter().all(|request| {
        state.active.iter().all(|active| {
            !scopes_conflict(
                &request.key,
                &request.scope,
                &active.holder.key,
                &active.holder.scope,
            )
        })
    })
}

fn grant_locked(
    inner: &Arc<Inner>,
    state: &mut State,
    requests: Vec<LeaseRequest>,
) -> Result<Vec<LeaseId>, LeaseError> {
    if !can_grant(state, &requests) {
        return Err(busy_holder(state, &requests));
    }
    let next_len = u64::try_from(requests.len()).map_err(|_| LeaseError::LimitExceeded)?;
    let next_id = state
        .next_lease
        .checked_add(next_len)
        .ok_or(LeaseError::IdExhausted)?;
    if state.active.len().saturating_add(requests.len()) > inner.config.max_active_leases {
        return Err(LeaseError::LimitExceeded);
    }
    let expires = Instant::now()
        .checked_add(inner.config.lease_ttl)
        .ok_or(LeaseError::LimitExceeded)?;
    let mut ids = Vec::with_capacity(requests.len());
    let mut raw = state.next_lease;
    for request in requests {
        let id = LeaseId(raw);
        raw = raw.checked_add(1).ok_or(LeaseError::IdExhausted)?;
        ids.push(id);
        state.active.push(LeaseRecord {
            holder: Holder {
                id,
                agent: request.agent,
                key: request.key,
                scope: request.scope,
                expires,
            },
            base: request.base,
            expires,
            write_allowed: true,
        });
    }
    state.next_lease = next_id;
    Ok(ids)
}

fn busy_holder(state: &State, requests: &[LeaseRequest]) -> LeaseError {
    let holder = state
        .active
        .iter()
        .find(|active| {
            requests.iter().any(|request| {
                scopes_conflict(
                    &request.key,
                    &request.scope,
                    &active.holder.key,
                    &active.holder.scope,
                )
            })
        })
        .map(|active| active.holder.agent);
    LeaseError::Busy { holder }
}

fn reap_locked(inner: &Arc<Inner>, state: &mut State, now: Instant) -> usize {
    let before = state.active.len();
    let mut events = Vec::new();
    state.active.retain(|record| {
        if record.expires <= now {
            events.push(Invalidation {
                agent: record.holder.agent,
                key: record.holder.key.clone(),
                lease: record.holder.id,
                current: None,
                reason: Some(RevokeReason::Expired),
            });
            false
        } else {
            true
        }
    });
    let removed = before.saturating_sub(state.active.len());
    service_queue(inner, state);
    publish_holders(inner, state);
    for event in events {
        let _ = inner.invalidations.send(event);
    }
    signal(inner);
    removed
}

fn publish_holders(inner: &Inner, state: &State) {
    let mut snapshot: Vec<_> = state
        .active
        .iter()
        .map(|record| record.holder.clone())
        .collect();
    snapshot.sort_by(|left, right| left.key.cmp(&right.key));
    let mut ranges: HashMap<LeaseKey, std::ops::Range<usize>> = HashMap::new();
    for (index, holder) in snapshot.iter().enumerate() {
        let end = index.saturating_add(1);
        ranges
            .entry(holder.key.clone())
            .and_modify(|range| range.end = end)
            .or_insert(index..end);
    }
    inner.holders.store(Arc::new(HolderSnapshot {
        holders: snapshot,
        ranges,
    }));
}

fn lock_state(mutex: &StateMutex<State>) -> StateMutexGuard<'_, State> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
fn signal(inner: &Inner) {
    inner
        .changed
        .send_modify(|version| *version = version.saturating_add(1));
}

#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use crate::SymbolPath;
    use loom::thread;

    fn manager() -> LeaseManager {
        LeaseManager::new(LeaseConfig {
            max_active_leases: 8,
            max_waiters: 8,
            acquire_timeout: Duration::from_secs(5),
            lease_ttl: Duration::from_secs(60),
            edits_before_revoke: 3,
        })
    }

    fn request(agent: u128, path: &str, scope: LeaseScope) -> LeaseRequest {
        LeaseRequest {
            agent: FrameId::from_u128(agent),
            key: LeaseKey::from_resolved_relative(path).expect("valid model path"),
            scope,
            base: None,
        }
    }

    fn symbol_scope() -> LeaseScope {
        LeaseScope::AstNode(SymbolPath::new(["module", "function"]).expect("valid symbol"))
    }

    fn enqueue(manager: &LeaseManager, request: LeaseRequest) -> u64 {
        let mut requests = vec![request];
        validate_requests(&mut requests).expect("valid non-overlapping acquire request");
        manager
            .enqueue(requests)
            .expect("bounded queue accepts request")
    }

    fn enqueue_many(manager: &LeaseManager, requests: Vec<LeaseRequest>) -> u64 {
        let mut requests = requests;
        validate_requests(&mut requests).expect("valid non-overlapping acquire request set");
        manager
            .enqueue(requests)
            .expect("bounded queue accepts request set")
    }

    fn take_guard(manager: &LeaseManager, ticket: u64) -> Option<LeaseGuard> {
        let ids = take_grant(&manager.inner, ticket).expect("grant lookup succeeds")?;
        assert_eq!(ids.len(), 1, "each modeled waiter owns one lease");
        Some(LeaseGuard {
            inner: Arc::clone(&manager.inner),
            id: ids[0],
        })
    }

    #[test]
    fn overlapping_waiters_keep_ticket_order_and_are_granted_once() {
        loom::model(|| {
            let manager = manager();
            let blocker = manager
                .try_acquire(request(1, "src/lib.rs", LeaseScope::EntireFile))
                .expect("initial whole-file lease");

            let first_manager = manager.clone();
            let first = thread::spawn(move || {
                enqueue(
                    &first_manager,
                    request(2, "src/lib.rs", LeaseScope::EntireFile),
                )
            });
            let second_manager = manager.clone();
            let second = thread::spawn(move || {
                enqueue(&second_manager, request(3, "src/lib.rs", symbol_scope()))
            });
            let first_ticket = first.join().expect("first waiter enqueued");
            let second_ticket = second.join().expect("second waiter enqueued");
            let (head, tail) = if first_ticket < second_ticket {
                (first_ticket, second_ticket)
            } else {
                (second_ticket, first_ticket)
            };

            drop(blocker);
            let head_guard = take_guard(&manager, head).expect("FIFO head is granted");
            assert_eq!(manager.active_count(), 1, "head produces one active grant");
            assert!(
                take_guard(&manager, head).is_none(),
                "a consumed ticket cannot yield a second grant"
            );
            assert!(
                take_guard(&manager, tail).is_none(),
                "tail must wait for head"
            );
            drop(head_guard);
            let tail_guard = take_guard(&manager, tail).expect("tail follows released head");
            drop(tail_guard);
            assert_eq!(manager.active_count(), 0, "all leases are released once");
        });
    }

    #[test]
    fn disjoint_waiter_progresses_while_conflicting_waiters_preserve_fifo() {
        loom::model(|| {
            let manager = manager();
            let blocker = manager
                .try_acquire(request(1, "src/lib.rs", LeaseScope::EntireFile))
                .expect("initial whole-file lease");

            let first_manager = manager.clone();
            let first = thread::spawn(move || {
                enqueue(
                    &first_manager,
                    request(2, "src/lib.rs", LeaseScope::EntireFile),
                )
            });
            let second_manager = manager.clone();
            let second = thread::spawn(move || {
                enqueue(&second_manager, request(3, "src/lib.rs", symbol_scope()))
            });
            let first_ticket = first.join().expect("first waiter enqueued");
            let second_ticket = second.join().expect("second waiter enqueued");
            let (head, tail) = if first_ticket < second_ticket {
                (first_ticket, second_ticket)
            } else {
                (second_ticket, first_ticket)
            };

            let disjoint_ticket =
                enqueue(&manager, request(4, "src/other.rs", LeaseScope::EntireFile));
            let disjoint_guard = take_guard(&manager, disjoint_ticket)
                .expect("unrelated key progresses past blocked queue head");
            assert!(take_guard(&manager, head).is_none());
            assert!(take_guard(&manager, tail).is_none());

            let bypass = manager
                .try_acquire(request(5, "src/third.rs", LeaseScope::EntireFile))
                .expect("immediate admission may bypass only non-conflicting waiters");
            assert!(matches!(
                manager.try_acquire(request(6, "src/lib.rs", symbol_scope())),
                Err(LeaseError::Busy { .. })
            ));

            drop(blocker);
            let head_guard = take_guard(&manager, head).expect("FIFO head follows blocker");
            assert!(take_guard(&manager, tail).is_none());
            drop(head_guard);
            let tail_guard = take_guard(&manager, tail).expect("conflicting tail follows head");
            drop(tail_guard);
            drop(disjoint_guard);
            drop(bypass);
            assert_eq!(
                manager.active_count(),
                0,
                "all independent grants are released"
            );
        });
    }

    #[test]
    fn disjoint_single_key_uses_remaining_capacity_ahead_of_blocked_multi_key_waiter() {
        loom::model(|| {
            let manager = LeaseManager::new(LeaseConfig {
                max_active_leases: 2,
                max_waiters: 8,
                acquire_timeout: Duration::from_secs(5),
                lease_ttl: Duration::from_secs(60),
                edits_before_revoke: 3,
            });
            let blocker = manager
                .try_acquire(request(1, "src/lib.rs", LeaseScope::EntireFile))
                .expect("initial whole-file lease");
            let multi_ticket = enqueue_many(
                &manager,
                vec![
                    request(2, "src/one.rs", LeaseScope::EntireFile),
                    request(2, "src/two.rs", LeaseScope::EntireFile),
                ],
            );
            assert!(take_guard(&manager, multi_ticket).is_none());

            let disjoint = manager
                .try_acquire(request(3, "src/three.rs", LeaseScope::EntireFile))
                .expect("immediate disjoint work uses remaining capacity");
            assert!(matches!(
                manager.try_acquire(request(4, "src/one.rs", LeaseScope::EntireFile)),
                Err(LeaseError::Busy { holder: None })
            ));
            assert!(take_guard(&manager, multi_ticket).is_none());

            drop(blocker);
            assert!(take_guard(&manager, multi_ticket).is_none());
            drop(disjoint);
            let multi = take_grant(&manager.inner, multi_ticket)
                .expect("grant lookup succeeds")
                .expect("multi-key waiter is granted when enough capacity is free");
            assert_eq!(multi.len(), 2, "multi-key admission remains atomic");
            assert_eq!(manager.active_count(), 2);
            for id in multi {
                drop(LeaseGuard {
                    inner: Arc::clone(&manager.inner),
                    id,
                });
            }
            assert_eq!(manager.active_count(), 0, "all leases are released");
        });
    }
}
