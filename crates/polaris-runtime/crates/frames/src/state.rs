//! Frame state transitions, local quota ledgers, and read-only watchdog snapshots.
use crate::quota::AttachedQuotaLease;
use crate::scope::child_expiry;
use crate::{FrameScope, QuotaError, ResourceQuota, ScopeError, scope_is_subset};
use polaris_sdk::{
    effort::Effort,
    error::{ErrorKind, HasKind},
    ids::{AgentRole, Cid, FastFingerprint, FrameId, ModelId},
};
use polaris_security_capability::{CapabilityError, Clock, Scope, Token, attenuate};
use std::{
    marker::PhantomData,
    sync::{Arc, Mutex, MutexGuard},
};

/// Frame state markers. Transitions consume the prior state.
pub struct Spawned;
/// Runnable state; only this state exposes step and delegation operations.
pub struct Active;
/// Suspended state; it cannot run steps or create descendants.
pub struct Suspended;
/// Terminal state; it cannot run steps or create descendants.
pub struct Finished;

/// Opaque pointer to a CAS-backed epistemic scratchpad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScratchpadRef(Cid);
impl ScratchpadRef {
    /// Wrap an existing CAS id; this does not claim the referenced content exists.
    #[must_use]
    pub const fn new(cid: Cid) -> Self {
        Self(cid)
    }
    /// Content-addressed scratchpad identifier.
    #[must_use]
    pub const fn cid(self) -> Cid {
        self.0
    }
}

/// Selected model and maximum effort authorized by the caller's current model policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelection {
    /// Verified model identifier chosen by the composition root.
    pub model: ModelId,
    /// Highest effort the selection permits.
    pub maximum_effort: Effort,
}

/// Sequential frame step number, starting at one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepNo(u32);
impl StepNo {
    /// Numeric step index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One independently verified diff in the frame history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffTimelineEntry {
    /// Step that produced the independently verified diff.
    pub step: StepNo,
    /// CAS evidence identifier supplied by the verifier.
    pub diff: Cid,
}

/// A public, secret-free frame view suitable for monitoring and watchdog policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameView {
    /// Frame identity.
    pub id: FrameId,
    /// Parent identity, if delegated.
    pub parent: Option<FrameId>,
    /// Role.
    pub role: AgentRole,
    /// Last recorded cheap source fingerprint.
    pub fingerprint: Option<FastFingerprint>,
    /// Reserved model/tool attempt count, including rejected and cancelled attempts.
    pub steps: u32,
    /// CAS scratchpad reference.
    pub scratchpad: ScratchpadRef,
}

/// Terminal status. `ClaimedDone` intentionally is not a terminal outcome; proof-of-done belongs
/// to an independent verifier and must be checked before a caller chooses `Completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Completed,
    Failed,
    Cancelled,
    Closed,
}

/// Frame lifecycle and policy errors.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// Quota reservation failed.
    #[error(transparent)]
    Quota(#[from] QuotaError),
    /// Child constraints were not proven to be subsets.
    #[error(transparent)]
    Scope(#[from] ScopeError),
    /// Capability attenuation failed.
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    /// The model provider rejected or failed the request.
    #[error(transparent)]
    Model(#[from] polaris_sdk::registry::ModelError),
    /// Frame nesting exceeded its configured maximum.
    #[error("frame depth limit exceeded")]
    DepthLimit,
    /// Frame has no remaining steps.
    #[error("frame step limit exceeded")]
    StepLimit,
    /// Model effort exceeds the caller's selection.
    #[error("requested model effort exceeds selected policy")]
    EffortDenied,
    /// Model token allowance is exhausted.
    #[error("frame token budget exhausted")]
    TokenBudget,
    /// Internal checked counter or timestamp arithmetic failed.
    #[error("frame accounting invariant failed")]
    Accounting,
    /// Provider usage exceeded the full prompt/output reservation; tools are not dispatched.
    #[error("provider usage exceeded reservation (observed {observed}, reserved {reserved})")]
    UsageExceeded { observed: u64, reserved: u64 },
    /// Provider emitted repeated or inconsistent usage data.
    #[error("invalid or repeated provider usage report")]
    InvalidUsage,
    /// Cancellation was observed at an await boundary.
    #[error("frame step cancelled")]
    Cancelled,
    /// Model output exceeded a configured byte or call limit.
    #[error("model or tool output exceeded its configured limit")]
    OutputLimit,
    /// Stream ended without a terminal finish chunk.
    #[error("model stream ended before a finish chunk")]
    IncompleteStream,
    /// Tool call fragments were incomplete or malformed.
    #[error("invalid model tool call")]
    InvalidToolCall,
    /// Role policy denied a model-requested tool before capability authorization.
    #[error("tool is outside the frame role allowlist")]
    RoleToolDenied,
    /// A dependency failed at a named boundary.
    #[error("{1} failed ({0:?})")]
    Backend(ErrorKind, &'static str),
}
impl HasKind for FrameError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Quota(QuotaError::Accounting) | Self::Accounting => ErrorKind::Fatal,
            Self::Quota(_)
            | Self::Scope(_)
            | Self::DepthLimit
            | Self::StepLimit
            | Self::EffortDenied
            | Self::RoleToolDenied
            | Self::TokenBudget => ErrorKind::Policy,
            Self::UsageExceeded { .. } | Self::InvalidUsage => ErrorKind::Fatal,
            Self::Capability(error) => error.kind(),

            Self::Model(error) => error.kind(),
            Self::Cancelled => ErrorKind::Transient,
            Self::OutputLimit | Self::IncompleteStream | Self::InvalidToolCall => {
                ErrorKind::Permanent
            }
            Self::Backend(kind, _) => *kind,
        }
    }
}

