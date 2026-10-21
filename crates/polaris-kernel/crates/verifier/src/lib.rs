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
mod verify;

pub use binding::{MutationBindingError, mutation_digest};
pub use evidence::{EvidenceError, EvidenceStore};
pub use verify::{
    ApprovedMutation, ApprovalReason, HybridMutationVerifier, InputField, Metric, MutationRequest,
    Rejection, VerificationDecision, VerifierConfig, VerifierError, MAX_DIFF_BYTES,
    MAX_EVIDENCE_BYTES, MAX_INTENT_BYTES, MAX_SOURCE_BYTES, MAX_TARGET_BYTES, MAX_TOKEN_BYTES,
};
