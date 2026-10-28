//! Lossless context virtualization with a stable prefix, mutable epistemic scratchpad, sliding
//! conversation window, and byte-exact CAS folding. CAS reads and writes use the storage crate's
//! synchronous API; callers should keep this blocking work off async executor threads.

mod assemble;
mod error;
mod fold;
mod prefix;
mod scratchpad;
mod window;

pub use assemble::{
    AssembleRequest, AssembledPrompt, AssemblyStats, ByteHeuristic, ContextConfig,
    ContextMapReport, ContextVirtualizer, PrefixStability, TokenEstimate, TokenEstimator,
    WindowArchiveRef, archive_window, window_omission_notice,
};
pub use error::ContextError;
pub use fold::{ByteRange, FileRead, FoldConfig, FoldKind, FoldRequest, Folded, ToolKind};
pub use prefix::{PrefixHash, StablePrefix, ToolProfile, serialize_prefix};
pub use scratchpad::{Entry, EntryKind, EpistemicScratchpad, ScratchpadConfig};
pub use window::ConversationWindow;