/// A frame carries its capability token and local constraints; secret-bearing values are omitted
/// from `Debug` and from watchdog views.
pub struct AgentContextFrame<S> {
    pub(crate) id: FrameId,
    pub(crate) parent: Option<FrameId>,
    pub(crate) role: AgentRole,
    pub(crate) token: Token,
    pub(crate) scope: FrameScope,
    pub(crate) remaining: Arc<Mutex<ResourceQuota>>,
    pub(crate) allocation: ResourceQuota,
    pub(crate) lease: Option<AttachedQuotaLease>,
    pub(crate) scratchpad: ScratchpadRef,
    pub(crate) history: Vec<DiffTimelineEntry>,
    pub(crate) steps: u32,
    pub(crate) fingerprint: Option<FastFingerprint>,
    pub(crate) depth: u8,
    pub(crate) model: Option<ModelId>,
    pub(crate) effort: Effort,
    _state: PhantomData<S>,
}

impl<S> std::fmt::Debug for AgentContextFrame<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContextFrame")
            .field("id", &self.id)
            .field("parent", &self.parent)
            .field("role", &self.role)
            .field("steps", &self.steps)
            .finish_non_exhaustive()
    }
}

impl AgentContextFrame<Spawned> {
    /// Create a trusted root frame from already-issued capability and quota state.
    /// The caller is responsible for validating that the token encodes `scope` before this call.
    pub fn new_root(
        id: FrameId,
        role: AgentRole,
        token: Token,
        scope: FrameScope,
        quota: ResourceQuota,
        scratchpad: ScratchpadRef,
    ) -> Self {
        Self {
            id,
            parent: None,
            role,
            token,
            scope,
            remaining: Arc::new(Mutex::new(quota.clone())),
            allocation: quota,
            lease: None,
            scratchpad,
            history: Vec::new(),
            steps: 0,
            fingerprint: None,
            depth: 1,
            model: None,
            effort: Effort::Low,
            _state: PhantomData,
        }
    }

    /// Activate with a model choice already resolved by the trusted composition root.
    pub fn activate(
        self,
        selection: ModelSelection,
        effort: Effort,
    ) -> Result<AgentContextFrame<Active>, FrameError> {
        if effort > selection.maximum_effort {
            return Err(FrameError::EffortDenied);
        }
        Ok(AgentContextFrame {
            id: self.id,
            parent: self.parent,
            role: self.role,
            token: self.token,
            scope: self.scope,
            remaining: self.remaining,
            allocation: self.allocation,
            lease: self.lease,
            scratchpad: self.scratchpad,
            history: self.history,
            steps: self.steps,
            fingerprint: self.fingerprint,
            depth: self.depth,
            model: Some(selection.model),
            effort,
            _state: PhantomData,
        })
    }
}

