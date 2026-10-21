//! CAS errors.

use std::io;

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{Cid, ShortRef};

/// Blob store failures.
#[derive(Debug, thiserror::Error)]
pub enum CasError {
    /// Filesystem failure (after the retry budget for in-use files).
    #[error("cas I/O error: {0}")]
    Io(#[from] io::Error),
    /// No blob with this id.
    #[error("blob {0} not found")]
    NotFound(Cid),
    /// No blob starts with this short reference.
    #[error("no blob matches §ref:{0}")]
    UnknownShortRef(ShortRef),
    /// The stored bytes do not hash to their id.
    #[error("blob {cid} is corrupt")]
    Corrupt {
        /// The affected blob.
        cid: Cid,
    },
    /// Several blobs share this short reference.
    #[error("short reference §ref:{0} is ambiguous")]
    Ambiguous(ShortRef),
    /// Another blob shares this blob's short reference; fold with the full id instead.
    #[error("short reference §ref:{0} collides with another blob")]
    ShortRefCollision(ShortRef),
    /// Over the configured size limit.
    #[error("blob of {size} bytes exceeds the {limit}-byte limit")]
    TooLarge {
        /// Configured limit.
        limit: u64,
        /// Actual or declared size.
        size: u64,
    },
    /// zstd failed (invalid, truncated or undeclared-size frame).
    #[error("zstd: {0}")]
    Decompress(String),
    /// The blob header is invalid.
    #[error("invalid blob format: {0}")]
    Format(&'static str),
}

impl HasKind for CasError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io(_) => ErrorKind::Transient,
            Self::NotFound(_)
            | Self::UnknownShortRef(_)
            | Self::Ambiguous(_)
            | Self::ShortRefCollision(_)
            | Self::TooLarge { .. } => ErrorKind::Permanent,
            Self::Corrupt { .. } | Self::Decompress(_) | Self::Format(_) => ErrorKind::Fatal,
        }
    }
}
