//! Durable content-store contract shared by runtime and storage adapters.

use bytes::Bytes;

use crate::{
    error::{ErrorKind, HasKind},
    ids::Cid,
};

/// Bounded content-addressed storage used for event payloads and frame transcripts.
pub trait ContentStore: Send + Sync {
    /// Persist a complete blob and return its BLAKE3 content identifier.
    ///
    /// # Errors
    /// [`ContentStoreError`] when the store is unavailable, the blob is too large, or storage
    /// integrity fails.
    fn put(&self, content: &[u8]) -> Result<Cid, ContentStoreError>;

    /// Check whether an exact content identifier is durably present without loading its payload.
    ///
    /// # Errors
    /// [`ContentStoreError`] when the backend cannot answer reliably.
    fn contains(&self, cid: &Cid) -> Result<bool, ContentStoreError>;

    /// Read a blob only when its decoded size is at most `max_bytes`.
    ///
    /// # Errors
    /// [`ContentStoreError`] when the blob is missing, corrupt, over the requested bound, or the
    /// store is unavailable.
    fn get(&self, cid: &Cid, max_bytes: u64) -> Result<Bytes, ContentStoreError>;
}

/// Sanitized content-store failures; no payload bytes are included.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentStoreError {
    /// The backend is temporarily unavailable.
    #[error("content store unavailable")]
    Unavailable,
    /// The requested content id has no stored blob.
    #[error("content blob not found: {0}")]
    NotFound(Cid),
    /// A stored blob failed its content-integrity check.
    #[error("content blob is corrupt: {0}")]
    Corrupt(Cid),
    /// A blob is over a configured bound.
    #[error("content blob exceeds size limit ({size} > {limit})")]
    TooLarge {
        /// Maximum permitted decoded byte count.
        limit: u64,
        /// Actual or declared decoded byte count.
        size: u64,
    },
    /// A stored blob could not be decoded.
    #[error("content blob encoding is invalid")]
    InvalidEncoding,
}

impl HasKind for ContentStoreError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable => ErrorKind::Transient,
            Self::NotFound(_) | Self::TooLarge { .. } => ErrorKind::Permanent,
            Self::Corrupt(_) | Self::InvalidEncoding => ErrorKind::Fatal,
        }
    }
}
