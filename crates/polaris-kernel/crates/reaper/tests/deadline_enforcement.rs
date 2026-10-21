//! Deadline Enforcement (descriptor: specs/descriptors/reaper_deadline_enforcement.json).
//!
//! All timing uses tokio's paused clock, so the asserted durations are exact virtual time.

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_kernel_reaper::{
    Disposition, Outcome, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
    TeardownError, TeardownReason,
};

type TestResult = Result<(), Box<dyn Error>>;
type Events = Arc<Mutex<Vec<String>>>;

#[derive(Clone, Copy)]
enum Script {
    Ok,
    FailGraceful,
    HangThenForceOk,
    HangForever,
}

struct Scripted {
    label: &'static str,
    script: Script,
    events: Events,
}

fn note(events: &Events, entry: String) {
    if let Ok(mut events) = events.lock() {
        events.push(entry);
    }
}

impl Teardown for Scripted {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        note(&self.events, format!("graceful:{}", self.label));
        match self.script {
            Script::Ok => Ok(Disposition::Clean),
            Script::FailGraceful => Err(TeardownError::Other("boom".to_owned())),
            Script::HangThenForceOk | Script::HangForever => {
                std::future::pending::<Result<Disposition, TeardownError>>().await
            }
        }
    }

    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        note(&self.events, format!("force:{}", self.label));
        match self.script {
            Script::HangThenForceOk => Ok(()),
            _ => std::future::pending::<Result<(), TeardownError>>().await,
        }
    }
}

fn scripted(label: &'static str, script: Script, events: &Events) -> Scripted {
    Scripted {
        label,
        script,
        events: Arc::clone(events),
    }
}

fn recorded(events: &Events) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(events.lock().map_err(|_| "events lock poisoned")?.clone())
}

fn config(graceful: u64, forced: u64, overall: u64) -> ShutdownConfig {
    ShutdownConfig {
        reason: TeardownReason::Shutdown,
        graceful: Duration::from_secs(graceful),
        forced: Duration::from_secs(forced),
        overall: Duration::from_secs(overall),
    }
}

#[tokio::test(start_paused = true)]
async fn hanging_resource_is_abandoned_at_its_deadline_and_others_complete() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let events = Events::default();
    let first = reaper.register(
        ResourceKind::Sandbox,
        scripted("first", Script::Ok, &events),
    )?;
    let hung = reaper.register(
        ResourceKind::Sandbox,
        scripted("hung", Script::HangForever, &events),
    )?;
    let last = reaper.register(ResourceKind::Sandbox, scripted("last", Script::Ok, &events))?;

    let report = reaper.shutdown_all(&config(2, 1, 60)).await?;

    assert_eq!(report.outcome(last), Some(&Outcome::Released));
    assert_eq!(report.outcome(hung), Some(&Outcome::Abandoned));
    assert_eq!(report.outcome(first), Some(&Outcome::Released));
    assert_eq!(report.elapsed, Duration::from_secs(3));
    assert!(!report.deadline_exceeded);
    assert_eq!(
        recorded(&events)?,
        [
            "graceful:last",
            "graceful:hung",
            "force:hung",
            "graceful:first"
        ]
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn graceful_hang_then_successful_force_is_forced() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let events = Events::default();
    let id = reaper.register(
        ResourceKind::Process,
        scripted("stubborn", Script::HangThenForceOk, &events),
    )?;

    let report = reaper.shutdown_all(&config(2, 1, 60)).await?;

    assert_eq!(report.outcome(id), Some(&Outcome::Forced));
    assert_eq!(report.elapsed, Duration::from_secs(2));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn failing_graceful_step_is_not_forced() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let events = Events::default();
    let id = reaper.register(
        ResourceKind::Sandbox,
        scripted("broken", Script::FailGraceful, &events),
    )?;

    let report = reaper.shutdown_all(&config(2, 1, 60)).await?;

    assert_eq!(
        report.outcome(id),
        Some(&Outcome::Failed("boom".to_owned()))
    );
    assert_eq!(recorded(&events)?, ["graceful:broken"]);
    assert_eq!(report.elapsed, Duration::ZERO);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn overall_deadline_skips_the_remaining_resources() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let events = Events::default();
    let h1 = reaper.register(
        ResourceKind::Sandbox,
        scripted("h1", Script::HangForever, &events),
    )?;
    let h2 = reaper.register(
        ResourceKind::Sandbox,
        scripted("h2", Script::HangForever, &events),
    )?;
    let h3 = reaper.register(
        ResourceKind::Sandbox,
        scripted("h3", Script::HangForever, &events),
    )?;

    let report = reaper.shutdown_all(&config(1, 1, 3)).await?;

    // h3: 1 s graceful + 1 s forced. h2: the last 1 s of budget is spent gracefully, nothing is
    // left to force with. h1: never started.
    assert_eq!(report.outcome(h3), Some(&Outcome::Abandoned));
    assert_eq!(report.outcome(h2), Some(&Outcome::Abandoned));
    assert_eq!(report.outcome(h1), Some(&Outcome::Skipped));
    assert!(report.deadline_exceeded);
    assert_eq!(report.elapsed, Duration::from_secs(3));
    assert_eq!(
        recorded(&events)?,
        ["graceful:h3", "force:h3", "graceful:h2"]
    );
    Ok(())
}

struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn task_ignoring_cancellation_is_aborted() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = SetOnDrop(Arc::clone(&dropped));
    let handle = tokio::spawn(async move {
        let _guard = guard;
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
    let id = reaper.register_task(handle)?;

    let report = reaper.shutdown_all(&config(2, 1, 60)).await?;

    assert_eq!(report.outcome(id), Some(&Outcome::Forced));
    assert!(
        dropped.load(Ordering::SeqCst),
        "the aborted task's future must have been dropped"
    );
    assert_eq!(report.elapsed, Duration::from_secs(2));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn unresolved_resources_stay_in_the_leak_report() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let events = Events::default();
    let fine = reaper.register(ResourceKind::Task, scripted("fine", Script::Ok, &events))?;
    let hung = reaper.register(
        ResourceKind::Sandbox,
        scripted("hung", Script::HangForever, &events),
    )?;
    let skipped = reaper.register(ResourceKind::TempDir, scripted("tmp", Script::Ok, &events))?;

    // Overall 2 s: the sandbox burns graceful 1 s + forced 1 s, so the temp dir is skipped.
    let report = reaper.shutdown_all(&config(1, 1, 2)).await?;

    assert_eq!(report.outcome(fine), Some(&Outcome::Released));
    assert_eq!(report.outcome(hung), Some(&Outcome::Abandoned));
    assert_eq!(report.outcome(skipped), Some(&Outcome::Skipped));
    let leaked: Vec<_> = reaper.leak_report().entries.iter().map(|e| e.id).collect();
    assert_eq!(leaked, [hung, skipped]);
    Ok(())
}
