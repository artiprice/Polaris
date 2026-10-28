//! Hybrid mutation authorization, exact metrics, semantic safety assessment, and evidence-backed
//! promotion of verified SDK commands.
//!
//! The request's byte snapshots and resolved workspace identity are supplied by the trusted
//! workspace owner. It must resolve symlinks and prove canonical containment before calling this
//! crate. This crate never opens or mutates source files, so callers must also hold a lease or use
//! an immutable snapshot to prevent a check/use race between these bytes and the eventual write.
//! The eventual mutation executor owns that final compare-and-swap check.

mod binding;
mod evidence;
mod execution;
mod frame_cancellation;
mod human;
mod local;
mod verify;

pub use binding::{MutationBindingError, mutation_digest};
pub use evidence::{EvidenceError, EvidenceStore};
pub use execution::LocalExecutionVerifier;
pub use frame_cancellation::{
    FrameCancellationEvidence, FrameCancellationVerifier, FrameCancellationVerifierError,
    load_cancellation_evidence, validate_cancellation_evidence,
};
pub use human::{HumanRequestVerifier, HumanRequestVerifierError};
pub use local::{LocalDirectiveVerifier, LocalVerificationError};
pub use verify::{
    ApprovalReason, ApprovedMutation, HybridMutationVerifier, InputField, MAX_DIFF_BYTES,
    MAX_EVIDENCE_BYTES, MAX_INTENT_BYTES, MAX_SOURCE_BYTES, MAX_TARGET_BYTES, MAX_TOKEN_BYTES,
    Metric, MutationRequest, Rejection, VerificationDecision, VerifierConfig, VerifierError,
};
