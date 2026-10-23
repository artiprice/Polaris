//! L5 adapter connecting Docker pool ownership to the generic Reaper journal.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use polaris_devenv::docker::{
    Docker, PoolCleanup, PoolCleanupError, PoolIdentity, PoolJournal, PoolJournalError, PoolOrphan,
    PoolRegistration,
};

use crate::reaper::{
    Disposition, Locator, ReapId, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};

const LOCATOR_PREFIX: &str = "docker-pool-v1";

/// Reaper-backed journal for Docker warm-pool ownership.
#[derive(Debug)]
pub struct ReaperPoolJournal {
    reaper: Arc<Reaper>,
}

impl ReaperPoolJournal {
    /// Connect a Docker pool journal to the Kernel's existing Reaper instance.
    #[must_use]
    pub const fn new(reaper: Arc<Reaper>) -> Self {
        Self { reaper }
    }

    /// Recover persisted Docker pool orphans before creating new work.
    ///
    /// # Errors
    /// Recover persisted Docker pool orphans using the live Engine as exact-label cleanup backend.
    ///
    /// # Errors
    /// Returns a typed journal failure if cleanup or durable acknowledgement fails.
    pub async fn recover_pending(&self, docker: &Docker) -> Result<usize, PoolJournalError> {
        self.recover_pending_with(&DockerPoolRecovery { docker })
            .await
    }

    /// Recover persisted Docker pool orphans through an injected exact-label cleanup backend.
    ///
    /// This seam also allows restart recovery to be exercised without a live Docker daemon.
    ///
    /// # Errors
    /// Returns a typed error when cleanup fails or a recovered registration cannot be acknowledged.
    pub async fn recover_pending_with(
        &self,
        cleanup: &dyn PoolCleanup,
    ) -> Result<usize, PoolJournalError> {
        let orphans = self.pending_orphans()?;
        let mut recovered = 0_usize;
        for orphan in orphans {
            cleanup
                .cleanup(&orphan.identity)
                .await
                .map_err(|error| PoolJournalError::Cleanup(error.to_string()))?;
            self.acknowledge_recovered(&orphan.registration)?;
            recovered = recovered.saturating_add(1);
        }
        Ok(recovered)
    }
}

#[derive(Debug)]
struct DockerPoolRecovery<'a> {
    docker: &'a Docker,
}

impl PoolCleanup for DockerPoolRecovery<'_> {
    fn cleanup<'a>(
        &'a self,
        identity: &'a PoolIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), PoolCleanupError>> + Send + 'a>> {
        Box::pin(async move {
            self.docker
                .recover_pool_orphans(identity.pool_id, identity.owner_id)
                .await
                .map(|_removed| ())
                .map_err(|error| PoolCleanupError(error.to_string()))
        })
    }
}

impl PoolJournal for ReaperPoolJournal {
    /// Persist cleanup metadata and retain the callback for orderly Kernel shutdown.
    ///
    /// # Errors
    /// Returns a typed error if Reaper cannot durably add the registration.
    fn register(
        &self,
        identity: PoolIdentity,
        cleanup: Box<dyn PoolCleanup>,
    ) -> Result<PoolRegistration, PoolJournalError> {
        let locator = encode_locator(identity);
        let id = self
            .reaper
            .register_journaled(
                ResourceKind::Sandbox,
                Locator::Label(locator),
                ReaperPoolCleanup { identity, cleanup },
            )
            .map_err(|error| PoolJournalError::Register(error.to_string()))?;
        match PoolRegistration::new(id.as_raw().to_string()) {
            Ok(registration) => Ok(registration),
            Err(error) => match self.reaper.release(id) {
                Ok(()) => Err(error),
                Err(release_error) => Err(PoolJournalError::Register(format!(
                    "{error}; rollback failed: {release_error}"
                ))),
            },
        }
    }

