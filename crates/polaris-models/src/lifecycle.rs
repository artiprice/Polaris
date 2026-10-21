//! Shared loader work and lease-counted draining. No state lock is held across an await.
use core::{fmt, future::Future};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::{sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering}}, time::{Duration, Instant}};
use tokio::{runtime::Handle, sync::Notify, task::JoinHandle};
use crate::{LoadError, ModelsError};

/// Real backend lifecycle contract. `M` must own its resources through RAII as a shutdown fallback.
/// Load/unload futures must be cancellation-safe: their operation timeout may drop the future.
pub trait ModelLoader<M>: Send + Sync + 'static {
    /// Load the actual model or worker resource; never return a dummy success object.
    fn load(&self) -> impl Future<Output = Result<M, LoadError>> + Send;
    /// Release backend resources after all leases are gone. Unload must be idempotent so a
    /// timeout/failure can be retried without claiming memory was successfully released.
    fn unload(&self, model: &M) -> impl Future<Output = Result<(), LoadError>> + Send;
}

/// Operational slot bounds; no default latency is a measured result.
#[derive(Debug, Clone, Copy)]
pub struct SlotConfig {
    /// Idle interval since load or last lease release before voluntary eviction.
    pub idle_ttl: Duration,
    /// Maximum concurrent leases.
    pub max_leases: u32,
    /// Shared load operation timeout.
    pub load_timeout: Duration,
    /// Actual unload operation timeout, after lease draining completes.
    pub unload_timeout: Duration,
}

impl Default for SlotConfig {
    fn default() -> Self {
        Self { idle_ttl: Duration::from_secs(45), max_leases: 1024,
            load_timeout: Duration::from_secs(300), unload_timeout: Duration::from_secs(30) }
    }
}

/// Observable lifecycle state, with no backend object or sensitive configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// No loaded resource.
    Cold,
    /// One shared real load is running.
    Loading,
    /// Loaded and accepting new leases.
    Active {
        /// Current lease count.
        leases: u32,
    },
    /// Existing users may finish; admissions are closed.
    Draining {
        /// Remaining leases.
        leases: u32,
    },
    /// Backend unload is running after the last lease ended.
    Unloading,
    /// Backend work failed; explicit retry/eviction is required.
    Failed,
    /// Permanent shutdown.
    Closed,
}

/// What an eviction request actually established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictOutcome {
    /// Real unload completed successfully and the slot released its model handle.
    Evicted,
    /// It was already unloaded.
    NotLoaded,
    /// Idle policy did not select this slot.
    NotIdle,
    /// Deadline elapsed while leases remained; draining continues in shared owner work.
    StillInUse {
        /// Lease count at the deadline.
        leases: u32,
    },
    /// Deadline elapsed during another lifecycle operation; owner work continues.
    Pending {
        /// State observed at the deadline.
        state: SlotState,
    },
    /// Actual load/unload failed; no successful memory-release claim is made.
    Failed(LoadError),
}

struct Tracker {
    leases: AtomicU32,
    last_used: AtomicU64,
    broken: AtomicBool,
    changed: Arc<Notify>,
    epoch: Instant,
}

impl Tracker {
    fn now(&self) -> u64 {
        match u64::try_from(self.epoch.elapsed().as_micros()) { Ok(now) => now, Err(_) => u64::MAX }
    }
}

/// A live resource borrow. The model reference cannot outlive this non-cloneable lease.
pub struct ModelLease<M> { model: Option<Arc<M>>, tracker: Arc<Tracker> }

impl<M> ModelLease<M> {
    /// Access the actual loaded resource while the lease is held.
    #[must_use]
    pub fn model(&self) -> Result<&M, ModelsError> {
        self.model.as_deref().ok_or(ModelsError::Integrity)
    }
}

impl<M> fmt::Debug for ModelLease<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_struct("ModelLease").finish_non_exhaustive() }
}

