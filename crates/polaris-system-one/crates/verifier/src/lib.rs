//! Deterministic-first command safety, mutation preflight, proof assessment and merge arbitration.
//!
//! No function executes commands or writes workspace files. Missing analysis, uncalibrated
//! schemas and incomplete proof evidence require approval. New production implementation is
//! unverified; the user has deferred validation.

mod arbiter;
mod diagnostics;
mod error;
mod gate;
mod postflight;
mod preflight;
mod scan;
mod semantic;
mod types;

pub use diagnostics::{Diagnostic, DiagnosticId, DiagnosticSeverity, RankedDiagnostic, rank_diagnostics};
pub use error::VerifierError;
pub use gate::{CommandFacts, ShellDialect};
pub use postflight::{FalsifierEvidence, MutationEvidence, ProofEvidence, ProofVerdict, TestRunFacts};
pub use scan::{SafetyScanner, Scanner};
pub use semantic::{CalibrationBindings, Verifier, VerifierConfig};
pub use types::{DegradedReason, Finding, GateVerdict, MergeDecision, MutationSafety,
    PostflightReport, PreflightReport, SafetyAssessment, SafetyRequest, SourceLocation, TamperKind};
