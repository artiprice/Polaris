//! # polaris-runtime-guards
//!
//! Deterministic blast-radius estimates and a completion proof gate. This crate does not execute
//! commands or access the workspace directly: callers inject workspace, sandbox, RTK, CAS and
//! verifier facts through typed inputs. Unknown command shapes are critical; incomplete or stale
//! evidence cannot produce `Completion<Proven>`.

mod ast_blast;
mod blast;
mod completion;
mod error;
mod proof;

pub use ast_blast::{
    AstBlastReport, AstChange, AstImpact, AstImpactLimits, CallSiteFacts, ReferenceConfidence,
    SymbolReferenceView, ast_blast_radius,
};
pub use blast::{
    BlastInput, BlastLimits, BlastReport, Confidence, EffectKind, FileFact, GlobPattern,
    GuardVerdict, Severity, ShellKind, WorkspacePath, WorkspaceView, blast_radius,
};
pub use completion::{Completion, Pending, Proven, ProofRecord};
pub use error::{GuardError, ViewError};
pub use proof::{
    IndependentCheck, IndependentProof, ParseStatus, ProofFailure, ProofPolicy, RunEvidence,
    TamperReport, TestRunner, TestSummary, proof_of_done,
};
