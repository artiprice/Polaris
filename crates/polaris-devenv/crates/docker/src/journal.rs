//! Durable ownership seam for Docker-managed warm pools.
use std::fmt::Debug;

use futures::future::BoxFuture;
use thiserror::Error;
use uuid::Uuid;

/// Stable identity written before a managed pool creates any Engine resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolIdentity {
    /// Pool label attached to every managed container.
    pub pool_id: Uuid,
    /// Owner label attached to every managed container.
    pub owner_id: Uuid,
}

impl PoolIdentity {
    /// Parse persisted pool and owner UUID labels into a typed identity.
    ///
    /// # Errors
    /// Returns [`PoolJournalError::InvalidIdentity`] if either value is malformed.
    pub fn parse(pool_id: &str, owner_id: &str) -> Result<Self, PoolJournalError> {
        let pool_id = Uuid::parse_str(pool_id).map_err(|_| PoolJournalError::InvalidIdentity)?;
        let owner_id = Uuid::parse_str(owner_id).map_err(|_| PoolJournalError::InvalidIdentity)?;
        Ok(Self { pool_id, owner_id })
    }
}

/// Opaque registration key returned by a [`PoolJournal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolRegistration(String);

impl PoolRegistration {
    /// Construct a registration key for a journal implementation.
    ///
    /// # Errors
    /// Returns [`PoolJournalError::InvalidRegistration`] for an empty or control-bearing key.
    pub fn new(token: impl Into<String>) -> Result<Self, PoolJournalError> {
        let token = token.into();
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(PoolJournalError::InvalidRegistration);
        }
        Ok(Self(token))
    }

    /// Borrow this journal's opaque registration key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A crashed pool registration awaiting exact-label recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolOrphan {
    /// Reaper registration used to acknowledge recovery.
    pub registration: PoolRegistration,
    /// Exact Docker label identity to recover.
    pub identity: PoolIdentity,
}

/// Failure at the Docker-owned persistence boundary.
#[derive(Debug, Error)]
pub enum PoolJournalError {
    /// The journal rejected or could not persist a registration.
    #[error("pool journal registration failed: {0}")]
    Register(String),
    /// The journal could not release a completed registration.
    #[error("pool journal release failed: {0}")]
    Release(String),
    /// The journal could not list confirmed orphan registrations.
    #[error("pool journal orphan listing failed: {0}")]
    ListOrphans(String),
    /// The journal could not acknowledge a recovered orphan.
    #[error("pool journal orphan acknowledgement failed: {0}")]
    Acknowledge(String),
    /// Exact-label backend cleanup failed during orphan recovery.
    #[error("pool orphan cleanup failed: {0}")]
    Cleanup(String),
    /// A persisted pool or owner identity is malformed.
    #[error("invalid Docker pool identity")]
    InvalidIdentity,
    /// A registration key is malformed.
    #[error("invalid pool journal registration key")]
    InvalidRegistration,
}

/// Failure while removing a pool's containers by their exact ownership labels.
#[derive(Debug, Error)]
#[error("pool cleanup failed: {0}")]
pub struct PoolCleanupError(pub String);

/// Async cleanup action retained by the lifecycle journal and invoked by Reaper shutdown.
pub trait PoolCleanup: Debug + Send + Sync {
    /// Remove all containers carrying both exact IDs in `identity`.
    ///
    /// # Errors
    /// Returns an error if Docker cannot confirm their removal.
    fn cleanup<'a>(
        &'a self,
        identity: &'a PoolIdentity,
    ) -> BoxFuture<'a, Result<(), PoolCleanupError>>;
}

/// Durable registration and orphan-recovery boundary owned by Docker.
///
/// Implementations must make `register` durable before returning. `pending_orphans` must contain
/// only registrations that no longer have a live in-process cleanup owner.
pub trait PoolJournal: Debug + Send + Sync {
    /// Durably register cleanup before the Docker Engine creates a pool container.
    ///
    /// # Errors
    /// Returns an error if the cleanup registration cannot be made durable.
    fn register(
        &self,
        identity: PoolIdentity,
        cleanup: Box<dyn PoolCleanup>,
    ) -> Result<PoolRegistration, PoolJournalError>;

    /// Remove a registration after authoritative Docker destruction has completed.
    ///
    /// # Errors
    /// Returns an error if the registration is unknown or its journal update fails.
    fn release(&self, registration: &PoolRegistration) -> Result<(), PoolJournalError>;

    /// List only persisted registrations whose owning process is gone.
    ///
    /// # Errors
    /// Returns an error if persisted orphan records cannot be read or decoded.
    fn pending_orphans(&self) -> Result<Vec<PoolOrphan>, PoolJournalError>;

    /// Acknowledge an orphan only after its exact-label resources are absent.
    ///
    /// # Errors
    /// Returns an error if the registration is live, unknown, or could not be durably removed.
    fn acknowledge_recovered(
        &self,
        registration: &PoolRegistration,
    ) -> Result<(), PoolJournalError>;
}
