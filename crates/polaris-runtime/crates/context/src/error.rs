//! Error classification for context assembly and CAS folding.

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_storage_cas::CasError;
use polaris_tools_rtk::RtkError;

/// Failure while assembling, folding, or expanding context.
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    /// The backing content store failed.
    #[error(transparent)]
    Store(#[from] CasError),
    /// A compiler diagnostic stream could not be interpreted by RTK.
    #[error(transparent)]
    Rtk(#[from] RtkError),
    /// The requested prompt cannot fit while preserving its immutable prefix and scratchpad.
    #[error("context budget cannot fit immutable prefix and scratchpad")]
    Budget,
    /// The caller supplied an invalid byte range.
    #[error("byte range is outside the referenced content")]
    InvalidRange,
    /// The prefix could not be represented within the stable serialization format.
    #[error("stable prefix contains a field too large to serialize")]
    PrefixTooLarge,
    /// A stable prefix repeated a tool name.
    #[error("stable prefix contains duplicate tool names")]
    DuplicateToolName,
    /// Token estimate arithmetic overflowed its public counter.
    #[error("token estimate exceeds supported range")]
    EstimateOverflow,
    /// Conversation history contains malformed or incomplete structured tool-call metadata.
    #[error("conversation history contains malformed or incomplete tool-call metadata")]
    ToolHistory,
    /// Scratchpad turn number moved backwards.
    #[error("scratchpad entries must be appended in nondecreasing turn order")]
    TurnOrder,
    /// A CAS value expected to contain UTF-8 text did not.
    #[error("stored context entry is not valid UTF-8")]
    InvalidUtf8,
    /// A context state lock was poisoned by a prior panic.
    #[error("context state lock is poisoned")]
    StatePoisoned,
}

impl HasKind for ContextError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Store(error) => error.kind(),
            Self::Rtk(_) | Self::InvalidRange | Self::TurnOrder | Self::PrefixTooLarge
            | Self::DuplicateToolName | Self::InvalidUtf8 | Self::ToolHistory => {
                ErrorKind::Permanent
            }
            Self::Budget | Self::EstimateOverflow => ErrorKind::Permanent,
            Self::StatePoisoned => ErrorKind::Fatal,
        }
    }
}
