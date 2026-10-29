//! Native P1 lifecycle workloads; sandbox cleanup below is explicitly a test double.
use polaris_kernel::{
    bus::{Bus, BusConfig},
    reaper::{
        Disposition, Outcome, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
        TeardownError,
    },
    storage::eventlog::{Durability, EventLog, EventLogConfig},
    supervisor::{
        BackoffSpec, ChildName, HealthPolicy, HealthProbe, RestartIntensity, Supervisor,
        SupervisorConfig, TaskError, TaskSpec, WorkerSpec,
    },
};
use polaris_platform::process::ProcessPlatform;
use polaris_sdk::{
    contracts::WorkerKind,
    ids::{Cid, SessionId},
    registry::{
        MemoryPressure, PlatformError, PlatformOps, ProcessExit, ProcessTree, SpawnSpec,
        StopOutcome,
    },
};
use std::{
    ffi::OsString,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct ObservedPlatform {
    native: ProcessPlatform,
    roots: Mutex<Vec<ProcessTree>>,
}
impl PlatformOps for ObservedPlatform {
    fn spawn_tree(&self, spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
        let tree = self.native.spawn_tree(spec)?;
        self.roots
            .lock()
            .map_err(|_| PlatformError::Io("fixture lock poisoned".into()))?
            .push(tree);
        Ok(tree)
    }
    async fn graceful_stop(
        &self,
        tree: &ProcessTree,
        grace: Duration,
    ) -> Result<StopOutcome, PlatformError> {
        self.native.graceful_stop(tree, grace).await
    }
    async fn try_wait_tree(
        &self,
        tree: &ProcessTree,
    ) -> Result<Option<ProcessExit>, PlatformError> {
        self.native.try_wait_tree(tree).await
    }
    fn kill_tree(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
        self.native.kill_tree(tree)
    }
    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
        self.native.memory_pressure()
    }
}

fn sleeper_spec() -> Result<SpawnSpec, std::io::Error> {
    Ok(SpawnSpec {
        program: std::env::current_exe()?,
        args: vec![
            OsString::from("--exact"),
            OsString::from("native_sleeper_fixture"),
        ],
        env: vec![(
            OsString::from("POLARIS_LIFECYCLE_FIXTURE"),
            OsString::from("sleep"),
        )],
        ..SpawnSpec::default()
    })
}

#[test]
fn native_sleeper_fixture() {
    if std::env::var_os("POLARIS_LIFECYCLE_FIXTURE").as_deref()
        == Some(std::ffi::OsStr::new("sleep"))
    {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn assert_roots_absent(roots: &[ProcessTree]) -> TestResult {
    let pids: Vec<_> = roots
        .iter()
        .map(|tree| Pid::from_u32(tree.root_pid))
        .collect();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
    for pid in pids {
        assert!(system.process(pid).is_none(), "owned root {pid} survived");
    }
    Ok(())
}

struct HangingProbe;
impl HealthProbe for HangingProbe {
    async fn check(&self, _tree: ProcessTree) -> Result<(), TaskError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn dropping_reaper_stops_process_even_when_native_adapter_is_shared() -> TestResult {
    let temp = tempfile::tempdir()?;
    let platform = Arc::new(ProcessPlatform::new());
    let tree = platform.spawn_tree(&sleeper_spec()?)?;
    let reaper = Reaper::new(&temp.path().join("run"))?;
    reaper.register_process(Arc::clone(&platform), tree)?;
    drop(reaper);
    // The adapter deliberately remains alive; only the resource guard can stop this process.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_roots_absent(&[tree])?;
    assert!(
        Reaper::new(&temp.path().join("run"))?
            .leak_report()
            .entries
            .len()
            == 1,
        "emergency Drop must not claim a journaled clean shutdown"
    );
    Ok(())
}

#[tokio::test]
async fn timed_out_health_probe_kills_owned_native_worker() -> TestResult {
    let temp = tempfile::tempdir()?;
    let store = Arc::new(EventLog::open(EventLogConfig::new(
        temp.path().join("events.redb"),
    ))?);
    let (bus, _commands) = Bus::new(BusConfig::default(), store)?;
    let supervisor = Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(1),
        CancellationToken::new(),
        SupervisorConfig::default(),
    );
    let platform = Arc::new(ObservedPlatform::default());
    let mut task = TaskSpec::new(ChildName::new("health-timeout", Cid::from_bytes([43; 32])));
    task.intensity.max_restarts = 0;
    let id = supervisor.spawn_worker_with_health(
        Arc::clone(&platform),
        WorkerSpec {
            task,
            process: sleeper_spec()?,
            poll_interval: Duration::from_millis(1),
            stop_grace: Duration::from_millis(10),
        },
        Arc::new(HangingProbe),
        HealthPolicy {
            interval: Duration::from_millis(1),
            timeout: Duration::from_millis(10),
        },
    )?;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), supervisor.wait(id))
            .await?
            .is_err()
    );
    supervisor.shutdown(Duration::from_secs(1)).await;
    let roots = platform
        .roots
        .lock()
        .map_err(|_| "fixture lock poisoned")?
        .clone();
    assert_eq!(roots.len(), 1);
    assert_roots_absent(&roots)?;
    assert_eq!(supervisor.snapshot().dlq_len, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "native 200-process crash/kill stress; run explicitly"]
