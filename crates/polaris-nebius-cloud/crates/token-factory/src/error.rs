//! Provider error types without secret-bearing diagnostics.

use polaris_sdk::registry::ModelError;
use polaris_sdk::{
    error::{ErrorKind, HasKind},
    secrets::SecretSourceError,
};

/// Credential resolution failures without source identifiers or secret values.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum CredentialError {
    /// The explicitly selected source could not complete the read.
    #[error("credential provider failed: {0}")]
    Source(#[from] SecretSourceError),
    /// The selected location has no value; no other location is tried.
    #[error("configured credential is missing")]
    Missing,
    /// The provider returned an empty or header-unsafe credential.
    #[error("configured credential is invalid")]
    Invalid,
}
impl HasKind for CredentialError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Source(error) => error.kind(),
            Self::Missing | Self::Invalid => ErrorKind::Permanent,
        }
    }
}

/// Configuration, transport, or protocol failure in the Token Factory adapter.
#[derive(Debug, thiserror::Error)]
pub enum TfError {
    /// Invalid base URL, limits, credential, or configured embedding dimension.
    #[error("invalid Token Factory client configuration")]
    Configuration,
    /// HTTP status failure. Response bodies are intentionally not included.
    #[error("Token Factory returned HTTP status {status}")]
    Http {
        status: u16,
        retry_after_ms: Option<u64>,
    },
    /// Request was rejected as malformed or too large.
    #[error("Token Factory rejected the request")]
    BadRequest,
    /// Credential was missing or rejected.
    #[error("Token Factory authentication failed")]
    Unauthorized,
    /// Request or body stream exceeded its configured deadline.
    #[error("Token Factory request timed out")]
    Timeout,
    /// An SSE frame or aggregate response exceeded a configured bound.
    #[error("Token Factory response exceeded a configured bound")]
    TooLarge,
    /// The HTTP body ended before a complete protocol stream was received.
    #[error("Token Factory stream ended before completion")]
    StreamTruncated,
    /// Local rate-limit accounting failed closed.
    #[error("Token Factory local rate-limit accounting failed")]
    RateLimit,
    /// The response did not match the supported OpenAI-compatible contract.
    #[error("invalid Token Factory response protocol")]
    Protocol,
    /// JSON request/response encoding failed; raw contents are never included.
    #[error("Token Factory JSON encoding failed")]
    Json,
    /// Transport error scrubbed to avoid retaining URLs or request details.
    #[error("Token Factory transport failed")]
    Transport,
}

impl From<TfError> for ModelError {
    fn from(error: TfError) -> Self {
        match error {
            TfError::Http {
                status: 429,
                retry_after_ms,
            } => Self::RateLimited { retry_after_ms },
            TfError::Unauthorized => Self::Unauthorized,
            TfError::Timeout => Self::Timeout,
            TfError::BadRequest => Self::BadRequest("provider rejected request".to_owned()),
            TfError::Transport => Self::Transport("provider transport failed".to_owned()),
            TfError::RateLimit => Self::Protocol("local rate-limit accounting failed".to_owned()),
            TfError::Http { status, .. } => Self::Upstream { status },
            other => Self::Protocol(other.to_string()),
        }
    }
}