impl<M> Drop for ModelLease<M> {
    fn drop(&mut self) {
        let release = self.model.take();
        if let Some(model) = release {
            if catch_unwind(AssertUnwindSafe(|| drop(model))).is_err() {
                self.tracker.broken.store(true, Ordering::Release);
            }
        }
        self.tracker.last_used.store(self.tracker.now(), Ordering::Release);
        if self.tracker.leases.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| count.checked_sub(1)).is_err() {
            self.tracker.broken.store(true, Ordering::Release);
        }
        self.tracker.changed.notify_waiters();
    }
}

enum Phase<M> {
    Cold, Loading, Active(Arc<M>), Draining(Arc<M>), Unloading(Arc<M>), Releasing,
    Failed { model: Option<Arc<M>>, error: LoadError, retry_load: bool }, Closed,
}

struct State<M> {
    phase: Phase<M>, paused: bool, closing: bool, eviction_requested: bool,
    generation: u64, work: Option<JoinHandle<()>>, eviction_succeeded: bool,
}

struct Inner<M, L> {
    loader: L, config: SlotConfig, state: Mutex<State<M>>, tracker: Arc<Tracker>,
}

/// Single-flight load, RAII usage leases, idle eviction and forced draining.
///
/// Shared worker tasks own load/drain operations even if the initiating caller is canceled.
/// `force_evict` pauses admission until `resume` so a heavy build does not race an immediate
/// automatic reload. The composition root must call `close` during orderly shutdown.
pub struct EvictableSlot<M, L> { inner: Arc<Inner<M, L>> }

impl<M, L> Clone for EvictableSlot<M, L> {
    fn clone(&self) -> Self { Self { inner: Arc::clone(&self.inner) } }
}

impl<M, L> fmt::Debug for EvictableSlot<M, L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_struct("EvictableSlot").finish_non_exhaustive() }
}

fn lock<M, L>(inner: &Inner<M, L>) -> Result<MutexGuard<'_, State<M>>, ModelsError> {
    if inner.tracker.broken.load(Ordering::Acquire) { return Err(ModelsError::Integrity); }
    inner.state.lock().map_err(|_| ModelsError::Integrity)
}

fn view<M>(state: &State<M>, leases: u32) -> SlotState {
    match &state.phase {
        Phase::Cold => SlotState::Cold,
        Phase::Loading => SlotState::Loading,
        Phase::Active(_) if state.paused => SlotState::Draining { leases },
        Phase::Active(_) => SlotState::Active { leases },
        Phase::Draining(_) => SlotState::Draining { leases },
        Phase::Unloading(_) | Phase::Releasing => SlotState::Unloading,
        Phase::Failed { .. } => SlotState::Failed,
        Phase::Closed => SlotState::Closed,
    }
}

fn retained<M>(phase: &Phase<M>) -> Option<Arc<M>> {
    match phase {
        Phase::Active(model) | Phase::Draining(model) | Phase::Unloading(model) => Some(Arc::clone(model)),
        Phase::Failed { model, .. } => model.clone(),
        Phase::Cold | Phase::Loading | Phase::Releasing | Phase::Closed => None,
    }
}

fn recover_finished<M>(state: &mut State<M>) {
    // A runtime can cancel tasks during shutdown. A later caller must not wait forever for a
    // notification from work whose handle is authoritatively finished.
    if state.work.as_ref().is_some_and(JoinHandle::is_finished) {
        let model = retained(&state.phase);
        let retry_load = matches!(&state.phase, Phase::Loading);
        state.phase = Phase::Failed { model, error: LoadError::TaskFailed, retry_load };
        state.eviction_succeeded = false;
        let _finished = state.work.take();
    }
}

#[derive(Clone, Copy)]
enum WorkKind {
    Load,
    Drain,
}

/// Ensures an interrupted worker fails its generation closed and wakes every waiter.
struct WorkDropGuard<M, L> {
    owner: Arc<Inner<M, L>>,
    generation: u64,
    kind: WorkKind,
    fallback: Option<Arc<M>>,
    armed: bool,
}

impl<M, L> WorkDropGuard<M, L> {
    fn new(owner: Arc<Inner<M, L>>, generation: u64, kind: WorkKind, fallback: Option<Arc<M>>) -> Self {
        Self { owner, generation, kind, fallback, armed: true }
    }

    fn finish(&mut self) {
        self.armed = false;
    }
}

