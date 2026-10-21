//! Context frames for bounded, delegated agent work.
//!
//! Frames share model and tool backends. This crate owns only frame-local state, capability
//! attenuation, quota carving, and the authorization-before-execution contract. The composition
//! root supplies backend implementations; no frame method writes workspace files directly.

mod scope;
mod quota;
mod state;
mod step;

pub use polaris_sdk::ids::{AgentRole, Cid, FastFingerprint, FrameId, ModelId, TokenCount};
pub use polaris_security_capability::Scope;
pub use scope::{scope_is_subset, FrameScope, ScopeError};
pub use quota::{NetworkPolicy, QuotaError, QuotaLease, ResourceQuota};
pub use state::{Active, AgentContextFrame, Finished, FrameError, FrameOutcome, FrameStack, FrameView, DiffTimelineEntry, ModelSelection, ScratchpadRef, Spawned, StepNo, Suspended};
pub use step::{AssembleRequest, Cancellation, ExecutedToolResult, FramePrompt, StepInput, StepOutcome, StepTool, ToolCall, ToolResultRef, ToolRunner, StepVerifier, Verification, run_step};