async fn two_hundred_native_worker_kills_respect_restart_budget_and_leave_no_roots() -> TestResult {
    let temp = tempfile::tempdir()?;
    let mut config = EventLogConfig::new(temp.path().join("events.redb"));
    config.durability = Durability::Eventual;
    let store = Arc::new(EventLog::open(config)?);
    let (bus, _commands) = Bus::new(BusConfig::default(), Arc::clone(&store))?;
    let supervisor = Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(1),
        CancellationToken::new(),
        SupervisorConfig::default(),
    );
    let platform = Arc::new(ObservedPlatform::default());
    let mut task = TaskSpec::new(ChildName::new("kill-stress", Cid::from_bytes([42; 32])));
    task.intensity = RestartIntensity {
        max_restarts: 199,
        window: Duration::from_secs(120),
    };
    task.backoff = BackoffSpec {
        min_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
        jitter: false,
    };
    task.worker = Some(WorkerKind::S1);
    let id = supervisor.spawn_worker(
        Arc::clone(&platform),
        WorkerSpec {
            task,
            process: sleeper_spec()?,
            poll_interval: Duration::from_millis(1),
            stop_grace: Duration::from_millis(10),
        },
    )?;
    let started = Instant::now();
    for index in 0..200 {
        let tree = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let tree = platform
                    .roots
                    .lock()
                    .map_err(|_| "fixture lock poisoned")?
                    .get(index)
                    .copied();
                if let Some(tree) = tree {
                    return Ok::<_, Box<dyn std::error::Error>>(tree);
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await??;
        let pid = Pid::from_u32(tree.root_pid);
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let process = system
            .process(pid)
            .ok_or("owned worker disappeared before injected kill")?;
        assert!(process.kill(), "native kill was refused");
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(5), supervisor.wait(id))
            .await?
            .is_err()
    );
    let snapshot = supervisor.snapshot();
    assert_eq!(
        snapshot.children.first().ok_or("missing child")?.restarts,
        199
    );
    assert_eq!(snapshot.dlq_len, 1);
    supervisor.shutdown(Duration::from_secs(1)).await;
    let roots = platform
        .roots
        .lock()
        .map_err(|_| "fixture lock poisoned")?
        .clone();
    assert_eq!(roots.len(), 200);
    assert_roots_absent(&roots)?;
    eprintln!(
        "native worker kills=200 restarts=199 dlq=1 surviving_roots=0 elapsed={:?}",
        started.elapsed()
    );
    Ok(())
}

struct FakeSandbox(Arc<AtomicUsize>);
impl Teardown for FakeSandbox {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Disposition::Clean)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "1000-directory and 20-process durable teardown workload; run explicitly"]
async fn reaper_thousand_directories_fifty_fake_sandboxes_twenty_native_processes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let reaper = Reaper::new(&temp.path().join("run"))?;
    let mut paths = Vec::new();
    for _ in 0..1000 {
        paths.push(reaper.create_temp_dir(temp.path())?.1);
    }
    let fake_released = Arc::new(AtomicUsize::new(0));
    for _ in 0..50 {
        reaper.register(
            ResourceKind::Sandbox,
            FakeSandbox(Arc::clone(&fake_released)),
        )?;
    }
    let platform = Arc::new(ProcessPlatform::new());
    let mut roots = Vec::new();
    for _ in 0..20 {
        let tree = platform.spawn_tree(&sleeper_spec()?)?;
        reaper.register_process(Arc::clone(&platform), tree)?;
        roots.push(tree);
    }
    let started = Instant::now();
    let report = reaper
        .shutdown_all(&ShutdownConfig {
            graceful: Duration::from_millis(50),
            forced: Duration::from_millis(200),
            overall: Duration::from_secs(10),
            ..ShutdownConfig::default()
        })
        .await?;
    assert_eq!(report.entries.len(), 1070);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| matches!(entry.outcome, Outcome::Released | Outcome::Forced)),
        "{report:?}"
    );
    assert!(!report.deadline_exceeded);
    assert!(report.journal_error.is_none());
    assert!(reaper.leak_report().entries.is_empty());
    assert_eq!(fake_released.load(Ordering::Relaxed), 50);
    assert!(paths.iter().all(|path| !path.exists()));
    assert_roots_absent(&roots)?;
    eprintln!(
        "reaper dirs=1000 fake_sandboxes=50 native_processes=20 unresolved=0 elapsed={:?}",
        started.elapsed()
    );
    Ok(())
}
