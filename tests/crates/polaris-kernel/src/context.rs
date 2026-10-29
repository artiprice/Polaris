// Unit tests of crates/polaris-kernel/src/context.rs (`tests`); compiled into that module via #[path].
use super::*;
use polaris_devenv::docker::{PoolCleanupError, PoolIdentity};
use std::future::Future;
use std::pin::Pin;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
struct RecordingCleanup(Mutex<Vec<PoolIdentity>>);

impl PoolCleanup for RecordingCleanup {
    fn cleanup<'a>(
        &'a self,
        identity: &'a PoolIdentity,
    ) -> BoxFuture<'a, Result<(), PoolCleanupError>> {
        Box::pin(async move {
            let Ok(mut identities) = self.0.lock() else {
                return Err(PoolCleanupError("recording lock poisoned".to_owned()));
            };
            identities.push(*identity);
            Ok(())
        })
    }
}

#[tokio::test]
async fn startup_recovers_docker_orphans_before_the_generic_reaper_sweep()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let identity = PoolIdentity::parse(
        "018f0000-0000-7000-8000-000000000001",
        "018f0000-0000-7000-8000-000000000002",
    )?;
    {
        let reaper = Reaper::new(root.path().join("run"))?;
        let _registration = reaper.register_journaled(
            crate::reaper::ResourceKind::Sandbox,
            crate::reaper::Locator::Label(format!(
                "docker-pool-v1/{}/{}",
                identity.pool_id, identity.owner_id
            )),
            StartupCleanup,
        )?;
    }

    let cleanup = RecordingCleanup(Mutex::new(Vec::new()));
    let context = KernelCtx::open_with_pool_cleanup(root.path(), &cleanup).await?;
    let cleaned = cleanup
        .0
        .lock()
        .map_err(|_| "recording lock poisoned")?
        .clone();
    assert_eq!(cleaned, [identity]);
    assert!(context.reaper.pending_orphans().is_empty());
    Ok(())
}

#[derive(Debug)]
struct StartupCleanup;

impl crate::reaper::Teardown for StartupCleanup {
    async fn graceful(
        &mut self,
        _ctx: crate::reaper::TeardownContext,
    ) -> Result<crate::reaper::Disposition, crate::reaper::TeardownError> {
        Ok(crate::reaper::Disposition::Clean)
    }
}