impl<M, L> Drop for WorkDropGuard<M, L> {
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut state = match self.owner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                self.owner.tracker.broken.store(true, Ordering::Release);
                poisoned.into_inner()
            }
        };
        if state.generation == self.generation {
            let interrupted = match self.kind {
                WorkKind::Load => matches!(&state.phase, Phase::Loading | Phase::Active(_)),
                WorkKind::Drain => matches!(&state.phase, Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing),
            };
            if interrupted {
                let retry_load = matches!(self.kind, WorkKind::Load);
                state.phase = Phase::Failed {
                    model: self.fallback.clone(),
                    error: LoadError::TaskFailed,
                    retry_load,
                };
                let _work = state.work.take();
                state.eviction_succeeded = false;
            }
        }
        drop(state);
        if let Some(fallback) = self.fallback.take() {
            if catch_unwind(AssertUnwindSafe(|| drop(fallback))).is_err() {
                self.owner.tracker.broken.store(true, Ordering::Release);
            }
        }
        self.owner.tracker.changed.notify_waiters();
    }
}

impl<M: Send + Sync + 'static, L: ModelLoader<M>> EvictableSlot<M, L> {
    /// Create an unloaded slot without starting a worker or simulating resource availability.
    ///
    /// # Errors
    /// Invalid zero bounds.
    pub fn new(loader: L, config: SlotConfig) -> Result<Self, ModelsError> {
        if config.max_leases == 0 || config.load_timeout.is_zero() || config.unload_timeout.is_zero() {
            return Err(ModelsError::Config("slot bounds"));
        }
        let tracker = Arc::new(Tracker { leases: AtomicU32::new(0), last_used: AtomicU64::new(0),
            broken: AtomicBool::new(false), changed: Arc::new(Notify::new()), epoch: Instant::now() });
        Ok(Self { inner: Arc::new(Inner { loader, config, tracker, state: Mutex::new(State {
            phase: Phase::Cold, paused: false, closing: false, eviction_requested: false,
            generation: 0, work: None, eviction_succeeded: false,
        }) }) })
    }

    /// Acquire a live model lease, sharing any in-flight load with concurrent callers.
    ///
    /// # Errors
    /// Closed/paused admissions, actual loader failure, lease limits or state integrity failure.
    pub async fn acquire(&self) -> Result<ModelLease<M>, ModelsError> {
        let runtime = Handle::try_current().map_err(|_| ModelsError::NoRuntime)?;
        loop {
            let notified = self.inner.tracker.changed.notified();
            tokio::pin!(notified);
            let _registered = notified.as_mut().enable();
            {
                let mut state = lock(&self.inner)?;
                recover_finished(&mut state);
                if state.closing { return Err(ModelsError::Closed); }
                if state.paused { return Err(ModelsError::Draining); }
                match &state.phase {
                    Phase::Active(model) => {
                        let model = Arc::clone(model);
                        self.inner.tracker.leases.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                            if count < self.inner.config.max_leases { count.checked_add(1) } else { None }
                        }).map_err(|_| ModelsError::LeaseLimit)?;
                        self.inner.tracker.last_used.store(self.inner.tracker.now(), Ordering::Release);
                        return Ok(ModelLease { model: Some(model), tracker: Arc::clone(&self.inner.tracker) });
                    },
                    Phase::Cold => start_load(&self.inner, &mut state, &runtime)?,
                    Phase::Failed { error, .. } => return Err(ModelsError::Load(error.clone())),
                    Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing => return Err(ModelsError::Draining),
                    Phase::Closed => return Err(ModelsError::Closed),
                    Phase::Loading => {},
                }
            }
            notified.await;
        }
    }

    /// Observe current state, detecting finished worker handles rather than assuming liveness.
    ///
    /// # Errors
    /// Private synchronization/accounting integrity failure.
    pub fn state(&self) -> Result<SlotState, ModelsError> {
        let mut state = lock(&self.inner)?;
        recover_finished(&mut state);
        Ok(view(&state, self.inner.tracker.leases.load(Ordering::Acquire)))
    }

    /// Time since the slot was loaded or its most recent lease was released.
    ///
    /// # Errors
    /// State synchronization or lease accounting failure.
    pub fn idle_for(&self) -> Result<Duration, ModelsError> {
        let _state = lock(&self.inner)?;
        let elapsed = self.inner.tracker.now().saturating_sub(
            self.inner.tracker.last_used.load(Ordering::Acquire),
        );
        Ok(Duration::from_micros(elapsed))
    }

    /// Explicitly allow another load after a failed load. Failed unloads require eviction retry.
    ///
    /// # Errors
    /// Closed slot, retained resource after unload failure, or state integrity failure.
    pub fn retry_failed_load(&self) -> Result<(), ModelsError> {
        let mut state = lock(&self.inner)?;
        recover_finished(&mut state);
        if state.closing { return Err(ModelsError::Closed); }
        match &state.phase {
            Phase::Failed { model: None, retry_load: true, .. } => state.phase = Phase::Cold,
            Phase::Failed { .. } => return Err(ModelsError::Draining),
            Phase::Cold | Phase::Active(_) | Phase::Loading | Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing | Phase::Closed => {
                return Err(ModelsError::Config("no failed load to retry"));
            },
        }
        self.inner.tracker.changed.notify_waiters();
        Ok(())
    }

    /// Resume admissions only after forced eviction work is no longer draining/loading.
    ///
    /// # Errors
    /// Permanent shutdown, unfinished drain or state failure.
    pub fn resume(&self) -> Result<(), ModelsError> {
        let mut state = lock(&self.inner)?;
        recover_finished(&mut state);
        if state.closing { return Err(ModelsError::Closed); }
        if !matches!(&state.phase, Phase::Cold | Phase::Active(_)) { return Err(ModelsError::Draining); }
        state.paused = false;
        state.eviction_requested = false;
        self.inner.tracker.changed.notify_waiters();
        Ok(())
    }

    /// Evict only a loaded, unused model idle past its TTL. New acquisition is refused during
    /// unloading, then permitted again once the slot is cold.
    ///
    /// # Errors
    /// Missing runtime or state synchronization failure.
    pub async fn evict_if_idle(&self) -> Result<EvictOutcome, ModelsError> {
        let runtime = Handle::try_current().map_err(|_| ModelsError::NoRuntime)?;
        {
            let mut state = lock(&self.inner)?;
            recover_finished(&mut state);
            let leases = self.inner.tracker.leases.load(Ordering::Acquire);
            let since = self.inner.tracker.now().saturating_sub(self.inner.tracker.last_used.load(Ordering::Acquire));
            match &state.phase {
                Phase::Cold | Phase::Closed => return Ok(EvictOutcome::NotLoaded),
                Phase::Active(_) if leases == 0 && Duration::from_micros(since) >= self.inner.config.idle_ttl => {
                    state.eviction_requested = true;
                    start_drain(&self.inner, &mut state, &runtime)?;
                },
                Phase::Active(_) | Phase::Loading | Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing | Phase::Failed { .. } => return Ok(EvictOutcome::NotIdle),
            }
        }
        self.wait_evicted(self.inner.config.unload_timeout).await
    }

    /// Pause admission and request owner-managed draining. A deadline does not cancel shared
    /// lifecycle work, and the result never claims eviction while leases remain.
    ///
    /// # Errors
    /// Missing runtime or private state integrity failure.
    pub async fn force_evict(&self, deadline: Duration) -> Result<EvictOutcome, ModelsError> {
        self.request_eviction(false)?;
        self.wait_evicted(deadline).await
    }

    /// Permanently close admission and drain for orderly shutdown. Existing lease holders may
    /// finish; timing out reports pending work and does not unload a live lease.
    ///
    /// # Errors
    /// Missing runtime or private state integrity failure.
    pub async fn close(&self, deadline: Duration) -> Result<EvictOutcome, ModelsError> {
        self.request_eviction(true)?;
        self.wait_evicted(deadline).await
    }

    fn request_eviction(&self, closing: bool) -> Result<(), ModelsError> {
        let runtime = Handle::try_current().map_err(|_| ModelsError::NoRuntime)?;
        let mut state = lock(&self.inner)?;
        recover_finished(&mut state);
        state.paused = true;
        state.closing |= closing;
        state.eviction_requested = true;
        state.eviction_succeeded = false;
        match &state.phase {
            Phase::Cold => { if state.closing { state.phase = Phase::Closed; } },
            Phase::Closed => {},
            Phase::Active(_) | Phase::Failed { model: Some(_), .. } => start_drain(&self.inner, &mut state, &runtime)?,
            Phase::Failed { model: None, .. } => {},
            Phase::Loading | Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing => {},
        }
        self.inner.tracker.changed.notify_waiters();
        Ok(())
    }

    async fn wait_evicted(&self, deadline: Duration) -> Result<EvictOutcome, ModelsError> {
        let wait = async {
            loop {
                let notified = self.inner.tracker.changed.notified();
                tokio::pin!(notified);
                let _registered = notified.as_mut().enable();
                {
                    let mut state = lock(&self.inner)?;
                    recover_finished(&mut state);
                    match &state.phase {
                        Phase::Cold => {
                            return Ok(if state.eviction_succeeded {
                                EvictOutcome::Evicted
                            } else {
                                EvictOutcome::NotLoaded
                            });
                        },
                        Phase::Closed => {
                            return Ok(if state.eviction_succeeded {
                                EvictOutcome::Evicted
                            } else {
                                EvictOutcome::NotLoaded
                            });
                        },
                        Phase::Failed { error, .. } => return Ok(EvictOutcome::Failed(error.clone())),
                        Phase::Loading | Phase::Active(_) | Phase::Draining(_) | Phase::Unloading(_) | Phase::Releasing => {},
                    }
                }
                notified.await;
            }
        };
        match tokio::time::timeout(deadline, wait).await {
            Ok(result) => result,
            Err(_) => {
                let leases = self.inner.tracker.leases.load(Ordering::Acquire);
                if leases > 0 { Ok(EvictOutcome::StillInUse { leases }) }
                else { Ok(EvictOutcome::Pending { state: self.state()? }) }
            },
        }
    }
}

