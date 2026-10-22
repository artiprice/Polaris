//! Errors never contain scanned source or credential values.
use polaris_sdk::error::{ErrorKind, HasKind};

/// Failure to establish a verifier result; callers must fail closed.
#[derive(Debug, thiserror::Error)]
pub enum VerifierError {
    /// Analysis could not complete within parser bounds.
    #[error("source analysis failed: {0}")]
    Parse(#[from] polaris_code_intel::CodeIntelError),
    /// Scanning failed, including exhausted bounded match storage.
    #[error("safety scan failed: {0}")]
    Scanner(#[from] polaris_security_automata::AutomataError),
    /// Invalid schema or returned decision shape.
    #[error("decision schema failed: {0}")]
    Schema(#[from] polaris_system_one_schemas::SchemaError),
    /// Input exceeded a processing bound.
    #[error("verifier input exceeds {limit} bytes")]
    InputLimit {
        /// Maximum accepted bytes.
        limit: usize,
    },
    /// Invalid caller input or configuration.
    #[error("invalid verifier input: {reason}")]
    InvalidInput {
        /// Static non-sensitive reason.
        reason: &'static str,
    },
}

impl HasKind for VerifierError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Parse(error) => error.kind(),
            Self::Scanner(error) => error.kind(),
            Self::Schema(_) | Self::InputLimit { .. } | Self::InvalidInput { .. } => {
                ErrorKind::Permanent
            }
        }
    }
}
