//! Errors intentionally omit trajectory text and serialization error excerpts.
use polaris_sdk::error::{ErrorKind, HasKind};

/// Local collection, masking or export failure. Every failure closes the export gate.
#[derive(Debug, thiserror::Error)]
pub enum DataLabError {
    /// Persisted consent is absent, disabled, revoked or belongs to another session.
    #[error("session has not opted in to trajectory export")]
    NotOptedIn,
    /// Proof is missing, not passing or bound to different artifacts.
    #[error("trajectory lacks matching proof-of-done evidence")]
    NotVerified,
    /// License/provenance policy excludes this trajectory.
    #[error("trajectory provenance is not exportable under the configured policy")]
    ProvenanceExcluded,
    /// A second scan still found sensitive content.
    #[error("residual secret or personal data blocks export")]
    ResidualSecret,
    /// A resource bound was exceeded.
    #[error("trajectory exceeds processing bounds")]
    Limit,
    /// Malformed versioned record or tool argument shape.
    #[error("invalid trajectory record: {0}")]
    Schema(&'static str),
    /// Consent state lock is poisoned; do not guess whether consent is active.
    #[error("consent state unavailable")]
    ConsentUnavailable,
    /// CAS returned an error or failed content identity verification.
    #[error("trajectory content storage failed: {0}")]
    Cas(#[from] polaris_storage_cas::CasError),
    /// Scanner could not complete, including too many matches.
    #[error("residual scan failed: {0}")]
    Scanner(#[from] polaris_security_automata::AutomataError),
    /// Persistent local export failed.
    #[error("local export I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Referenced bytes did not match their claimed content identity.
    #[error("trajectory content identity mismatch")]
    DigestMismatch,
}

impl HasKind for DataLabError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::NotOptedIn
            | Self::NotVerified
            | Self::ProvenanceExcluded
            | Self::ResidualSecret => ErrorKind::Policy,
            Self::Limit | Self::Schema(_) => ErrorKind::Permanent,
            Self::ConsentUnavailable | Self::DigestMismatch => ErrorKind::Fatal,
            Self::Cas(error) => error.kind(),
            Self::Scanner(error) => error.kind(),
            Self::Io(_) => ErrorKind::Transient,
        }
    }
}