fn start_load<M: Send + Sync + 'static, L: ModelLoader<M>>(
    inner: &Arc<Inner<M, L>>, state: &mut State<M>, runtime: &Handle,
) -> Result<(), ModelsError> {
    if inner.tracker.leases.load(Ordering::Acquire) != 0 { return Err(ModelsError::Integrity); }
    state.generation = state.generation.checked_add(1).ok_or(ModelsError::Integrity)?;
    let generation = state.generation;
    state.phase = Phase::Loading;
    state.eviction_succeeded = false;
    let owner = Arc::clone(inner);
    let worker = WorkDropGuard::new(Arc::clone(&owner), generation, WorkKind::Load, None);
    state.work = Some(runtime.spawn(async move {
        let mut work = worker;
        let result = match tokio::time::timeout(owner.config.load_timeout, owner.loader.load()).await {
            Ok(result) => result,
            Err(_) => Err(LoadError::Timeout),
        };
        if let Ok(mut state) = lock(&owner) {
            if state.generation != generation || !matches!(&state.phase, Phase::Loading) {
                owner.tracker.broken.store(true, Ordering::Release);
            } else {
                let _previous = state.work.take();
                match result {
                    Ok(model) => {
                        let model = Arc::new(model);
                        state.phase = Phase::Active(Arc::clone(&model));
                        work.fallback = Some(Arc::clone(&model));
                        owner.tracker.last_used.store(owner.tracker.now(), Ordering::Release);
                        if state.eviction_requested {
                            match Handle::try_current() {
                                Ok(current) => {
                                    if start_drain(&owner, &mut state, &current).is_err() {
                                        state.phase = Phase::Failed { model: Some(Arc::clone(&model)), error: LoadError::TaskFailed, retry_load: false };
                                    } else {
                                        work.fallback = None;
                                    }
                                },
                                Err(_) => state.phase = Phase::Failed { model: Some(Arc::clone(&model)), error: LoadError::TaskFailed, retry_load: false },
                            }
                        }
                    },
                    Err(error) => state.phase = Phase::Failed { model: None, error, retry_load: true },
                }
                work.finish();
            }
        } else {
            owner.tracker.broken.store(true, Ordering::Release);
        }
        drop(work);
        owner.tracker.changed.notify_waiters();
    }));
    Ok(())
}

