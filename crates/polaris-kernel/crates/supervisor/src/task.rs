//! Task ownership, one-for-one restart, and joined cancellation. User factories never execute
//! while a supervisor mutex is held, including their synchronous construction code.

use crate::{
    BreakerConfig, BreakerHandle, BreakerState, CircuitBreaker, DeadLetter, DeadLetterQueue,
    RestartIntensity, RestartVerdict, RestartWindow,
};
use backon::{BackoffBuilder, ExponentialBuilder};
use polaris_kernel_bus::Bus;
use polaris_sdk::contracts::{Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{Cid, FrameId, Seq, SessionId};
use polaris_sdk::registry::EventStore;
use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

// No user code runs under these locks. Tokio isolates user panics in a separate child task.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Stable diagnostic name with an explicit CAS identity (no secret text is placed in events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChildName {
    label: &'static str,
    cid: Cid,
}
impl ChildName {
    /// Pair a static diagnostic label with its CAS reference.
    #[must_use]
    pub const fn new(label: &'static str, cid: Cid) -> Self {
        Self { label, cid }
    }
    /// Diagnostic label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }
    /// CAS identity published in failure events.
    #[must_use]
    pub const fn cid(self) -> Cid {
        self.cid
    }
}
/// Globally unique task identity; another supervisor cannot accidentally accept this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChildId(FrameId);
/// Restart semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Restart even after a clean exit.
    Permanent,
    /// Restart only an abnormal exit.
    Transient,
    /// Never restart or escalate an exit.
    Temporary,
}
/// Exhausted restart budget behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnExhausted {
    /// Keep healthy siblings alive.
    StopChild,
    /// Cancel the root, allowing the composition root to shut down the entire tree.
    ShutdownSupervisor,
}
/// Last observed task exit class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    /// Returned `Ok(())`.
    Clean,
    /// Returned a typed error.
    Failed,
    /// Tokio isolated a panic.
    Panicked,
    /// Task was aborted or externally cancelled.
    Killed,
    /// Native worker ended by a signal or an OS exception status.
    Crashed,
}
/// Current child state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildStatus {
    /// An actor future is running.
    Running,
    /// Waiting through restart backoff.
    BackingOff,
    /// Exited without further restart.
    Exited(ExitClass),
    /// Budget or fatal-error escalation stopped the child.
    GaveUp,
    /// Explicitly stopped; no restart will follow.
    Stopped,
}
/// Joined stop result (no surviving actor future).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// Exited before the grace period expired.
    Graceful,
    /// Aborted after grace, then joined.
    Aborted,
}
/// Backoff configuration, implemented by `backon`'s exponential iterator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffSpec {
    /// Initial delay.
    pub min_delay: Duration,
    /// Maximum exponential base delay.
    pub max_delay: Duration,
    /// Random jitter in production; disable for deterministic tests.
    pub jitter: bool,
}
impl Default for BackoffSpec {
    fn default() -> Self {
        Self {
            min_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            jitter: true,
        }
    }
}
/// One task's supervision policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskSpec {
    /// Diagnostic child identity.
    pub name: ChildName,
    /// Clean/abnormal exit restart policy.
    pub policy: RestartPolicy,
    /// Sliding restart budget.
    pub intensity: RestartIntensity,
    /// Restart delays.
    pub backoff: BackoffSpec,
    /// Escalation when retries end.
    pub on_exhausted: OnExhausted,
    /// Optional worker mode used by durable restart telemetry.
    pub worker: Option<WorkerKind>,
}
impl TaskSpec {
    /// Permanent task, three restarts per ten seconds, jittered exponential backoff.
    #[must_use]
    pub fn new(name: ChildName) -> Self {
        Self {
            name,
            policy: RestartPolicy::Permanent,
            intensity: RestartIntensity::default(),
            backoff: BackoffSpec::default(),
            on_exhausted: OnExhausted::StopChild,
            worker: None,
        }
    }
}
/// Bounded supervisor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisorConfig {
    /// Maximum retained dead letters.
    pub dlq_capacity: NonZeroUsize,
    /// Maximum registered children, including retained completion records.
    pub max_children: NonZeroUsize,
}
impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            dlq_capacity: NonZeroUsize::MIN.saturating_add(255),
            max_children: NonZeroUsize::MIN.saturating_add(1023),
        }
    }
}
/// Error returned by user actor futures; the message is never emitted into bus/log fields.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("supervised task failed: {message}")]
pub struct TaskError {
    message: String,
    kind: ErrorKind,
    exit: ExitClass,
}
impl TaskError {
    /// Native crash, typed failure, or other classification retained by the supervisor.
    #[must_use]
    pub const fn exit_class(&self) -> ExitClass {
        self.exit
    }
    /// Retryable error.
    #[must_use]
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Transient,
            exit: ExitClass::Failed,
        }
    }
    /// Integrity failure; immediately escalated without retry.
    #[must_use]
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Fatal,
            exit: ExitClass::Failed,
        }
    }
    /// Native process signal/exception failure; restart policy still decides whether to retry.
    #[must_use]
    pub fn crashed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::Transient,
            exit: ExitClass::Crashed,
        }
    }
}
impl HasKind for TaskError {
    fn kind(&self) -> ErrorKind {
        self.kind
    }
}
/// Supervision API failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SupervisorError {
    /// Identity does not belong to this supervisor.
    #[error("unknown child")]
    UnknownChild,
    /// Root cancellation has started.
    #[error("supervisor is shutting down")]
    ShuttingDown,
    /// No further safe restart is allowed.
    #[error("restart intensity exceeded for {child:?}")]
    IntensityExceeded {
        /// Exhausted child.
        child: ChildName,
    },
    /// Invalid configuration, duplicate child or missing runtime.
    #[error("invalid supervision configuration: {0}")]
    Config(&'static str),
}
impl HasKind for SupervisorError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::UnknownChild | Self::ShuttingDown | Self::Config(_) => ErrorKind::Permanent,
            Self::IntensityExceeded { .. } => ErrorKind::Fatal,
        }
    }
}
/// One child status in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildSnapshot {
    /// Unique identity.
    pub id: ChildId,
    /// Diagnostic child name.
    pub name: ChildName,
    /// Current lifecycle status.
    pub status: ChildStatus,
    /// Total actual restarts.
    pub restarts: u32,
}
/// One circuit's admission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerSnapshot {
    /// Diagnostic circuit name.
    pub name: ChildName,
    /// Current admission state.
    pub state: BreakerState,
}
/// Consistent snapshots of each independently owned component (not an atomic global timestamp).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorSnapshot {
    /// Retained children in identity order.
    pub children: Vec<ChildSnapshot>,
    /// Registered circuits in name order.
    pub breakers: Vec<BreakerSnapshot>,
    /// Retained dead letters.
    pub dlq_len: usize,
    /// Dead-letter evictions.
    pub dlq_dropped: u64,
    /// Event publications that failed; these failures never stop the supervision loop.
    pub events_lost: u64,
}
type Completion = Option<Result<ChildStatus, SupervisorError>>;
struct Child {
    snapshot: Mutex<ChildSnapshot>,
    token: CancellationToken,
    abort: Mutex<Option<AbortHandle>>,
    completion: watch::Sender<Completion>,
}
pub(crate) struct Inner<S> {
    bus: Arc<Bus<S>>,
    session: SessionId,
    root: CancellationToken,
    config: SupervisorConfig,
    children: Mutex<BTreeMap<ChildId, Arc<Child>>>,
    breakers: Mutex<BTreeMap<ChildName, Arc<Mutex<CircuitBreaker>>>>,
    dlq: Mutex<DeadLetterQueue<Cid>>,
    events_lost: AtomicU64,
    tracker: TaskTracker,
}
impl<S: EventStore + 'static> Inner<S> {
    pub(crate) async fn emit(&self, event: Event) {
        let bus = Arc::clone(&self.bus);
        let record = EventRecord::new(Seq::ZERO, Timestamp::now(), self.session, event);
        let result = tokio::task::spawn_blocking(move || bus.publish(record)).await;
        if !matches!(result, Ok(Ok(_))) {
            let _previous =
                self.events_lost
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                        Some(count.saturating_add(1))
                    });
        }
    }
    async fn dead_letter(&self, item: Cid, child: ChildName, reason: ExitClass, attempts: u32) {
        let _evicted = lock(&self.dlq).push(DeadLetter {
            item,
            child,
            reason,
            attempts,
        });
        self.emit(Event::DeadLettered { message: item }).await;
    }
}

