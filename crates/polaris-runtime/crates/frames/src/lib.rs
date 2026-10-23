//! Context frames for bounded, delegated agent work.
//!
//! Frames share model and tool backends. This crate owns only frame-local state, capability
//! attenuation, quota carving, and the authorization-before-execution contract. The composition
//! root supplies backend implementations; no frame method writes workspace files directly.

mod journal;
mod quota;
mod scope;
mod state;
mod step;

pub use journal::{FrameJournal, FrameJournalError, FrameStepRecord, JournalAdapter};
pub use polaris_sdk::ids::{AgentRole, Cid, FastFingerprint, FrameId, ModelId, TokenCount};
pub use polaris_security_capability::Scope;
pub use quota::{NetworkPolicy, QuotaError, QuotaLease, ResourceQuota};
pub use scope::{FrameScope, ScopeError, scope_is_subset};
pub use state::{
    Active, AgentContextFrame, DiffTimelineEntry, Finished, FrameError, FrameOutcome, FrameStack,
    FrameView, ModelSelection, ScratchpadRef, Spawned, StepNo, Suspended,
};
pub use step::{
    AssembleRequest, Cancellation, ExecutedToolResult, FramePrompt, FrameRunLimits,
    FrameRunOutcome, FrameRunStop, StepExecution, StepInput, StepOutcome, StepTool, StepVerifier,
    ToolCall, ToolResultRef, ToolRunner, Verification, run_frame, run_step,
};
