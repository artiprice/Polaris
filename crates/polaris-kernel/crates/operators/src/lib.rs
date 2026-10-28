//! Typed composition primitives for kernel request operators.
//!
//! The crate provides generic control flow and audit records. Concrete stages stay in their owning
//! crates; the composition root must inject real operators for every stage it claims to execute.
mod pipeline;
mod policy;
mod rate_limit;
mod sanitize;
mod triage;

pub use pipeline::{
    FoldOperator, Guard, IncompleteRequest, MAX_FOLD_ITEMS, MAX_STAGE_RECORDS, Pipeline,
    PipelineError, RequestStage, StageRecord, StageStatus,
};
pub use policy::{
    ClosedOp, ClosedOpError, FailureOpenObserver, FailureOpenReason, Fallback, OpenOp, OpenOpError,
    RaceOp, RetryOp, RetryOpError,
};
pub use rate_limit::{
    RateLimitInput, RateLimitOp, RateLimitOpConfigError, RateLimitOpError, RateLimited,
};
pub use sanitize::{
    MAX_SANITIZE_BYTES, MaskedText, SanitizeConfigError, SanitizeError, SanitizePiiOp,
};

pub use triage::{S1TriageOp, TriageError, TriageInput};
