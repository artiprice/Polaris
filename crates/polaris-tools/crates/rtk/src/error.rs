//! RTK errors.

use std::io;

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_storage_cas::CasError;

/// Failures of the RTK engine.
#[derive(Debug, thiserror::Error)]
pub enum RtkError {
    /// Filesystem failure (after the retry budget for files held by other processes).
    #[error("rtk I/O error: {0}")]
    Io(#[from] io::Error),
    /// Storing the raw stream in the CAS failed.
    #[error("rtk could not store the raw stream: {0}")]
    Cas(#[from] CasError),
    /// A structured report could not be serialised.
    #[error("rtk JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// The piped stream is larger than [`crate::MAX_STREAM_BYTES`].
    #[error("stream of {size} bytes exceeds the {limit}-byte limit")]
    StreamTooLarge {
        /// Size of the rejected stream.
        size: usize,
        /// The configured limit.
        limit: usize,
    },
    /// The ledger file changed between reading it and replacing it.
    #[error("the ledger was changed by another writer while it was being updated")]
    ConcurrentWrite,
    /// The advisory ledger lock stayed busy for the whole waiting budget.
    #[error("timed out waiting for the ledger lock")]
    LockTimeout,
}

impl HasKind for RtkError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io(_) | Self::ConcurrentWrite | Self::LockTimeout => ErrorKind::Transient,
            Self::Cas(inner) => inner.kind(),
            Self::Json(_) | Self::StreamTooLarge { .. } => ErrorKind::Permanent,
        }
    }
}
