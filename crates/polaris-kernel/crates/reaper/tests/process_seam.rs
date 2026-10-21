//! Process Seam (descriptor: specs/descriptors/reaper_process_seam.json).
//!
//! The real Job Object / process-group implementation of `PlatformOps` is
//! `polaris-platform-process` (P2); here the seam is exercised with a fake.

use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_kernel_reaper::{Locator, Outcome, Reaper, ResourceKind, ShutdownConfig};
use polaris_sdk::registry::{
    MemoryPressure, PlatformError, PlatformOps, ProcessTree, SpawnSpec, StopOutcome,
};

type TestResult = Result<(), Box<dyn Error>>;

struct FakePlatform {
    answer: Result<StopOutcome, PlatformError>,
    calls: Mutex<Vec<(u32, Duration)>>,
}

impl FakePlatform {
    fn answering(answer: Result<StopOutcome, PlatformError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Result<Vec<(u32, Duration)>, Box<dyn Error>> {
        Ok(self
            .calls
            .lock()
            .map_err(|_| "calls lock poisoned")?
            .clone())
    }
}

impl PlatformOps for FakePlatform {
    fn spawn_tree(&self, _spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
        Err(PlatformError::Unsupported("fake platform does not spawn"))
    }

    async fn graceful_stop(
        &self,
        tree: &ProcessTree,
        grace: Duration,
    ) -> Result<StopOutcome, PlatformError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((tree.root_pid, grace));
        }
        self.answer.clone()
    }

    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
        Ok(MemoryPressure::Normal)
    }
}

async fn stop_one(
    answer: Result<StopOutcome, PlatformError>,
) -> Result<(Outcome, Arc<FakePlatform>), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let platform = FakePlatform::answering(answer);
    let id = reaper.register_process(Arc::clone(&platform), ProcessTree { root_pid: 4242 })?;
    let config = ShutdownConfig {
        graceful: Duration::from_secs(8),
        ..ShutdownConfig::default()
    };
    let report = reaper.shutdown_all(&config).await?;
    let outcome = report
        .outcome(id)
        .cloned()
        .ok_or("process missing from report")?;
    Ok((outcome, platform))
}

#[tokio::test]
async fn process_is_stopped_through_platform_ops() -> TestResult {
    let (outcome, platform) = stop_one(Ok(StopOutcome::Exited { code: Some(0) })).await?;

    assert_eq!(outcome, Outcome::Released);
    assert_eq!(platform.calls()?, [(4242, Duration::from_secs(4))]);
    Ok(())
}

#[tokio::test]
async fn killed_after_grace_is_reported_forced() -> TestResult {
    let (outcome, _) = stop_one(Ok(StopOutcome::Killed)).await?;

    assert_eq!(outcome, Outcome::Forced);
    Ok(())
}

#[tokio::test]
async fn platform_error_is_reported_failed() -> TestResult {
    let (outcome, platform) = stop_one(Err(PlatformError::PermissionDenied(
        "job object".to_owned(),
    )))
    .await?;

    match outcome {
        Outcome::Failed(text) => assert!(text.contains("job object"), "{text}"),
        other => return Err(format!("expected Failed, got {other:?}").into()),
    }
    assert_eq!(platform.calls()?.len(), 1, "a refused stop is not retried");
    Ok(())
}

#[test]
fn process_registration_is_journaled_by_pid() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let platform = FakePlatform::answering(Ok(StopOutcome::Killed));

    let id = reaper.register_process(platform, ProcessTree { root_pid: 777 })?;

    let report = reaper.leak_report();
    let entry = report.entries.first().ok_or("process not listed")?;
    assert_eq!(entry.id, id);
    assert_eq!(entry.kind, ResourceKind::Process);
    assert_eq!(entry.locator, Locator::Pid(777));
    let journal =
        std::fs::read_to_string(dir.path().join(polaris_kernel_reaper::JOURNAL_FILE_NAME))?;
    assert!(journal.contains("777"), "{journal}");
    Ok(())
}
