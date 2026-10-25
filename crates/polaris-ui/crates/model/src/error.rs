//! Errors of the composer and the reducer. Both implement [`HasKind`]: every failure here is a
//! rejected input, so retrying the same input fails the same way ([`ErrorKind::Permanent`]).

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::Seq;

/// A composer operation was refused; the draft is left exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposerError {
    /// The draft (including the payload of every paste chip) would exceed
    /// [`MAX_TEXT_BYTES`](crate::composer::MAX_TEXT_BYTES).
    #[error("draft would be {attempted} bytes, the limit is {limit}")]
    TextTooLarge {
        /// The limit in bytes.
        limit: usize,
        /// The size the draft would have reached.
        attempted: usize,
    },
    /// One image is larger than [`MAX_IMAGE_RGBA`](crate::composer::MAX_IMAGE_RGBA).
    #[error("image is {len} bytes, the limit is {limit}")]
    ImageTooLarge {
        /// Size of the rejected image in bytes.
        len: usize,
        /// The per-image limit in bytes.
        limit: usize,
    },
    /// An image of size zero cannot be attached.
    #[error("image has no pixel data")]
    ImageEmpty,
    /// All attached images together would exceed
    /// [`MAX_ATTACHMENTS_TOTAL`](crate::composer::MAX_ATTACHMENTS_TOTAL).
    #[error("attachments would total {total} bytes, the limit is {limit}")]
    AttachmentsTooLarge {
        /// Total image bytes the draft would hold.
        total: usize,
        /// The session limit in bytes.
        limit: usize,
    },
    /// There is nothing to submit: the draft is blank and has no attachment.
    #[error("nothing to submit")]
    EmptySubmission,
    /// A prepared send no longer matches the composer revision.
    #[error("prepared submission is stale")]
    StalePreparedSubmission,
    /// No text chip with this id exists in the draft.
    #[error("no such paste chip")]
    NoSuchChip,
    /// A byte range is out of bounds, reversed, off a char boundary or cuts through a chip.
    #[error("invalid text range")]
    InvalidRange,
}

impl HasKind for ComposerError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}

/// A category of bounded event-derived metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionResource {
    SessionViews,
    SessionRows,
    ToolActivities,
    ToolMetadataBytes,
}

/// A resolved text append was refused without changing the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    #[error("conversation entry limit reached ({limit})")]
    ConversationEntries { limit: usize },
    #[error("conversation byte limit reached ({limit})")]
    ConversationBytes { limit: usize },
    #[error("tool metadata byte limit reached ({limit})")]
    ToolMetadataBytes { limit: usize },
    #[error("view byte accounting overflowed")]
    AccountingOverflow,
    #[error("view storage allocation failed")]
    AllocationFailed,
}
impl HasKind for ViewError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}
/// An event record could not be folded into the view state; the state is left unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReduceError {
    /// A record was skipped: the view expected `expected` but received `got`. The caller
    /// should replay the log from `expected` (or rebuild from a snapshot).
    #[error("sequence gap: expected {expected:?}, got {got:?}")]
    SeqGap {
        /// The next sequence number the view needs.
        expected: Seq,
        /// The sequence number that arrived.
        got: Seq,
    },
    /// A record arrived twice or out of order (its sequence number is not after `last`).
    #[error("out of order: already at {last:?}, got {got:?}")]
    OutOfOrder {
        /// The newest sequence number folded so far.
        last: Seq,
        /// The sequence number that arrived.
        got: Seq,
    },
    /// Applying the event would exceed a bounded projection.
    #[error("{resource:?} capacity exceeded (limit {limit})")]
    CapacityExceeded {
        resource: ProjectionResource,
        limit: usize,
    },
    /// A bounded projection allocation could not be reserved.
    #[error("projection allocation failed")]
    AllocationFailed,
}

impl HasKind for ReduceError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}