impl AgentContextFrame<Active> {
    /// Spawn a child only after proving scope, lifetime, network, and numeric quotas narrow.
    pub fn spawn_child(
        &mut self,
        role: AgentRole,
        child_scope: Scope,
        quota: ResourceQuota,
        scratchpad: ScratchpadRef,
        clock: &impl Clock,
    ) -> Result<AgentContextFrame<Spawned>, FrameError> {
        scope_is_subset(&self.scope.capability, &child_scope)?;
        let now = clock.now().as_microsecond();
        let expires_at_micros = child_expiry(
            self.scope.expires_at_micros,
            now,
            child_scope.ttl.as_micros(),
        )?;
        let next_depth = self.depth.checked_add(1).ok_or(FrameError::Accounting)?;
        if next_depth > self.allocation.max_depth || next_depth > quota.max_depth {
            return Err(FrameError::DepthLimit);
        }
        let child_token = attenuate(&self.token, &child_scope, clock)?;
        let lease = {
            let mut available = lock_recover(&self.remaining);
            available.reserve(&quota)?
        };
        let id = FrameId::new_v7();
        let remaining = Arc::new(Mutex::new(quota.clone()));
        let lease = Some(lease.attach(Arc::clone(&self.remaining), Arc::clone(&remaining)));
        Ok(AgentContextFrame {
            id,
            parent: Some(self.id),
            role,
            token: child_token,
            scope: FrameScope {
                capability: child_scope,
                expires_at_micros,
            },
            remaining,
            allocation: quota,
            lease,
            scratchpad,
            history: Vec::new(),
            steps: 0,
            fingerprint: None,
            depth: next_depth,
            model: None,
            effort: Effort::Low,
            _state: PhantomData,
        })
    }

    /// Suspend while preserving the frame's secret and quota state.
    pub fn suspend(self) -> AgentContextFrame<Suspended> {
        self.map_state()
    }

    /// Reserve one attempt before assembly or model work. Failed attempts still consume a step.
    pub(crate) fn begin_step(&mut self) -> Result<StepNo, FrameError> {
        let mut remaining = lock_recover(&self.remaining);
        if self.steps >= self.allocation.max_steps || remaining.max_steps == 0 {
            return Err(FrameError::StepLimit);
        }
        remaining.max_steps = remaining
            .max_steps
            .checked_sub(1)
            .ok_or(FrameError::Accounting)?;
        self.steps = self.steps.checked_add(1).ok_or(FrameError::Accounting)?;
        Ok(StepNo(self.steps))
    }

    /// Record trusted source evidence for an already-reserved step.
    pub(crate) fn complete_step(
        &mut self,
        step: StepNo,
        fingerprint: FastFingerprint,
        verified_diffs: Vec<Cid>,
    ) {
        self.fingerprint = Some(fingerprint);
        self.history.extend(
            verified_diffs
                .into_iter()
                .map(|diff| DiffTimelineEntry { step, diff }),
        );
    }

    /// Reserve a trusted prompt-token upper bound and the complete requested output cap before
    /// the provider call. Neither amount is refunded on missing usage, errors, or cancellation.
    pub(crate) fn reserve_model_tokens(
        &self,
        input_upper_bound: polaris_sdk::ids::TokenCount,
        requested_output: Option<polaris_sdk::ids::TokenCount>,
    ) -> Result<polaris_sdk::ids::TokenCount, FrameError> {
        let mut remaining = lock_recover(&self.remaining);
        let input = u64::from(input_upper_bound.get());
        let available = remaining
            .token_budget
            .checked_sub(input)
            .ok_or(FrameError::TokenBudget)?;
        let available_output = match u32::try_from(available) {
            Ok(value) => value,
            Err(_) => u32::MAX,
        };
        let output =
            requested_output.map_or(available_output, |count| count.get().min(available_output));
        if output == 0 {
            return Err(FrameError::TokenBudget);
        }
        let reserved = input
            .checked_add(u64::from(output))
            .ok_or(FrameError::Accounting)?;
        remaining.token_budget = remaining
            .token_budget
            .checked_sub(reserved)
            .ok_or(FrameError::Accounting)?;
        Ok(polaris_sdk::ids::TokenCount::new(output))
    }

    /// Reconcile provider-reported prompt/completion counts with the already-debited reservation.
    /// Completion usage must include all generated tokens, including reasoning; hidden reasoning
    /// has no separate SDK usage field. Under-reports never refund reservation. An over-report is
    /// charged from remaining budget when possible and always fails fatally before tools run.
    pub(crate) fn validate_reported_usage(
        &self,
        input_upper_bound: polaris_sdk::ids::TokenCount,
        output_limit: polaris_sdk::ids::TokenCount,
        usage: polaris_sdk::registry::Usage,
    ) -> Result<(), FrameError> {
        let prompt = u64::from(usage.prompt_tokens.get());
        let completion = u64::from(usage.completion_tokens.get());
        let cached = usage.cached_tokens.get();
        if cached > usage.prompt_tokens.get() {
            return Err(FrameError::InvalidUsage);
        }
        let observed = prompt
            .checked_add(completion)
            .ok_or(FrameError::Accounting)?;
        let reserved = u64::from(input_upper_bound.get())
            .checked_add(u64::from(output_limit.get()))
            .ok_or(FrameError::Accounting)?;
        if observed > reserved {
            let overage = observed
                .checked_sub(reserved)
                .ok_or(FrameError::Accounting)?;
            let mut remaining = lock_recover(&self.remaining);
            remaining.token_budget = match remaining.token_budget.checked_sub(overage) {
                Some(value) => value,
                None => {
                    remaining.token_budget = 0;
                    return Err(FrameError::Accounting);
                }
            };
            return Err(FrameError::UsageExceeded { observed, reserved });
        }
        if prompt > u64::from(input_upper_bound.get()) || completion > u64::from(output_limit.get())
        {
            return Err(FrameError::Quota(QuotaError::Exceeded(
                "reported model usage",
            )));
        }
        Ok(())
    }
    /// Number of reserved model/tool attempts in this frame.
    #[must_use]
    pub const fn steps(&self) -> u32 {
        self.steps
    }

