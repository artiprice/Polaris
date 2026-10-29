//! Startup ordering and cleanup through the real supervisor task owner.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_kernel_bus::{Bus, BusConfig};
use polaris_kernel_supervisor::{
    BackoffSpec, ChildName, HealthPolicy, HealthProbe, RestartPolicy, Supervisor, SupervisorConfig,
    TaskError, TaskSpec, WorkerLifecycle, WorkerSpec,
};
use polaris_sdk::contracts::EventRecord;
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_sdk::registry::{
    EventStore, EventStoreError, MemoryPressure, PlatformError, PlatformOps, ProcessExit,
    ProcessTree, SpawnSpec, StopOutcome,
};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct Store(Mutex<Vec<EventRecord>>);

impl EventStore for Store {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        let mut records = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("lock".into()))?;
        let next = u64::try_from(records.len())
            .map_err(|_| EventStoreError::Unavailable("length".into()))?
            .checked_add(1)
            .ok_or_else(|| EventStoreError::Unavailable("overflow".into()))?;
        let seq = Seq::new(next);
        records.push(record.with_seq(seq));
        Ok(seq)
    }

    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("lock".into()))?
            .iter()
            .filter(|record| record.seq() >= from && record.seq() <= to)
            .copied()
            .collect())
    }
}

#[derive(Default)]
struct Platform {
    next: AtomicU32,
    live: Mutex<BTreeSet<u64>>,
    fail_first: AtomicBool,
    fail_wait: AtomicBool,
    fail_stop: AtomicBool,
    pending_stop: AtomicBool,
    stop_started: AtomicBool,
    kills: AtomicUsize,
}

impl Platform {
    fn remove(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
        let mut live = self
            .live
            .lock()
            .map_err(|_| PlatformError::Io("lock".into()))?;
        if !live.remove(&tree.handle_id) {
            return Err(PlatformError::NotFound("tree".into()));
        }
        Ok(())
    }

    fn live_count(&self) -> usize {
        match self.live.lock() {
            Ok(live) => live.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }
}

impl PlatformOps for Platform {
    fn spawn_tree(&self, _: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
        let root_pid = self
            .next
            .fetch_add(1, Ordering::SeqCst)
            .checked_add(1)
            .ok_or_else(|| PlatformError::Io("id".into()))?;
        let handle_id = u64::from(root_pid);
        self.live
            .lock()
            .map_err(|_| PlatformError::Io("lock".into()))?
            .insert(handle_id);
        Ok(ProcessTree {
            root_pid,
            handle_id,
        })
    }

    async fn graceful_stop(
        &self,
        tree: &ProcessTree,
        _: Duration,
    ) -> Result<StopOutcome, PlatformError> {
        self.stop_started.store(true, Ordering::SeqCst);
        if self.pending_stop.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        if self.fail_stop.load(Ordering::SeqCst) {
            return Err(PlatformError::Io("stop failed".into()));
        }
        self.remove(tree)?;
        Ok(StopOutcome::Exited { code: Some(0) })
    }

    fn kill_tree(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.remove(tree)
    }

    async fn try_wait_tree(
        &self,
        tree: &ProcessTree,
    ) -> Result<Option<ProcessExit>, PlatformError> {
        if self.fail_wait.load(Ordering::SeqCst) {
            return Err(PlatformError::Io("wait failed".into()));
        }
        if tree.handle_id == 1 && self.fail_first.swap(false, Ordering::SeqCst) {
            self.remove(tree)?;
            return Ok(Some(ProcessExit { code: Some(17) }));
        }
        Ok(None)
    }

    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
        Ok(MemoryPressure::Normal)
    }
}

#[derive(Clone, Copy)]
enum InitMode {
    Ready,
    Fail,
    Pending,
}

struct Hooks {
    mode: InitMode,
    initialized: Mutex<BTreeSet<u64>>,
    starts: AtomicUsize,
    probes: AtomicUsize,
    retired: AtomicUsize,
}

