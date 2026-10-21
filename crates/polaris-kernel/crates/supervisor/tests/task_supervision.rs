//! Task Supervision (descriptor: specs/descriptors/supervisor_task_supervision.json).

use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_kernel_bus::{Bus, BusConfig};
use polaris_kernel_supervisor::{
    BackoffSpec, ChildName, ChildStatus, ExitClass, OnExhausted, RestartIntensity, RestartPolicy,
    StopOutcome, Supervisor, SupervisorConfig, SupervisorError, TaskError, TaskSpec,
};
use polaris_sdk::contracts::{Event, EventRecord, WorkerKind};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};
use polaris_telemetry::Telemetry;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;
type Sup = Supervisor<Arc<MemStore>>;

const SESSION: u128 = 11;

/// In-memory store with gap-free sequence numbers and an injectable failure.
#[derive(Default)]
struct MemStore {
    records: Mutex<Vec<EventRecord>>,
    fail: AtomicBool,
}

impl MemStore {
    fn records(&self) -> Vec<EventRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn events(&self) -> Vec<Event> {
        self.records().iter().map(|rec| *rec.event()).collect()
    }
}

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(EventStoreError::Unavailable("injected failure".into()));
        }
        let mut records = self
            .records
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        let seq = Seq::new(
            u64::try_from(records.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        );
        records.push(record.with_seq(seq));
        Ok(seq)
    }
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        let records = self
            .records
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        Ok(records
            .iter()
            .filter(|r| r.seq() >= from && r.seq() <= to)
            .copied()
            .collect())
    }
}

/// Counts live actor futures: incremented on creation, decremented when the future is dropped.
struct Live(Arc<AtomicUsize>);

impl Live {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(counter))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn name(label: &'static str, byte: u8) -> ChildName {
    ChildName::new(label, Cid::from_bytes([byte; 32]))
}

fn fixture_with(root: CancellationToken) -> Result<(Sup, Arc<MemStore>), Box<dyn Error>> {
    let store = Arc::new(MemStore::default());
    let (bus, _commands) = Bus::new(BusConfig::default(), Arc::clone(&store))?;
    let sup = Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(SESSION),
        root,
        SupervisorConfig::default(),
    );
    Ok((sup, store))
}

fn fixture() -> Result<(Sup, Arc<MemStore>), Box<dyn Error>> {
    fixture_with(CancellationToken::new())
}

/// 100 ms .. 1 s backoff without jitter, `max_restarts` per hour.
fn spec(label: &'static str, byte: u8, max_restarts: u32) -> TaskSpec {
    TaskSpec {
        intensity: RestartIntensity {
            max_restarts,
            window: Duration::from_secs(3_600),
        },
        backoff: BackoffSpec {
            min_delay: ms(100),
            max_delay: ms(1_000),
            jitter: false,
        },
        ..TaskSpec::new(name(label, byte))
    }
}

/// Panics, as a faulty actor would.
fn boom() -> Result<(), TaskError> {
    panic!("deliberate test panic")
}

async fn next<T>(rx: &mut mpsc::Receiver<T>) -> Result<T, Box<dyn Error>> {
    tokio::time::timeout(Duration::from_secs(600), rx.recv())
        .await?
        .ok_or_else(|| "channel closed".into())
}

