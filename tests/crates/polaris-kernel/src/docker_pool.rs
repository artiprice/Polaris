// Unit tests of crates/polaris-kernel/src/docker_pool.rs (`tests`); compiled into that module via #[path].
use super::*;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
#[tokio::test]
async fn live_warm_pool_restart_recovery_uses_exact_owner_journal() {
    if std::env::var("POLARIS_DEVD_DOCKER_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }

    use polaris_devenv::docker::{
        ContainerSpec, Docker, DockerEndpoint, ImageRef, MIN_MEMORY_BYTES, WarmPoolConfig,
    };

    let endpoint = DockerEndpoint::UnixSocket("unix:///var/run/docker.sock".to_owned());
    let docker = Docker::connect(&endpoint).await;
    assert!(
        docker.is_ok(),
        "authorized WSL Docker daemon must be available"
    );
    let Ok(docker) = docker else { return };
    let image =
        ImageRef::new("sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697");
    assert!(
        image.is_ok(),
        "pinned sandbox image reference must validate"
    );
    let Ok(image) = image else { return };
    let spec = ContainerSpec::hardened(
        image,
        vec!["infinity".to_owned()],
        Vec::new(),
        MIN_MEMORY_BYTES,
        1_000_000_000,
    );
    assert!(spec.is_ok(), "bounded warm-pool spec must validate");
    let Ok(spec) = spec else { return };
    let config = WarmPoolConfig::new(1, MIN_MEMORY_BYTES);
    assert!(config.is_ok(), "single-container pool budget must validate");
    let Ok(config) = config else { return };
    let identity = PoolIdentity {
        pool_id: config.pool_id,
        owner_id: config.owner_id,
    };

    let root = tempfile::tempdir();
    assert!(root.is_ok(), "isolated Reaper journal directory must open");
    let Ok(root) = root else { return };
    let journal_path = root.path().join("run");
    let pool = {
        let reaper = Reaper::new(&journal_path);
        assert!(reaper.is_ok(), "fresh Reaper journal must open");
        let Ok(reaper) = reaper else { return };
        let journal = Arc::new(ReaperPoolJournal::new(Arc::new(reaper)));
        let pool = docker
            .create_warm_pool_journaled(spec, config, journal)
            .await;
        assert!(pool.is_ok(), "journaled warm pool must create successfully");
        let Ok(pool) = pool else { return };
        assert_eq!(pool.ready_count(), 1, "one exact-owned container is ready");
        pool
    };
    drop(pool);

    // Reopening the persisted Reaper file models a process restart: active callbacks are
    // gone, while the Docker identity remains an orphan awaiting exact-label recovery.
    let reaper = Reaper::new(&journal_path);
    assert!(reaper.is_ok(), "persisted Reaper journal must reopen");
    let Ok(reaper) = reaper else { return };
    let journal = ReaperPoolJournal::new(Arc::new(reaper));
    let pending = journal.pending_orphans();
    assert!(matches!(pending.as_ref(), Ok(orphans) if orphans.len() == 1));
    assert!(matches!(
        pending.as_ref().ok().and_then(|orphans| orphans.first()),
        Some(orphan) if orphan.identity == identity
    ));

    let recovered = journal.recover_pending(&docker).await;
    assert!(
        matches!(recovered, Ok(1)),
        "restart recovery removes this owner's orphan"
    );
    let still_present = docker
        .recover_pool_orphans(identity.pool_id, identity.owner_id)
        .await;
    assert!(
        matches!(still_present, Ok(0)),
        "exact-label query confirms authoritative absence"
    );
    assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.is_empty()));
}
use tempfile::TempDir;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
struct NoopPoolCleanup;

impl PoolCleanup for NoopPoolCleanup {
    fn cleanup<'a>(
        &'a self,
        _identity: &'a PoolIdentity,
    ) -> BoxFuture<'a, Result<(), PoolCleanupError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug)]
struct RecordingPoolCleanup {
    identities: Arc<Mutex<Vec<PoolIdentity>>>,
    fail: bool,
}

impl PoolCleanup for RecordingPoolCleanup {
    fn cleanup<'a>(
        &'a self,
        identity: &'a PoolIdentity,
    ) -> BoxFuture<'a, Result<(), PoolCleanupError>> {
        let fail = self.fail;
        Box::pin(async move {
            let Ok(mut identities) = self.identities.lock() else {
                return Err(PoolCleanupError("recording lock poisoned".to_owned()));
            };
            identities.push(*identity);
            if fail {
                Err(PoolCleanupError("synthetic Engine failure".to_owned()))
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Debug)]
struct NoopTeardown;

impl Teardown for NoopTeardown {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        Ok(Disposition::Clean)
    }
}

fn reaper_in(temp: &TempDir) -> Result<Reaper, crate::reaper::ReaperError> {
    Reaper::new(Path::new(temp.path()))
}