    /// Release only an active registration encoded as a Docker pool locator.
    ///
    /// # Errors
    /// Returns a typed error for malformed, unknown, or unresolved orphan registrations.
    fn release(&self, registration: &PoolRegistration) -> Result<(), PoolJournalError> {
        let id = registration_id(registration)?;
        let matches = self.reaper.leak_report().entries.iter().any(|entry| {
            entry.id == id
                && entry.kind == ResourceKind::Sandbox
                && matches!(&entry.locator, Locator::Label(locator) if decode_locator(locator).is_ok())
        });
        if !matches {
            return Err(PoolJournalError::Release(
                "registration is not an active Docker pool".to_owned(),
            ));
        }
        self.reaper
            .release(id)
            .map_err(|error| PoolJournalError::Release(error.to_string()))
    }

    /// Return only orphaned Docker pool entries, excluding active registrations and other kinds.
    ///
    /// # Errors
    /// Returns a typed error if a persisted Docker locator is malformed.
    fn pending_orphans(&self) -> Result<Vec<PoolOrphan>, PoolJournalError> {
        self.reaper
            .pending_orphans()
            .into_iter()
            .filter_map(|entry| match entry.locator {
                Locator::Label(locator) if locator.starts_with(&format!("{LOCATOR_PREFIX}/")) => {
                    Some(decode_locator(&locator).and_then(|identity| {
                        Ok(PoolOrphan {
                            registration: PoolRegistration::new(entry.id.as_raw().to_string())?,
                            identity,
                        })
                    }))
                }
                _ => None,
            })
            .collect()
    }

    /// Clear a Docker orphan after the caller confirms exact-label Engine cleanup.
    ///
    /// # Errors
    /// Returns a typed error if the registration is active, unknown, or cannot be removed.
    fn acknowledge_recovered(
        &self,
        registration: &PoolRegistration,
    ) -> Result<(), PoolJournalError> {
        let id = registration_id(registration)?;
        let is_docker_orphan = self.reaper.pending_orphans().iter().any(|entry| {
            entry.id == id
                && entry.kind == ResourceKind::Sandbox
                && matches!(&entry.locator, Locator::Label(locator) if decode_locator(locator).is_ok())
        });
        if !is_docker_orphan {
            return Err(PoolJournalError::Acknowledge(
                "registration is not a pending Docker pool orphan".to_owned(),
            ));
        }
        self.reaper
            .acknowledge_orphan(id)
            .map_err(|error| PoolJournalError::Acknowledge(error.to_string()))
    }
}

#[derive(Debug)]
struct ReaperPoolCleanup {
    identity: PoolIdentity,
    cleanup: Box<dyn PoolCleanup>,
}

impl Teardown for ReaperPoolCleanup {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.cleanup
            .cleanup(&self.identity)
            .await
            .map_err(|error: PoolCleanupError| TeardownError::Other(error.to_string()))?;
        Ok(Disposition::Forced)
    }

    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        self.cleanup
            .cleanup(&self.identity)
            .await
            .map_err(|error: PoolCleanupError| TeardownError::Other(error.to_string()))
    }
}

fn encode_locator(identity: PoolIdentity) -> String {
    format!(
        "{LOCATOR_PREFIX}/{}/{}",
        identity.pool_id, identity.owner_id
    )
}

fn decode_locator(locator: &str) -> Result<PoolIdentity, PoolJournalError> {
    let mut parts = locator.split('/');
    let prefix = parts.next();
    let pool = parts.next();
    let owner = parts.next();
    if prefix != Some(LOCATOR_PREFIX) || parts.next().is_some() {
        return Err(PoolJournalError::ListOrphans(
            "invalid Docker pool locator".to_owned(),
        ));
    }
    let pool =
        pool.ok_or_else(|| PoolJournalError::ListOrphans("missing pool identity".to_owned()))?;
    let owner =
        owner.ok_or_else(|| PoolJournalError::ListOrphans("missing owner identity".to_owned()))?;
    PoolIdentity::parse(pool, owner)
        .map_err(|_| PoolJournalError::ListOrphans("invalid pool owner identity".to_owned()))
}

fn registration_id(registration: &PoolRegistration) -> Result<ReapId, PoolJournalError> {
    let raw = registration
        .as_str()
        .parse::<u64>()
        .map_err(|_| PoolJournalError::InvalidRegistration)?;
    ReapId::from_raw(raw).map_err(|_| PoolJournalError::InvalidRegistration)
}

#[cfg(test)]
mod tests {
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
        let image = ImageRef::new(
            "sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697",
        );
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
}
