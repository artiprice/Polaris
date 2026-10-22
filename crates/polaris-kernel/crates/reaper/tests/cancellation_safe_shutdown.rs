//! A dropped shutdown waiter must not detach cleanup from durable reaper state.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use polaris_kernel_reaper::{
    Disposition, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext, TeardownError,
};

type TestResult = Result<(), Box<dyn Error>>;

struct DelayedCleanup(Arc<AtomicBool>);

impl Teardown for DelayedCleanup {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.0.store(true, Ordering::SeqCst);
        Ok(Disposition::Clean)
    }
}

#[tokio::test]
async fn aborting_shutdown_waiter_keeps_cleanup_and_journal_updates_running() -> TestResult {
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let cleaned = Arc::new(AtomicBool::new(false));
    reaper.register(ResourceKind::Task, DelayedCleanup(Arc::clone(&cleaned)))?;

    let waiter = tokio::spawn({
        let reaper = Arc::clone(&reaper);
        async move { reaper.shutdown_all(&ShutdownConfig::default()).await }
    });
    for _attempt in 0..20 {
        if reaper.is_cancelled() {
            break;
        }
        tokio::task::yield_now().await;
    }
    waiter.abort();
    let _aborted_waiter = waiter.await;

    tokio::time::timeout(Duration::from_secs(1), async {
        while !cleaned.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !reaper.leak_report().entries.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}