impl Hooks {
    fn new(mode: InitMode) -> Self {
        Self {
            mode,
            initialized: Mutex::new(BTreeSet::new()),
            starts: AtomicUsize::new(0),
            probes: AtomicUsize::new(0),
            retired: AtomicUsize::new(0),
        }
    }
}

impl HealthProbe for Hooks {
    async fn check(&self, tree: ProcessTree) -> Result<(), TaskError> {
        if !self
            .initialized
            .lock()
            .map_err(|_| TaskError::fatal("lock"))?
            .contains(&tree.handle_id)
        {
            return Err(TaskError::fatal("health ran before startup"));
        }
        self.probes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl WorkerLifecycle for Hooks {
    async fn initialize(&self, tree: ProcessTree) -> Result<(), TaskError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            InitMode::Ready => {
                self.initialized
                    .lock()
                    .map_err(|_| TaskError::fatal("lock"))?
                    .insert(tree.handle_id);
                Ok(())
            }
            InitMode::Fail => Err(TaskError::fatal("bootstrap failed")),
            InitMode::Pending => std::future::pending().await,
        }
    }

    fn retire(&self, tree: ProcessTree) {
        self.retired.fetch_add(1, Ordering::SeqCst);
        match self.initialized.lock() {
            Ok(mut initialized) => {
                initialized.remove(&tree.handle_id);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(&tree.handle_id);
            }
        }
    }
}

fn fixture() -> Result<Supervisor<Arc<Store>>, Box<dyn std::error::Error>> {
    let (bus, _commands) = Bus::new(BusConfig::default(), Arc::new(Store::default()))?;
    Ok(Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(41),
        CancellationToken::new(),
        SupervisorConfig::default(),
    ))
}

fn spec() -> WorkerSpec {
    let mut task = TaskSpec::new(ChildName::new(
        "initialized-worker",
        Cid::from_bytes([41; 32]),
    ));
    task.policy = RestartPolicy::Temporary;
    task.backoff = BackoffSpec {
        min_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
        jitter: false,
    };
    WorkerSpec {
        task,
        process: SpawnSpec::default(),
        poll_interval: Duration::from_millis(1),
        stop_grace: Duration::from_millis(10),
    }
}

fn health() -> HealthPolicy {
    HealthPolicy {
        interval: Duration::from_millis(1),
        timeout: Duration::from_millis(100),
    }
}

