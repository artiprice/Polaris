//! Named semaphore pools and their bounded runtime counters.

use crate::DagError;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Work class protected from oversubscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pool {
    /// Model and inference calls.
    Inference,
    /// Web search and browser calls.
    Web,
    /// Documentation retrieval and generation.
    Docs,
    /// Disk and local indexing work.
    Disk,
    /// System-one verifier calls.
    S1,
    /// Sandbox execution.
    Sandbox,
}

/// Maximum concurrent permits for each named pool.
///
/// `Default` uses the plan's initial example values; operators should tune limits from measured
/// provider and host capacity before treating them as production capacity targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolLimits {
    /// Maximum inference operations.
    pub inference: u32,
    /// Maximum web operations.
    pub web: u32,
    /// Maximum documentation operations.
    pub docs: u32,
    /// Maximum disk operations.
    pub disk: u32,
    /// Maximum verifier operations.
    pub s1: u32,
    /// Maximum sandbox operations.
    pub sandbox: u32,
}

impl Default for PoolLimits {
    fn default() -> Self {
        Self {
            inference: 6,
            web: 10,
            docs: 16,
            disk: 64,
            s1: 3,
            sandbox: 4,
        }
    }
}

impl PoolLimits {
    fn get(self, pool: Pool) -> u32 {
        match pool {
            Pool::Inference => self.inference,
            Pool::Web => self.web,
            Pool::Docs => self.docs,
            Pool::Disk => self.disk,
            Pool::S1 => self.s1,
            Pool::Sandbox => self.sandbox,
        }
    }
}

/// Current counters for one pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolCounters {
    /// Configured permit limit.
    pub limit: u32,
    /// Permits currently held by running operators.
    pub active: u32,
    /// Acquisition futures waiting for permits.
    pub queued: u64,
    /// Tasks for which cancellation/loser abort was initiated.
    pub aborted_early: u64,
}

/// Snapshot of named pool counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// Inference pool counters.
    pub inference: PoolCounters,
    /// Web pool counters.
    pub web: PoolCounters,
    /// Documentation pool counters.
    pub docs: PoolCounters,
    /// Disk pool counters.
    pub disk: PoolCounters,
    /// Verifier pool counters.
    pub s1: PoolCounters,
    /// Sandbox pool counters.
    pub sandbox: PoolCounters,
}

struct PoolInner {
    pool: Pool,
    semaphore: Arc<Semaphore>,
    limit: AtomicU32,
    active: AtomicU32,
    queued: AtomicU64,
    aborted_early: AtomicU64,
    gate: Mutex<()>,
}

impl PoolInner {
    fn lock(&self) -> MutexGuard<'_, ()> {
        match self.gate.lock() {
            Ok(guard) => guard,
            Err(poisoned) => PoisonError::into_inner(poisoned),
        }
    }

    fn queue(self: &Arc<Self>) -> Result<QueueGuard, DagError> {
        let _gate = self.lock();
        self.queued
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_add(1)
            })
            .map_err(|_| DagError::CounterOverflow)?;
        Ok(QueueGuard {
            inner: Arc::clone(self),
            armed: true,
        })
    }

    fn remove_queue_locked(&self) {
        let current = self.queued.load(Ordering::SeqCst);
        if current > 0 {
            self.queued.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn counters(&self) -> PoolCounters {
        PoolCounters {
            limit: self.limit.load(Ordering::SeqCst),
            active: self.active.load(Ordering::SeqCst),
            queued: self.queued.load(Ordering::SeqCst),
            aborted_early: self.aborted_early.load(Ordering::SeqCst),
        }
    }

    fn note_aborted(&self) {
        let _ = self
            .aborted_early
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_add(1))
            });
    }
}

struct QueueGuard {
    inner: Arc<PoolInner>,
    armed: bool,
}

impl QueueGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for QueueGuard {
    fn drop(&mut self) {
        if self.armed {
            let _gate = self.inner.lock();
            self.inner.remove_queue_locked();
        }
    }
}

/// Named concurrency limits backed by Tokio semaphores.
#[derive(Clone)]
pub struct Pools {
    inference: Arc<PoolInner>,
    web: Arc<PoolInner>,
    docs: Arc<PoolInner>,
    disk: Arc<PoolInner>,
    s1: Arc<PoolInner>,
    sandbox: Arc<PoolInner>,
}

impl Pools {
    /// Creates all six pools with positive limits.
    pub fn new(limits: PoolLimits) -> Result<Self, DagError> {
        for pool in [
            Pool::Inference,
            Pool::Web,
            Pool::Docs,
            Pool::Disk,
            Pool::S1,
            Pool::Sandbox,
        ] {
            let limit = limits.get(pool);
            checked_limit(limit)?;
        }
        Ok(Self {
            inference: make_pool(Pool::Inference, limits.inference)?,
            web: make_pool(Pool::Web, limits.web)?,
            docs: make_pool(Pool::Docs, limits.docs)?,
            disk: make_pool(Pool::Disk, limits.disk)?,
            s1: make_pool(Pool::S1, limits.s1)?,
            sandbox: make_pool(Pool::Sandbox, limits.sandbox)?,
        })
    }

