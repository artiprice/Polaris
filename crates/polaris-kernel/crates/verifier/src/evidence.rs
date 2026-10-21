//! Content-addressed evidence persistence seam.

use polaris_sdk::ids::Cid;

/// Failure while writing or reading a verification record from the configured CAS backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    /// The evidence backend failed to persist or retrieve a record.
    #[error("verification evidence storage failed")]
    Backend,
    /// Read-back evidence did not match the requested content id.
    #[error("verification evidence failed digest verification")]
    DigestMismatch,
}

/// Injected content-addressed storage used for durable approval evidence.
///
/// Implementations must store immutable bytes under their BLAKE3 id. The verifier independently
/// checks the returned id and performs a read-back before it creates an approval witness.
pub trait EvidenceStore: Send + Sync {
    /// Persist bytes and return their BLAKE3 content id.
    ///
    /// # Errors
    /// Returns [`EvidenceError::Backend`] on storage failure.
    fn put(&self, bytes: &[u8]) -> Result<Cid, EvidenceError>;

    /// Read bytes previously written under `cid`.
    ///
    /// # Errors
    /// Returns [`EvidenceError::Backend`] on storage failure.
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, EvidenceError>;
}