async fn until(mut condition: impl FnMut() -> bool) -> TestResult {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn bootstrap_precedes_health_and_restarts() -> TestResult {
    let sup = fixture()?;
    let platform = Arc::new(Platform::default());
    platform.fail_first.store(true, Ordering::SeqCst);
    let hooks = Arc::new(Hooks::new(InitMode::Ready));
    let mut worker = spec();
    worker.task.policy = RestartPolicy::Permanent;
    sup.spawn_worker_with_lifecycle(
        Arc::clone(&platform),
        worker,
        Arc::clone(&hooks),
        health(),
        Duration::from_secs(1),
    )?;
    until(|| hooks.starts.load(Ordering::SeqCst) >= 2 && hooks.probes.load(Ordering::SeqCst) > 0)
        .await?;
    let _ = sup.shutdown(Duration::from_secs(1)).await;
    assert_eq!(platform.live_count(), 0);
    assert_eq!(
        hooks.starts.load(Ordering::SeqCst),
        hooks.retired.load(Ordering::SeqCst)
    );
    Ok(())
}

#[tokio::test]
async fn failed_bootstrap_is_retired_and_never_probed() -> TestResult {
    let sup = fixture()?;
    let platform = Arc::new(Platform::default());
    let hooks = Arc::new(Hooks::new(InitMode::Fail));
    sup.spawn_worker_with_lifecycle(
        Arc::clone(&platform),
        spec(),
        Arc::clone(&hooks),
        health(),
        Duration::from_secs(1),
    )?;
    until(|| hooks.retired.load(Ordering::SeqCst) == 1).await?;
    let _ = sup.shutdown(Duration::from_secs(1)).await;
    assert_eq!(platform.live_count(), 0);
    assert_eq!(hooks.probes.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn unconfirmed_cleanup_never_starts_another_generation() -> TestResult {
    let sup = fixture()?;
    let platform = Arc::new(Platform::default());
    platform.fail_wait.store(true, Ordering::SeqCst);
    platform.fail_stop.store(true, Ordering::SeqCst);
    let hooks = Arc::new(Hooks::new(InitMode::Ready));
    let mut worker = spec();
    worker.task.policy = RestartPolicy::Permanent;
    sup.spawn_worker_with_lifecycle(
        Arc::clone(&platform),
        worker,
        Arc::clone(&hooks),
        health(),
        Duration::from_secs(1),
    )?;
    until(|| {
        sup.snapshot()
            .children
            .iter()
            .any(|child| child.status == polaris_kernel_supervisor::ChildStatus::GaveUp)
    })
    .await?;
    let _ = sup.shutdown(Duration::from_secs(1)).await;
    assert_eq!(
        hooks.starts.load(Ordering::SeqCst),
        1,
        "uncertain termination must prohibit automatic replacement"
    );
    assert_eq!(platform.live_count(), 0);
    Ok(())
}

#[tokio::test]
async fn startup_deadline_and_cancellation_retain_process_ownership() -> TestResult {
    for startup_timeout in [Duration::from_millis(5), Duration::from_secs(1)] {
        let sup = fixture()?;
        let platform = Arc::new(Platform::default());
        let hooks = Arc::new(Hooks::new(InitMode::Pending));
        sup.spawn_worker_with_lifecycle(
            Arc::clone(&platform),
            spec(),
            Arc::clone(&hooks),
            health(),
            startup_timeout,
        )?;
        until(|| hooks.starts.load(Ordering::SeqCst) == 1).await?;
        if startup_timeout < Duration::from_secs(1) {
            until(|| hooks.retired.load(Ordering::SeqCst) == 1).await?;
        }
        let _ = sup.shutdown(Duration::from_millis(100)).await;
        assert_eq!(platform.live_count(), 0);
        assert_eq!(hooks.retired.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.probes.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn pending_stop_is_bounded_and_retires_before_supervisor_abort() -> TestResult {
    let sup = fixture()?;
    let platform = Arc::new(Platform::default());
    platform.pending_stop.store(true, Ordering::SeqCst);
    let hooks = Arc::new(Hooks::new(InitMode::Ready));
    sup.spawn_worker_with_lifecycle(
        Arc::clone(&platform),
        spec(),
        Arc::clone(&hooks),
        health(),
        Duration::from_secs(1),
    )?;
    until(|| hooks.probes.load(Ordering::SeqCst) > 0).await?;
    let hooks_at_shutdown = Arc::clone(&hooks);
    let observed = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        hooks_at_shutdown.retired.load(Ordering::SeqCst)
    });
    let _ = sup.shutdown(Duration::from_secs(2)).await;
    assert_eq!(
        observed.await?,
        1,
        "generation must retire within its own cleanup bound before supervisor abort"
    );
    assert_eq!(platform.live_count(), 0);
    assert_eq!(platform.kills.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn retirement_precedes_slow_stop() -> TestResult {
    let sup = fixture()?;
    let platform = Arc::new(Platform::default());
    platform.pending_stop.store(true, Ordering::SeqCst);
    let hooks = Arc::new(Hooks::new(InitMode::Ready));
    sup.spawn_worker_with_lifecycle(
        Arc::clone(&platform),
        spec(),
        Arc::clone(&hooks),
        health(),
        Duration::from_secs(1),
    )?;
    until(|| hooks.probes.load(Ordering::SeqCst) > 0).await?;
    let shutdown = tokio::spawn(async move { sup.shutdown(Duration::from_secs(2)).await });
    until(|| platform.stop_started.load(Ordering::SeqCst)).await?;
    let retired_when_stop_started = hooks.retired.load(Ordering::SeqCst);
    let _ = shutdown.await?;
    assert_eq!(
        retired_when_stop_started, 1,
        "stopping generation must be retired before platform cleanup starts"
    );
    assert_eq!(platform.live_count(), 0);
    assert_eq!(hooks.retired.load(Ordering::SeqCst), 1);
    Ok(())
}
