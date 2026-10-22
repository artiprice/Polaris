//! Provider error types without secret-bearing diagnostics.

use polaris_sdk::registry::ModelError;

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
            TfError::Http { status, .. } => Self::Upstream { status },
            other => Self::Protocol(other.to_string()),
        }
    }
}
