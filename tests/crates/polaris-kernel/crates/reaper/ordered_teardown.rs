//! Ordered Teardown (descriptor: specs/descriptors/reaper_ordered_teardown.json).

use std::error::Error;
use std::sync::{Arc, Mutex};

use polaris_kernel_reaper::{
    Disposition, Outcome, ReapId, Reaper, ReaperError, ResourceKind, ShutdownConfig, Teardown,
    TeardownContext, TeardownError, TeardownReason,
};

type TestResult = Result<(), Box<dyn Error>>;
type Log = Arc<Mutex<Vec<(&'static str, TeardownReason)>>>;

/// Appends its label (and the reason it was given) to a shared log; optionally fails.
struct Recorder {
    label: &'static str,
    log: Log,
    fail: bool,
}

impl Teardown for Recorder {
    async fn graceful(&mut self, ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        if let Ok(mut log) = self.log.lock() {
            log.push((self.label, ctx.reason));
        }
        if self.fail {
            return Err(TeardownError::Other(format!("{} refused", self.label)));
        }
        Ok(Disposition::Clean)
    }
}

fn recorder(label: &'static str, log: &Log, fail: bool) -> Recorder {
    Recorder {
        label,
        log: Arc::clone(log),
        fail,
    }
}

fn labels(log: &Log) -> Result<Vec<&'static str>, Box<dyn Error>> {
    let guard = log.lock().map_err(|_| "log lock poisoned")?;
    Ok(guard.iter().map(|(label, _)| *label).collect())
}

#[tokio::test]
async fn teardown_follows_kind_priority_then_reverse_registration() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    let registrations = [
        ("temp_a", ResourceKind::TempDir),
        ("task_b", ResourceKind::Task),
        ("sandbox_c", ResourceKind::Sandbox),
        ("process_d", ResourceKind::Process),
        ("task_e", ResourceKind::Task),
        ("temp_f", ResourceKind::TempDir),
        ("sandbox_g", ResourceKind::Sandbox),
        ("ipc_h", ResourceKind::IpcSegment),
        ("pipe_i", ResourceKind::WorkerPipe),
    ];
    for (label, kind) in registrations {
        reaper.register(kind, recorder(label, &log, false))?;
    }

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    let expected = [
        "task_e",
        "task_b",
        "process_d",
        "pipe_i",
        "sandbox_g",
        "sandbox_c",
        "temp_f",
        "temp_a",
        "ipc_h",
    ];
    assert_eq!(labels(&log)?, expected);
    let kinds: Vec<ResourceKind> = report.entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            ResourceKind::Task,
            ResourceKind::Task,
            ResourceKind::Process,
            ResourceKind::WorkerPipe,
            ResourceKind::Sandbox,
            ResourceKind::Sandbox,
            ResourceKind::TempDir,
            ResourceKind::TempDir,
            ResourceKind::IpcSegment,
        ]
    );
    assert!(
        report
            .entries
            .iter()
            .all(|e| e.outcome == Outcome::Released)
    );
    assert!(!report.deadline_exceeded);
    Ok(())
}

#[tokio::test]
async fn report_lists_every_resource_once() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    let mut ids: Vec<ReapId> = Vec::new();
    for label in ["a", "b", "c", "d"] {
        ids.push(reaper.register(ResourceKind::Sandbox, recorder(label, &log, false))?);
    }

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert_eq!(report.entries.len(), ids.len());
    for id in &ids {
        let hits = report.entries.iter().filter(|e| e.id == *id).count();
        assert_eq!(hits, 1, "{id:?} must appear exactly once");
    }
    Ok(())
}

#[tokio::test]
async fn failing_resource_is_reported_and_the_rest_complete() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    let first = reaper.register(ResourceKind::Sandbox, recorder("first", &log, false))?;
    let broken = reaper.register(ResourceKind::Sandbox, recorder("broken", &log, true))?;
    let last = reaper.register(ResourceKind::Sandbox, recorder("last", &log, false))?;

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert_eq!(labels(&log)?, ["last", "broken", "first"]);
    assert_eq!(report.outcome(first), Some(&Outcome::Released));
    assert_eq!(report.outcome(last), Some(&Outcome::Released));
    assert_eq!(
        report.outcome(broken),
        Some(&Outcome::Failed("broken refused".to_owned()))
    );
    Ok(())
}

#[tokio::test]
async fn second_call_is_rejected_and_touches_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    reaper.register(ResourceKind::Sandbox, recorder("only", &log, false))?;

    reaper.shutdown_all(&ShutdownConfig::default()).await?;
    let second = reaper.shutdown_all(&ShutdownConfig::default()).await;

    assert!(matches!(second, Err(ReaperError::AlreadyShutDown)));
    assert_eq!(labels(&log)?, ["only"]);
    Ok(())
}

#[tokio::test]
async fn register_after_shutdown_is_rejected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    reaper.shutdown_all(&ShutdownConfig::default()).await?;

    let late = reaper.register(ResourceKind::Sandbox, recorder("late", &log, false));

    assert!(matches!(late, Err(ReaperError::AlreadyShutDown)));
    assert!(labels(&log)?.is_empty());
    Ok(())
}

#[tokio::test]
async fn release_removes_a_resource_from_teardown() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    let kept = reaper.register(ResourceKind::Sandbox, recorder("kept", &log, false))?;
    let released = reaper.register(ResourceKind::Sandbox, recorder("released", &log, false))?;

    reaper.release(released)?;
    assert!(matches!(
        reaper.release(released),
        Err(ReaperError::UnknownResource)
    ));
    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert_eq!(labels(&log)?, ["kept"]);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.outcome(kept), Some(&Outcome::Released));
    assert_eq!(report.outcome(released), None);
    Ok(())
}

#[tokio::test]
async fn reason_is_passed_to_every_resource() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let log = Log::default();
    reaper.register(ResourceKind::Task, recorder("t", &log, false))?;
    reaper.register(ResourceKind::TempDir, recorder("d", &log, false))?;
    let config = ShutdownConfig {
        reason: TeardownReason::CtrlC,
        ..ShutdownConfig::default()
    };

    reaper.shutdown_all(&config).await?;

    let guard = log.lock().map_err(|_| "log lock poisoned")?;
    assert_eq!(guard.len(), 2);
    assert!(
        guard
            .iter()
            .all(|(_, reason)| *reason == TeardownReason::CtrlC)
    );
    Ok(())
}