    /// Waits until a permit from `pool` is available.
    ///
    /// Dropping this future while queued removes its queue counter. A closed semaphore is returned
    /// as [`DagError::Closed`].
    pub async fn acquire(&self, pool: Pool) -> Result<PoolPermit, DagError> {
        let inner = Arc::clone(self.get(pool));
        let mut queued = inner.queue()?;
        let permit = Arc::clone(&inner)
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| DagError::Closed)?;
        {
            let _gate = inner.lock();
            inner.remove_queue_locked();
            inner.active.fetch_add(1, Ordering::SeqCst);
            queued.disarm();
        }
        Ok(PoolPermit {
            lease: Arc::new(PermitLease {
                inner,
                _permit: permit,
            }),
        })
    }

    /// Changes a pool's permit limit when no work is active or queued.
    ///
    /// Refusing live resize avoids transiently exceeding the newly configured bound while an
    /// operator is holding an old permit count.
    pub fn resize(&self, pool: Pool, permits: u32) -> Result<(), DagError> {
        let inner = self.get(pool);
        let target = checked_limit(permits)?;
        let _gate = inner.lock();
        if inner.active.load(Ordering::SeqCst) != 0 || inner.queued.load(Ordering::SeqCst) != 0 {
            return Err(DagError::PoolBusy);
        }
        let current = inner.limit.load(Ordering::SeqCst);
        let current_usize = usize::try_from(current).map_err(|_| DagError::InvalidLimits)?;
        if inner.semaphore.available_permits() != current_usize {
            return Err(DagError::PoolBusy);
        }
        if target > current_usize {
            inner.semaphore.add_permits(target - current_usize);
        } else if target < current_usize {
            let decrease = current_usize - target;
            let removed = inner.semaphore.forget_permits(decrease);
            if removed != decrease {
                return Err(DagError::PoolBusy);
            }
        }
        inner.limit.store(permits, Ordering::SeqCst);
        Ok(())
    }

    /// Closes all semaphores so queued and future acquisitions return [`DagError::Closed`].
    pub fn close(&self) {
        for pool in self.all() {
            pool.semaphore.close();
        }
    }

    /// Returns a point-in-time snapshot of active, queued, and canceled work.
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            inference: self.inference.counters(),
            web: self.web.counters(),
            docs: self.docs.counters(),
            disk: self.disk.counters(),
            s1: self.s1.counters(),
            sandbox: self.sandbox.counters(),
        }
    }

    pub(crate) fn note_aborted(&self, pool: Pool) {
        self.get(pool).note_aborted();
    }

    fn get(&self, pool: Pool) -> &Arc<PoolInner> {
        match pool {
            Pool::Inference => &self.inference,
            Pool::Web => &self.web,
            Pool::Docs => &self.docs,
            Pool::Disk => &self.disk,
            Pool::S1 => &self.s1,
            Pool::Sandbox => &self.sandbox,
        }
    }

    fn all(&self) -> [&Arc<PoolInner>; 6] {
        [
            &self.inference,
            &self.web,
            &self.docs,
            &self.disk,
            &self.s1,
            &self.sandbox,
        ]
    }
}

fn make_pool(pool: Pool, limit: u32) -> Result<Arc<PoolInner>, DagError> {
    let permits = checked_limit(limit)?;
    Ok(Arc::new(PoolInner {
        pool,
        semaphore: Arc::new(Semaphore::new(permits)),
        limit: AtomicU32::new(limit),
        active: AtomicU32::new(0),
        queued: AtomicU64::new(0),
        aborted_early: AtomicU64::new(0),
        gate: Mutex::new(()),
    }))
}

fn checked_limit(limit: u32) -> Result<usize, DagError> {
    let permits = usize::try_from(limit).map_err(|_| DagError::InvalidLimits)?;
    if permits == 0 || permits > Semaphore::MAX_PERMITS {
        return Err(DagError::InvalidLimits);
    }
    Ok(permits)
}

/// RAII permit held for the complete lifetime of one operator attempt.
#[derive(Clone)]
#[must_use = "dropping the last permit handle releases the pool slot"]
pub struct PoolPermit {
    lease: Arc<PermitLease>,
}

struct PermitLease {
    inner: Arc<PoolInner>,
    _permit: OwnedSemaphorePermit,
}

impl PoolPermit {
    /// Pool from which this permit was acquired.
    pub fn pool(&self) -> Pool {
        self.lease.inner.pool
    }
}

impl Drop for PermitLease {
    fn drop(&mut self) {
        {
            let _gate = self.inner.lock();
            let active = self.inner.active.load(Ordering::SeqCst);
            if active > 0 {
                self.inner.active.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}