    /// Ordered, verifier-supplied diff timeline.
    #[must_use]
    pub fn diff_timeline(&self) -> &[DiffTimelineEntry] {
        &self.history
    }
    /// Secret-free read-only snapshot for watchdogs.
    #[must_use]
    pub fn view(&self) -> FrameView {
        FrameView {
            id: self.id,
            parent: self.parent,
            role: self.role,
            fingerprint: self.fingerprint,
            steps: self.steps,
            scratchpad: self.scratchpad,
        }
    }

    /// Charge CPU measured by a trusted backend. This does not grant or start a process.
    pub fn charge_cpu_millis(&self, measured: u32) -> Result<(), FrameError> {
        let mut remaining = lock_recover(&self.remaining);
        remaining.cpu_millis = remaining
            .cpu_millis
            .checked_sub(measured)
            .ok_or(FrameError::Quota(QuotaError::Exceeded("cpu_millis")))?;
        Ok(())
    }

    /// Charge memory measured by a trusted backend. This does not allocate memory itself.
    pub fn charge_memory_mb(&self, measured: u32) -> Result<(), FrameError> {
        let mut remaining = lock_recover(&self.remaining);
        remaining.memory_mb = remaining
            .memory_mb
            .checked_sub(measured)
            .ok_or(FrameError::Quota(QuotaError::Exceeded("memory_mb")))?;
        Ok(())
    }

    /// End the frame. This does not assert proof-of-done; callers must independently verify before
    /// choosing `FrameOutcome::Completed`.
    pub fn finish(self, _outcome: FrameOutcome) -> AgentContextFrame<Finished> {
        self.map_state()
    }
}

impl AgentContextFrame<Suspended> {
    /// Resume a suspended frame. Its model and effort remain fixed to the prior selection.
    pub fn resume(self) -> AgentContextFrame<Active> {
        self.map_state()
    }
}

impl<S> AgentContextFrame<S> {
    fn map_state<T>(self) -> AgentContextFrame<T> {
        AgentContextFrame {
            id: self.id,
            parent: self.parent,
            role: self.role,
            token: self.token,
            scope: self.scope,
            remaining: self.remaining,
            allocation: self.allocation,
            lease: self.lease,
            scratchpad: self.scratchpad,
            history: self.history,
            steps: self.steps,
            fingerprint: self.fingerprint,
            depth: self.depth,
            model: self.model,
            effort: self.effort,
            _state: PhantomData,
        }
    }
}

/// Bounded active frame stack, ordered from root to current frame.
#[derive(Debug)]
pub struct FrameStack {
    frames: Vec<FrameId>,
    maximum_depth: usize,
}
impl Default for FrameStack {
    fn default() -> Self {
        Self::new(3)
    }
}
impl FrameStack {
    /// Create a stack with a bounded depth (the documented default is three).
    #[must_use]
    pub fn new(maximum_depth: usize) -> Self {
        Self {
            frames: Vec::new(),
            maximum_depth,
        }
    }
    /// Push a frame after checking its declared parent and stack bound.
    pub fn push(&mut self, id: FrameId, parent: Option<FrameId>) -> Result<(), FrameError> {
        if self.frames.len() >= self.maximum_depth {
            return Err(FrameError::DepthLimit);
        }
        if self.frames.last().copied() != parent {
            return Err(FrameError::Accounting);
        }
        self.frames.push(id);
        Ok(())
    }
    /// Remove only the current frame.
    pub fn pop(&mut self, id: FrameId) -> Result<(), FrameError> {
        match self.frames.last() {
            Some(current) if *current == id => {
                let _ = self.frames.pop();
                Ok(())
            }
            _ => Err(FrameError::Accounting),
        }
    }
    /// Current stack depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.frames.len()
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
