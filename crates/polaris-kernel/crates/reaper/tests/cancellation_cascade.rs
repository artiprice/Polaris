//! Cancellation Cascade (descriptor: specs/descriptors/reaper_cancellation_cascade.json).

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use polaris_kernel_reaper::{
    Disposition, Outcome, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
    TeardownError,
};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

fn reaper_in(dir: &tempfile::TempDir) -> Result<Reaper, Box<dyn Error>> {
    Ok(Reaper::new(dir.path())?)
}

/// Records whether its token was already cancelled when the reaper reached it.
struct TokenProbe {
    token: CancellationToken,
    saw_cancelled: Arc<AtomicBool>,
}

impl Teardown for TokenProbe {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.saw_cancelled
            .store(self.token.is_cancelled(), Ordering::SeqCst);
        Ok(Disposition::Clean)
    }
}

#[test]
fn cancelling_root_cancels_grandchildren() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    let child = reaper.child_token();
    let grandchild = child.child_token();
    let great_grandchild = grandchild.child_token();
    assert!(!reaper.is_cancelled());
    assert!(!great_grandchild.is_cancelled());

    reaper.cancel();

    assert!(reaper.is_cancelled());
    assert!(child.is_cancelled());
    assert!(grandchild.is_cancelled());
    assert!(great_grandchild.is_cancelled());
    Ok(())
}

#[test]
fn cancelling_child_leaves_sibling_running() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    let left = reaper.child_token();
    let left_leaf = left.child_token();
    let right = reaper.child_token();
    let right_leaf = right.child_token();

    left.cancel();

    assert!(left.is_cancelled());
    assert!(left_leaf.is_cancelled());
    assert!(!right.is_cancelled());
    assert!(!right_leaf.is_cancelled());
    assert!(!reaper.is_cancelled());
    Ok(())
}

#[test]
fn token_created_after_cancel_is_already_cancelled() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    reaper.cancel();
    assert!(reaper.child_token().is_cancelled());
    Ok(())
}

#[tokio::test]
async fn shutdown_cancels_root_before_any_teardown() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    let saw_cancelled = Arc::new(AtomicBool::new(false));
    reaper.register(
        ResourceKind::Sandbox,
        TokenProbe {
            token: reaper.child_token(),
            saw_cancelled: Arc::clone(&saw_cancelled),
        },
    )?;

    reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert!(saw_cancelled.load(Ordering::SeqCst));
    assert!(reaper.is_cancelled());
    Ok(())
}

#[tokio::test]
async fn cancellation_wakes_a_cooperative_task() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    let token = reaper.child_token();
    let handle = tokio::spawn(async move { token.cancelled().await });
    let id = reaper.register_task(handle)?;

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert_eq!(report.outcome(id), Some(&Outcome::Released));
    assert!(!report.deadline_exceeded);
    Ok(())
}

#[test]
fn dropping_the_reaper_cancels_the_tree() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = reaper_in(&dir)?;
    let child = reaper.child_token();

    drop(reaper);

    assert!(child.is_cancelled());
    Ok(())
}