/// Owner of every actor and monitoring task. Explicit shutdown joins both layers; Drop requests
/// cancellation and aborts actors as a best effort, but cannot join asynchronously.
pub struct Supervisor<S: EventStore + 'static> {
    inner: Arc<Inner<S>>,
}
impl<S: EventStore + 'static> core::fmt::Debug for Supervisor<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Supervisor")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}
impl<S: EventStore + 'static> Supervisor<S> {
    /// Construct over an existing durable bus, session and cancellation root.
    #[must_use]
    pub fn new(
        bus: Arc<Bus<S>>,
        session: SessionId,
        root: CancellationToken,
        config: SupervisorConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                bus,
                session,
                root,
                config,
                children: Mutex::new(BTreeMap::new()),
                breakers: Mutex::new(BTreeMap::new()),
                dlq: Mutex::new(DeadLetterQueue::new(config.dlq_capacity)),
                events_lost: AtomicU64::new(0),
                tracker: TaskTracker::new(),
            }),
        }
    }
    /// Spawn a monitored actor factory. Both factory construction and polling are panic-isolated.
    ///
    /// # Errors
    /// Rejects invalid delays, missing runtime, duplicate names, capacity overflow or shutdown.
    pub fn spawn_task<F, Fut>(&self, spec: TaskSpec, make: F) -> Result<ChildId, SupervisorError>
    where
        F: Fn(CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), TaskError>> + Send + 'static,
    {
        if self.inner.root.is_cancelled() {
            return Err(SupervisorError::ShuttingDown);
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| SupervisorError::Config("Tokio runtime required"))?;
        if spec.backoff.min_delay > spec.backoff.max_delay || spec.backoff.max_delay.is_zero() {
            return Err(SupervisorError::Config("invalid backoff bounds"));
        }
        let id = ChildId(FrameId::new_v7());
        let (completion, _initial) = watch::channel(None);
        let child = Arc::new(Child {
            snapshot: Mutex::new(ChildSnapshot {
                id,
                name: spec.name,
                status: ChildStatus::Running,
                restarts: 0,
            }),
            token: self.inner.root.child_token(),
            abort: Mutex::new(None),
            completion,
        });
        {
            let mut children = lock(&self.inner.children);
            if self.inner.root.is_cancelled() {
                return Err(SupervisorError::ShuttingDown);
            }
            if children.len() >= self.inner.config.max_children.get() {
                return Err(SupervisorError::Config("child capacity exceeded"));
            }
            if children
                .values()
                .any(|other| lock(&other.snapshot).name == spec.name)
            {
                return Err(SupervisorError::Config("duplicate child name"));
            }
            let _previous = children.insert(id, Arc::clone(&child));
            // Registration and monitor tracking are one operation relative to shutdown.
            let inner = Arc::clone(&self.inner);
            let _monitor = self.inner.tracker.spawn_on(
                async move {
                    let result = run(&inner, &child, spec, Arc::new(make)).await;
                    let _previous = child.completion.send_replace(Some(result));
                },
                &runtime,
            );
        }
        Ok(id)
    }
    fn child(&self, id: ChildId) -> Result<Arc<Child>, SupervisorError> {
        lock(&self.inner.children)
            .get(&id)
            .cloned()
            .ok_or(SupervisorError::UnknownChild)
    }
    /// Wait for terminal status; repeated and concurrent waits are supported.
    ///
    /// # Errors
    /// Unknown id, or the child's recorded restart escalation.
    pub async fn wait(&self, id: ChildId) -> Result<ChildStatus, SupervisorError> {
        let child = self.child(id)?;
        completion(&child).await
    }
    /// Cancel, wait through grace, then abort and join the actor.
    ///
    /// # Errors
    /// Returns `UnknownChild` for an id owned elsewhere.
    pub async fn stop(&self, id: ChildId, grace: Duration) -> Result<StopOutcome, SupervisorError> {
        let child = self.child(id)?;
        child.token.cancel();
        if tokio::time::timeout(grace, completion(&child))
            .await
            .is_ok()
        {
            return Ok(StopOutcome::Graceful);
        }
        if let Some(abort) = lock(&child.abort).as_ref() {
            abort.abort();
        }
        let _terminal = completion(&child).await;
        Ok(StopOutcome::Aborted)
    }
    /// Cancel the whole tree and join all actor/monitor tasks under one shared grace period.
    /// Abort stragglers after grace, then join their cancellation to release owned resources.
    pub async fn shutdown(&self, grace: Duration) -> StopOutcome {
        self.inner.root.cancel();
        // Synchronise with registration before closing the tracker.
        {
            let _registered = lock(&self.inner.children);
            self.inner.tracker.close();
        }
        if tokio::time::timeout(grace, self.inner.tracker.wait())
            .await
            .is_ok()
        {
            return StopOutcome::Graceful;
        }
        for child in lock(&self.inner.children).values() {
            if let Some(abort) = lock(&child.abort).as_ref() {
                abort.abort();
            }
        }
        self.inner.tracker.wait().await;
        StopOutcome::Aborted
    }
    /// Queue failed work and publish its identity, even when an older letter was evicted.
    pub async fn dead_letter(&self, item: Cid, child: ChildName, reason: ExitClass, attempts: u32) {
        self.inner.dead_letter(item, child, reason, attempts).await;
    }
    /// Drain retained failed work without resetting eviction counters.
    pub fn drain_dead_letters(&self) -> Vec<DeadLetter<Cid>> {
        lock(&self.inner.dlq).drain()
    }
    /// Get or register a shared circuit; the first policy registered for a name is authoritative.
    #[must_use]
    pub fn breaker(&self, name: ChildName, config: BreakerConfig) -> BreakerHandle<S> {
        let state = Arc::clone(lock(&self.inner.breakers).entry(name).or_insert_with(|| {
            Arc::new(Mutex::new(CircuitBreaker::new(
                config.max_failures,
                config.window,
                config.cooldown,
            )))
        }));
        BreakerHandle {
            name,
            state,
            inner: Arc::clone(&self.inner),
        }
    }
    /// Observe statuses and failure-accounting counters.
    #[must_use]
    pub fn snapshot(&self) -> SupervisorSnapshot {
        let children = lock(&self.inner.children)
            .values()
            .map(|child| *lock(&child.snapshot))
            .collect();
        let breakers = lock(&self.inner.breakers)
            .iter()
            .map(|(name, state)| BreakerSnapshot {
                name: *name,
                state: lock(state).state(),
            })
            .collect();
        let dlq = lock(&self.inner.dlq);
        SupervisorSnapshot {
            children,
            breakers,
            dlq_len: dlq.len(),
            dlq_dropped: dlq.dropped(),
            events_lost: self.inner.events_lost.load(Ordering::Relaxed),
        }
    }
}
impl<S: EventStore + 'static> Drop for Supervisor<S> {
    fn drop(&mut self) {
        self.inner.root.cancel();
        for child in lock(&self.inner.children).values() {
            if let Some(abort) = lock(&child.abort).as_ref() {
                abort.abort();
            }
        }
    }
}
async fn completion(child: &Child) -> Result<ChildStatus, SupervisorError> {
    let mut receiver = child.completion.subscribe();
    loop {
        if let Some(result) = receiver.borrow_and_update().clone() {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(SupervisorError::ShuttingDown);
        }
    }
}
async fn run<S, F, Fut>(
    inner: &Arc<Inner<S>>,
    child: &Arc<Child>,
    spec: TaskSpec,
    make: Arc<F>,
) -> Result<ChildStatus, SupervisorError>
where
    S: EventStore + 'static,
    F: Fn(CancellationToken) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), TaskError>> + Send + 'static,
{
    let mut budget = RestartWindow::new(spec.intensity);
    let mut builder = ExponentialBuilder::default()
        .with_min_delay(spec.backoff.min_delay)
        .with_max_delay(spec.backoff.max_delay)
        .without_max_times();
    if spec.backoff.jitter {
        builder = builder.with_jitter();
    }
    let mut delays = builder.build();
    let mut attempts = 0u32;
    loop {
        if child.token.is_cancelled() {
            lock(&child.snapshot).status = ChildStatus::Stopped;
            return Ok(ChildStatus::Stopped);
        }
        attempts = attempts.saturating_add(1);
        lock(&child.snapshot).status = ChildStatus::Running;
        let factory = Arc::clone(&make);
        let token = child.token.clone();
        let actor = tokio::spawn(async move { factory(token).await });
        *lock(&child.abort) = Some(actor.abort_handle());
        let result = actor.await;
        *lock(&child.abort) = None;
        if child.token.is_cancelled() {
            lock(&child.snapshot).status = ChildStatus::Stopped;
            return Ok(ChildStatus::Stopped);
        }
        let (exit, fatal) = match result {
            Ok(Ok(())) => (ExitClass::Clean, false),
            Ok(Err(error)) => (error.exit, error.kind() == ErrorKind::Fatal),
            Err(error) if error.is_panic() => (ExitClass::Panicked, false),
            Err(_) => (ExitClass::Killed, false),
        };
        if spec.policy == RestartPolicy::Temporary
            || (spec.policy == RestartPolicy::Transient && exit == ExitClass::Clean)
        {
            let status = ChildStatus::Exited(exit);
            lock(&child.snapshot).status = status;
            return Ok(status);
        }
        let restarts_in_window = if fatal {
            None
        } else {
            match budget.record(Instant::now()) {
                RestartVerdict::Allowed { restarts_in_window } => Some(restarts_in_window),
                RestartVerdict::Exceeded => None,
            }
        };
        let Some(restarts_in_window) = restarts_in_window else {
            lock(&child.snapshot).status = ChildStatus::GaveUp;
            inner
                .emit(Event::CircuitTripped {
                    breaker: spec.name.cid(),
                })
                .await;
            inner
                .dead_letter(spec.name.cid(), spec.name, exit, attempts)
                .await;
            if spec.on_exhausted == OnExhausted::ShutdownSupervisor {
                inner.root.cancel();
            }
            return Err(SupervisorError::IntensityExceeded { child: spec.name });
        };
        lock(&child.snapshot).status = ChildStatus::BackingOff;
        let delay = match delays.next() {
            Some(delay) => delay.min(spec.backoff.max_delay),
            None => spec.backoff.max_delay,
        };
        tokio::select! { biased; () = child.token.cancelled() => continue, () = tokio::time::sleep(delay) => {} }
        if child.token.is_cancelled() {
            continue;
        }
        if let Some(worker) = spec.worker {
            inner
                .emit(Event::WorkerRestarted {
                    worker,
                    restarts: restarts_in_window,
                })
                .await;
        }
        let mut snapshot = lock(&child.snapshot);
        snapshot.restarts = snapshot.restarts.saturating_add(1);
    }
}
