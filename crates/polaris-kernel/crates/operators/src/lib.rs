//! Typed composition primitives for kernel request operators.
//!
//! The crate provides generic control flow and audit records. Concrete stages stay in their owning
//! crates; the composition root must inject real operators for every stage it claims to execute.
mod pipeline;
mod policy;
mod sanitize;
mod triage;

pub use pipeline::{
    FoldOperator, Guard, IncompleteRequest, Pipeline, PipelineError, RequestStage, StageRecord,
    StageStatus, MAX_FOLD_ITEMS, MAX_STAGE_RECORDS,
};
pub use policy::{
    ClosedOp, ClosedOpError, FailureOpenObserver, FailureOpenReason, Fallback, OpenOp,
    OpenOpError, RaceOp, RetryOp, RetryOpError,
};
pub use sanitize::{MaskedText, MAX_SANITIZE_BYTES, SanitizeConfigError, SanitizeError, SanitizePiiOp};



pub use triage::{S1TriageOp, TriageError, TriageInput};