#[tokio::test(start_paused = true)]
async fn panicking_task_is_restarted() -> TestResult {
    let (sup, _store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let (tx, mut rx) = mpsc::channel::<u32>(8);
    let id = sup.spawn_task(spec("flaky", 1, 3), {
        let runs = Arc::clone(&runs);
        move |token| {
            let runs = Arc::clone(&runs);
            let tx = tx.clone();
            async move {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = tx.send(n).await;
                if n == 1 {
                    return boom();
                }
                token.cancelled().await;
                Ok(())
            }
        }
    })?;
    assert_eq!(next(&mut rx).await?, 1);
    assert_eq!(next(&mut rx).await?, 2, "the panicked task ran again");
    let snap = sup.snapshot();
    let child = snap.children.first().ok_or("child missing")?;
    assert_eq!(child.restarts, 1);
    assert_eq!(child.status, ChildStatus::Running);
    assert_eq!(
        sup.stop(id, Duration::from_secs(1)).await?,
        StopOutcome::Graceful
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn restart_is_published_as_worker_restarted() -> TestResult {
    let (sup, store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let (tx, mut rx) = mpsc::channel::<u32>(8);
    let task_spec = TaskSpec {
        worker: Some(WorkerKind::Ingest),
        ..spec("ingest-proxy", 2, 5)
    };
    let id = sup.spawn_task(task_spec, {
        let runs = Arc::clone(&runs);
        move |token| {
            let runs = Arc::clone(&runs);
            let tx = tx.clone();
            async move {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                // The restart event must already be durable when the next run starts.
                let _ = tx.send(n).await;
                if n <= 2 {
                    return boom();
                }
                token.cancelled().await;
                Ok(())
            }
        }
    })?;
    assert_eq!(next(&mut rx).await?, 1);
    assert!(store.events().is_empty());
    assert_eq!(next(&mut rx).await?, 2);
    assert_eq!(
        store.events().first(),
        Some(&Event::WorkerRestarted {
            worker: WorkerKind::Ingest,
            restarts: 1
        })
    );
    assert_eq!(next(&mut rx).await?, 3);
    assert_eq!(
        store.events(),
        vec![
            Event::WorkerRestarted {
                worker: WorkerKind::Ingest,
                restarts: 1
            },
            Event::WorkerRestarted {
                worker: WorkerKind::Ingest,
                restarts: 2
            },
        ]
    );
    let records = store.records();
    assert!(
        records
            .iter()
            .all(|r| r.session() == SessionId::from_u128(SESSION))
    );
    sup.stop(id, Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn backoff_grows_between_restarts() -> TestResult {
    let (sup, _store) = fixture()?;
    let (tx, mut rx) = mpsc::channel::<Instant>(16);
    let id = sup.spawn_task(spec("grower", 3, 10), move |_token| {
        let tx = tx.clone();
        async move {
            let _ = tx.send(Instant::now()).await;
            Err(TaskError::transient("keep failing"))
        }
    })?;
    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(next(&mut rx).await?);
    }
    let gaps: Vec<Duration> = times
        .iter()
        .zip(times.iter().skip(1))
        .map(|(earlier, later)| *later - *earlier)
        .collect();
    // min 100 ms, factor 2, max 1 s: 100, 200, 400, 800.
    let expected = [ms(100), ms(200), ms(400), ms(800)];
    assert_eq!(
        gaps.len(),
        expected.len(),
        "every restart gap must be measured"
    );
    for (actual, nominal) in gaps.iter().zip(expected) {
        // User-approved contract correction: Tokio's timer wheel rounds deadlines to ms.
        // Early retries and drift beyond one timer tick still fail this check.
        assert!(
            *actual >= nominal && *actual <= nominal.saturating_add(ms(1)),
            "expected {nominal:?}..={:?}, got {actual:?}",
            nominal.saturating_add(ms(1)),
        );
    }
    sup.stop(id, Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn intensity_exceeded_routes_to_dlq() -> TestResult {
    let (sup, store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let child = name("always-fails", 4);
    let task_spec = TaskSpec {
        worker: Some(WorkerKind::S1),
        ..spec("always-fails", 4, 2)
    };
    let id = sup.spawn_task(task_spec, {
        let runs = Arc::clone(&runs);
        move |_token| {
            let runs = Arc::clone(&runs);
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                Err(TaskError::transient("always"))
            }
        }
    })?;
    let outcome = sup.wait(id).await;
    assert!(
        matches!(&outcome, Err(SupervisorError::IntensityExceeded { child: c }) if *c == child),
        "got {outcome:?}"
    );
    assert_eq!(runs.load(Ordering::SeqCst), 3, "initial run + 2 restarts");
    assert_eq!(
        store.events(),
        vec![
            Event::WorkerRestarted {
                worker: WorkerKind::S1,
                restarts: 1
            },
            Event::WorkerRestarted {
                worker: WorkerKind::S1,
                restarts: 2
            },
            Event::CircuitTripped {
                breaker: child.cid()
            },
            Event::DeadLettered {
                message: child.cid()
            },
        ]
    );
    let snap = sup.snapshot();
    assert_eq!(snap.dlq_len, 1);
    let entry = snap.children.first().ok_or("child missing")?;
    assert_eq!(entry.status, ChildStatus::GaveUp);
    assert_eq!(entry.restarts, 2);
    let letters = sup.drain_dead_letters();
    let letter = letters.first().ok_or("no dead letter")?;
    assert_eq!(letter.item, child.cid());
    assert_eq!(letter.child, child);
    assert_eq!(letter.reason, ExitClass::Failed);
    assert_eq!(letter.attempts, 3);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn no_restart_storm_after_give_up() -> TestResult {
    let (sup, store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let id = sup.spawn_task(spec("storm", 5, 3), {
        let runs = Arc::clone(&runs);
        move |_token| {
            let runs = Arc::clone(&runs);
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                boom()
            }
        }
    })?;
    assert!(sup.wait(id).await.is_err());
    let after_give_up = runs.load(Ordering::SeqCst);
    assert_eq!(after_give_up, 4);
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    assert_eq!(runs.load(Ordering::SeqCst), after_give_up);
    // The trip is announced once, not once per second.
    let trips = store
        .events()
        .iter()
        .filter(|e| matches!(e, Event::CircuitTripped { .. }))
        .count();
    assert_eq!(trips, 1);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn temporary_child_not_restarted() -> TestResult {
    let (sup, store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let task_spec = TaskSpec {
        policy: RestartPolicy::Temporary,
        ..spec("one-shot", 6, 5)
    };
    let id = sup.spawn_task(task_spec, {
        let runs = Arc::clone(&runs);
        move |_token| {
            let runs = Arc::clone(&runs);
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                Err(TaskError::transient("boom"))
            }
        }
    })?;
    assert_eq!(sup.wait(id).await?, ChildStatus::Exited(ExitClass::Failed));
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(
        store.events().is_empty(),
        "a Temporary child is not escalated"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn transient_child_stays_down_after_clean_exit() -> TestResult {
    let (sup, _store) = fixture()?;
    let transient_runs = Arc::new(AtomicU32::new(0));
    let permanent_runs = Arc::new(AtomicU32::new(0));
    let transient = sup.spawn_task(
        TaskSpec {
            policy: RestartPolicy::Transient,
            ..spec("transient", 7, 5)
        },
        {
            let runs = Arc::clone(&transient_runs);
            move |_token| {
                let runs = Arc::clone(&runs);
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        },
    )?;
    let (tx, mut rx) = mpsc::channel::<u32>(8);
    let permanent = sup.spawn_task(spec("permanent", 8, 5), {
        let runs = Arc::clone(&permanent_runs);
        move |token| {
            let runs = Arc::clone(&runs);
            let tx = tx.clone();
            async move {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = tx.send(n).await;
                if n == 1 {
                    return Ok(());
                }
                token.cancelled().await;
                Ok(())
            }
        }
    })?;
    assert_eq!(
        sup.wait(transient).await?,
        ChildStatus::Exited(ExitClass::Clean)
    );
    assert_eq!(next(&mut rx).await?, 1);
    assert_eq!(
        next(&mut rx).await?,
        2,
        "a Permanent child restarts after Ok"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(transient_runs.load(Ordering::SeqCst), 1);
    assert_eq!(permanent_runs.load(Ordering::SeqCst), 2);
    sup.stop(permanent, Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn fatal_error_gives_up_immediately() -> TestResult {
    let (sup, store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let child = name("corrupt", 9);
    let id = sup.spawn_task(spec("corrupt", 9, 10), {
        let runs = Arc::clone(&runs);
        move |_token| {
            let runs = Arc::clone(&runs);
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                Err(TaskError::fatal("integrity in doubt"))
            }
        }
    })?;
    assert!(matches!(
        sup.wait(id).await,
        Err(SupervisorError::IntensityExceeded { .. })
    ));
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.events(),
        vec![
            Event::CircuitTripped {
                breaker: child.cid()
            },
            Event::DeadLettered {
                message: child.cid()
            },
        ]
    );
    let letters = sup.drain_dead_letters();
    assert_eq!(letters.first().map(|l| l.attempts), Some(1));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn exhausted_escalation_cancels_root() -> TestResult {
    let root = CancellationToken::new();
    let (sup, _store) = fixture_with(root.clone())?;
    let sibling_stopped = Arc::new(AtomicBool::new(false));
    let sibling = sup.spawn_task(spec("sibling", 10, 3), {
        let flag = Arc::clone(&sibling_stopped);
        move |token| {
            let flag = Arc::clone(&flag);
            async move {
                token.cancelled().await;
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
    })?;
    let critical = sup.spawn_task(
        TaskSpec {
            on_exhausted: OnExhausted::ShutdownSupervisor,
            ..spec("critical", 11, 0)
        },
        |_token| async { Err(TaskError::transient("down")) },
    )?;
    assert!(sup.wait(critical).await.is_err());
    assert!(
        root.is_cancelled(),
        "exhaustion escalated to the root token"
    );
    assert_eq!(sup.wait(sibling).await?, ChildStatus::Stopped);
    assert!(sibling_stopped.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn cancel_token_stops_child() -> TestResult {
    let (sup, _store) = fixture()?;
    let runs = Arc::new(AtomicU32::new(0));
    let (tx, mut rx) = mpsc::channel::<u32>(4);
    let id = sup.spawn_task(spec("cooperative", 12, 5), {
        let runs = Arc::clone(&runs);
        move |token| {
            let runs = Arc::clone(&runs);
            let tx = tx.clone();
            async move {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = tx.send(n).await;
                token.cancelled().await;
                Ok(())
            }
        }
    })?;
    assert_eq!(next(&mut rx).await?, 1);
    let started = Instant::now();
    assert_eq!(
        sup.stop(id, Duration::from_secs(30)).await?,
        StopOutcome::Graceful
    );
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_eq!(sup.wait(id).await?, ChildStatus::Stopped);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "a stopped child is not restarted"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stop_aborts_child_that_ignores_cancel() -> TestResult {
    let (sup, _store) = fixture()?;
    let live = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = mpsc::channel::<()>(1);
    let id = sup.spawn_task(spec("stubborn", 13, 5), {
        let live = Arc::clone(&live);
        move |_token| {
            let live = Arc::clone(&live);
            let tx = tx.clone();
            async move {
                let _guard = Live::new(&live);
                let _ = tx.send(()).await;
                std::future::pending::<Result<(), TaskError>>().await
            }
        }
    })?;
    next(&mut rx).await?;
    assert_eq!(live.load(Ordering::SeqCst), 1);
    let started = Instant::now();
    assert_eq!(sup.stop(id, ms(1_500)).await?, StopOutcome::Aborted);
    assert!(started.elapsed() >= ms(1_500), "grace period was honoured");
    assert_eq!(live.load(Ordering::SeqCst), 0, "the future was dropped");
    assert_eq!(sup.wait(id).await?, ChildStatus::Stopped);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn unknown_child_is_error() -> TestResult {
    let (owner, _s1) = fixture()?;
    let (other, _s2) = fixture()?;
    let id = owner.spawn_task(spec("idle", 14, 1), |token| async move {
        token.cancelled().await;
        Ok(())
    })?;
    assert!(matches!(
        other.stop(id, ms(10)).await,
        Err(SupervisorError::UnknownChild)
    ));
    assert!(matches!(
        other.wait(id).await,
        Err(SupervisorError::UnknownChild)
    ));
    owner.stop(id, ms(10)).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn panic_does_not_poison_supervisor() -> TestResult {
    let (sup, _store) = fixture()?;
    let ticks = Arc::new(AtomicU32::new(0));
    let healthy = sup.spawn_task(spec("healthy", 15, 3), {
        let ticks = Arc::clone(&ticks);
        move |token| {
            let ticks = Arc::clone(&ticks);
            async move {
                loop {
                    tokio::select! {
                        () = token.cancelled() => return Ok(()),
                        () = tokio::time::sleep(ms(10)) => {
                            ticks.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
    })?;
    let doomed = sup.spawn_task(
        TaskSpec {
            policy: RestartPolicy::Temporary,
            ..spec("doomed", 16, 3)
        },
        |_token| async { boom() },
    )?;
    assert_eq!(
        sup.wait(doomed).await?,
        ChildStatus::Exited(ExitClass::Panicked)
    );
    let before = ticks.load(Ordering::SeqCst);
    tokio::time::sleep(ms(100)).await;
    assert!(
        ticks.load(Ordering::SeqCst) > before,
        "sibling kept running"
    );
    // The supervisor itself is fully usable afterwards.
    let late = sup.spawn_task(spec("late", 17, 3), |token| async move {
        token.cancelled().await;
        Ok(())
    })?;
    assert_eq!(sup.snapshot().children.len(), 3);
    assert_eq!(sup.stop(late, ms(10)).await?, StopOutcome::Graceful);
    assert_eq!(sup.stop(healthy, ms(10)).await?, StopOutcome::Graceful);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_all_children_without_leaks() -> TestResult {
    let (sup, _store) = fixture()?;
    let live = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = mpsc::channel::<()>(16);
    for i in 0..5_u8 {
        sup.spawn_task(spec("cooperative", 20 + i, 3), {
            let live = Arc::clone(&live);
            let tx = tx.clone();
            move |token| {
                let live = Arc::clone(&live);
                let tx = tx.clone();
                async move {
                    let _guard = Live::new(&live);
                    let _ = tx.send(()).await;
                    token.cancelled().await;
                    Ok(())
                }
            }
        })?;
    }
    sup.spawn_task(spec("stubborn", 30, 3), {
        let live = Arc::clone(&live);
        let tx = tx.clone();
        move |_token| {
            let live = Arc::clone(&live);
            let tx = tx.clone();
            async move {
                let _guard = Live::new(&live);
                let _ = tx.send(()).await;
                std::future::pending::<Result<(), TaskError>>().await
            }
        }
    })?;
    for _ in 0..6 {
        next(&mut rx).await?;
    }
    assert_eq!(live.load(Ordering::SeqCst), 6);
    assert_eq!(sup.shutdown(ms(500)).await, StopOutcome::Aborted);
    assert_eq!(
        live.load(Ordering::SeqCst),
        0,
        "every actor future was dropped"
    );
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    let alive = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    assert_eq!(alive, 0, "no supervised task outlived shutdown");
    let snap = sup.snapshot();
    assert_eq!(snap.children.len(), 6);
    assert!(
        snap.children
            .iter()
            .all(|c| c.status == ChildStatus::Stopped)
    );
    let late = sup.spawn_task(spec("late", 31, 3), |_token| async { Ok(()) });
    assert!(matches!(late, Err(SupervisorError::ShuttingDown)));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn publish_failure_does_not_stop_supervision() -> TestResult {
    let (sup, store) = fixture()?;
    store.fail.store(true, Ordering::SeqCst);
    let (tx, mut rx) = mpsc::channel::<u32>(8);
    let runs = Arc::new(AtomicU32::new(0));
    let task_spec = TaskSpec {
        worker: Some(WorkerKind::Nano),
        ..spec("nano-proxy", 40, 5)
    };
    let id = sup.spawn_task(task_spec, {
        let runs = Arc::clone(&runs);
        move |token| {
            let runs = Arc::clone(&runs);
            let tx = tx.clone();
            async move {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = tx.send(n).await;
                if n == 1 {
                    return Err(TaskError::transient("once"));
                }
                token.cancelled().await;
                Ok(())
            }
        }
    })?;
    assert_eq!(next(&mut rx).await?, 1);
    assert_eq!(
        next(&mut rx).await?,
        2,
        "restarted although the store refused the event"
    );
    assert!(store.events().is_empty());
    assert_eq!(sup.snapshot().events_lost, 1);
    sup.stop(id, ms(10)).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn telemetry_folds_supervisor_events() -> TestResult {
    let (sup, store) = fixture()?;
    let task_spec = TaskSpec {
        worker: Some(WorkerKind::S1),
        ..spec("always-fails", 50, 2)
    };
    let id = sup.spawn_task(task_spec, |_token| async {
        Err(TaskError::transient("always"))
    })?;
    assert!(sup.wait(id).await.is_err());
    let mut telemetry = Telemetry::new(SessionId::from_u128(SESSION))?;
    for record in store.records() {
        telemetry.record_record(&record)?;
    }
    // snapshot() is deliberately the last publication, not the live aggregate.
    // Preserve the telemetry API's explicit publication boundary.
    telemetry.publish(polaris_sdk::contracts::Timestamp::now());
    let outcomes = telemetry.snapshot().outcomes;
    assert_eq!(outcomes.worker_restarts, 2);
    assert_eq!(outcomes.circuits_tripped, 1);
    assert_eq!(outcomes.dead_letters, 1);
    Ok(())
}
