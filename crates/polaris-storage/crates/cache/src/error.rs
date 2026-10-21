//! Cache errors.

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::registry::CacheStoreError;

/// Tiered-cache failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    /// The L2 exact store failed (I/O, corruption, oversized value).
    #[error("L2 cache store: {0}")]
    L2(#[from] CacheStoreError),
    /// An L2 value does not carry a valid envelope.
    #[error("cache entry codec: {0}")]
    Codec(String),
    /// Invalid configuration (e.g. zero capacity).
    #[error("invalid cache configuration: {0}")]
    Config(String),
}

impl HasKind for CacheError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::L2(inner) => inner.kind(),
            Self::Codec(_) | Self::Config(_) => ErrorKind::Permanent,
        }
    }
}