fn start_drain<M: Send + Sync + 'static, L: ModelLoader<M>>(
    inner: &Arc<Inner<M, L>>, state: &mut State<M>, runtime: &Handle,
) -> Result<(), ModelsError> {
    let model = retained(&state.phase).ok_or(ModelsError::Integrity)?;
    state.generation = state.generation.checked_add(1).ok_or(ModelsError::Integrity)?;
    let generation = state.generation;
    state.phase = Phase::Draining(Arc::clone(&model));
    state.eviction_succeeded = false;
    let owner = Arc::clone(inner);
    let worker = WorkDropGuard::new(Arc::clone(&owner), generation, WorkKind::Drain, Some(Arc::clone(&model)));
    state.work = Some(runtime.spawn(async move {
        let mut work = worker;
        loop {
            let notified = owner.tracker.changed.notified();
            tokio::pin!(notified);
            let _registered = notified.as_mut().enable();
            let ready = match lock(&owner) {
                Ok(mut state) => {
                    if state.generation != generation { owner.tracker.broken.store(true, Ordering::Release); return; }
                    if owner.tracker.leases.load(Ordering::Acquire) == 0 {
                        state.phase = Phase::Unloading(Arc::clone(&model));
                        true
                    } else { false }
                },
                Err(_) => { owner.tracker.broken.store(true, Ordering::Release); return; },
            };
            if ready { break; }
            notified.await;
        }

        let result = match tokio::time::timeout(owner.config.unload_timeout, owner.loader.unload(model.as_ref())).await {
            Ok(result) => result,
            Err(_) => Err(LoadError::Timeout),
        };
        if let Err(error) = result {
            if let Ok(mut state) = lock(&owner) {
                if state.generation == generation {
                    let _previous = state.work.take();
                    state.phase = Phase::Failed { model: Some(Arc::clone(&model)), error, retry_load: false };
                    state.eviction_succeeded = false;
                    work.finish();
                } else {
                    owner.tracker.broken.store(true, Ordering::Release);
                }
            } else {
                owner.tracker.broken.store(true, Ordering::Release);
            }
            drop(work);
            owner.tracker.changed.notify_waiters();
            return;
        }

        let retired = match lock(&owner) {
            Ok(mut state) if state.generation == generation => {
                let phase = core::mem::replace(&mut state.phase, Phase::Releasing);
                let _previous = state.work.take();
                match phase {
                    Phase::Unloading(retired) => Some(retired),
                    other => {
                        state.phase = other;
                        owner.tracker.broken.store(true, Ordering::Release);
                        None
                    },
                }
            },
            _ => {
                owner.tracker.broken.store(true, Ordering::Release);
                None
            },
        };
        let Some(retired) = retired else {
            drop(work);
            owner.tracker.changed.notify_waiters();
            return;
        };

        let fallback = work.fallback.take();
        let retired_ok = drop_arc_safely(retired);
        let worker_ok = drop_arc_safely(model);
        let fallback_ok = match fallback {
            Some(fallback) => drop_arc_safely(fallback),
            None => true,
        };
        let released = retired_ok && worker_ok && fallback_ok;

        if let Ok(mut state) = lock(&owner) {
            if state.generation == generation && matches!(&state.phase, Phase::Releasing) {
                let _previous = state.work.take();
                if released {
                    state.phase = if state.closing { Phase::Closed } else { Phase::Cold };
                    state.eviction_succeeded = true;
                    state.eviction_requested = false;
                } else {
                    state.phase = Phase::Failed { model: None, error: LoadError::TaskFailed, retry_load: false };
                    state.eviction_succeeded = false;
                }
                work.finish();
            } else {
                owner.tracker.broken.store(true, Ordering::Release);
            }
        } else {
            owner.tracker.broken.store(true, Ordering::Release);
        }
        drop(work);
        owner.tracker.changed.notify_waiters();
    }));
    Ok(())
}

fn drop_arc_safely<M>(model: Arc<M>) -> bool {
    catch_unwind(AssertUnwindSafe(|| drop(model))).is_ok()
}