fn test_identity() -> Option<PoolIdentity> {
    PoolIdentity::parse(
        "018f0000-0000-7000-8000-000000000001",
        "018f0000-0000-7000-8000-000000000002",
    )
    .ok()
}

#[test]
fn live_pool_is_not_reported_as_orphan_and_release_removes_registration() {
    let directory = tempfile::tempdir();
    assert!(directory.is_ok());
    let Ok(directory) = directory else { return };
    let reaper = reaper_in(&directory);
    assert!(reaper.is_ok());
    let Ok(reaper) = reaper else { return };
    let journal = ReaperPoolJournal::new(Arc::new(reaper));
    let Some(identity) = test_identity() else {
        return;
    };
    let registration = journal.register(identity, Box::new(NoopPoolCleanup));
    assert!(registration.is_ok());
    let Ok(registration) = registration else {
        return;
    };
    assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.is_empty()));
    assert!(matches!(journal.release(&registration), Ok(())));
    assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.is_empty()));
}

#[tokio::test]
async fn restart_recovers_only_orphaned_docker_pool_locators() {
    let directory = tempfile::tempdir();
    assert!(directory.is_ok());
    let Ok(directory) = directory else { return };
    let Some(identity) = test_identity() else {
        return;
    };
    let registration = {
        let reaper = reaper_in(&directory);
        assert!(reaper.is_ok());
        let Ok(reaper) = reaper else { return };
        let journal = ReaperPoolJournal::new(Arc::new(reaper));
        let docker_registration = journal.register(identity, Box::new(NoopPoolCleanup));
        assert!(docker_registration.is_ok());
        let Ok(docker_registration) = docker_registration else {
            return;
        };
        let unrelated = journal.reaper.register_journaled(
            ResourceKind::Sandbox,
            Locator::Label("another-backend/resource-7".to_owned()),
            NoopTeardown,
        );
        assert!(unrelated.is_ok());
        docker_registration
    };
    let reaper = reaper_in(&directory);
    assert!(reaper.is_ok());
    let Ok(reaper) = reaper else { return };
    let journal = ReaperPoolJournal::new(Arc::new(reaper));
    let orphans = journal.pending_orphans();
    assert!(matches!(orphans.as_ref(), Ok(entries) if entries.len() == 1));
    assert!(matches!(
        orphans.as_ref().ok().and_then(|entries| entries.first()),
        Some(orphan) if orphan.identity == identity && orphan.registration == registration
    ));
    let cleaned = Arc::new(Mutex::new(Vec::new()));
    let backend = RecordingPoolCleanup {
        identities: Arc::clone(&cleaned),
        fail: false,
    };
    assert!(matches!(
        journal.recover_pending_with(&backend).await,
        Ok(1)
    ));
    assert!(matches!(cleaned.lock(), Ok(ids) if ids.as_slice() == [identity]));
    assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.is_empty()));
    assert_eq!(journal.reaper.pending_orphans().len(), 1);
}

#[tokio::test]
async fn failed_backend_cleanup_keeps_the_orphan_journaled() {
    let directory = tempfile::tempdir();
    assert!(directory.is_ok());
    let Ok(directory) = directory else { return };
    let Some(identity) = test_identity() else {
        return;
    };
    let registration = {
        let reaper = reaper_in(&directory);
        assert!(reaper.is_ok());
        let Ok(reaper) = reaper else { return };
        ReaperPoolJournal::new(Arc::new(reaper)).register(identity, Box::new(NoopPoolCleanup))
    };
    assert!(registration.is_ok());
    let Ok(registration) = registration else {
        return;
    };
    let reaper = reaper_in(&directory);
    assert!(reaper.is_ok());
    let Ok(reaper) = reaper else { return };
    let journal = ReaperPoolJournal::new(Arc::new(reaper));
    let backend = RecordingPoolCleanup {
        identities: Arc::new(Mutex::new(Vec::new())),
        fail: true,
    };
    assert!(matches!(
        journal.recover_pending_with(&backend).await,
        Err(PoolJournalError::Cleanup(_))
    ));
    assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.len() == 1));
    assert!(matches!(
        journal.acknowledge_recovered(&registration),
        Ok(())
    ));
}

#[test]
fn active_pool_registration_cannot_be_acknowledged_as_an_orphan() {
    let directory = tempfile::tempdir();
    assert!(directory.is_ok());
    let Ok(directory) = directory else { return };
    let reaper = reaper_in(&directory);
    assert!(reaper.is_ok());
    let Ok(reaper) = reaper else { return };
    let journal = ReaperPoolJournal::new(Arc::new(reaper));
    let Some(identity) = test_identity() else {
        return;
    };
    let registration = journal.register(identity, Box::new(NoopPoolCleanup));
    assert!(registration.is_ok());
    if let Ok(registration) = registration {
        assert!(matches!(
            journal.acknowledge_recovered(&registration),
            Err(PoolJournalError::Acknowledge(_))
        ));
        assert!(matches!(journal.pending_orphans(), Ok(orphans) if orphans.is_empty()));
    }
}
