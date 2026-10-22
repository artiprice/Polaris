//! Local, opt-in trajectory collection and masked JSONL export.
//!
//! Only independently proved trajectories can transition to the masked export state. Consent is
//! session-scoped and revocable, all exported strings receive a second signature/entropy scan,
//! and referenced training artifacts are masked copies in CAS. This crate contains no remote
//! Data Lab endpoint or fine-tuning simulation. New implementation is unverified.

mod consent;
mod error;
mod export;
mod trajectory;

pub use consent::{ConsentRecord, OptIn};
pub use error::DataLabError;
pub use export::{ExportConfig, ExportFormat, ExportPreview, ExportReceipt, TrajectoryExporter};
pub use trajectory::{
    Draft, Masked, MessageInput, MessageRole, ProofFacts, ProofResolver, ProofStatus, Provenance,
    RawContent, ResidualGuard, ToolCallInput, Trajectory, TrajectoryInput, TrajectoryLimits,
    Verified,
};
