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
#[path = "../../../tests/crates/polaris-kernel/src/docker_pool.rs"]
mod tests;
