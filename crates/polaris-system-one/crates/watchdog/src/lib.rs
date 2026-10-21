//! Advisory monitoring for agent steps: deterministic role checks, repetition detection, context
//! pressure, then schema-backed S1 goal-drift judgment. This crate never authorizes tool access;
//! capability checks remain the execution boundary.

mod inspect;
mod loops;
mod role;
mod types;

pub use inspect::{Watchdog, WatchdogError};
pub use loops::{LoopWindow, StepBuildError, build_step_record};
pub use role::{RolePolicy, RoleTable, RoleTableError, ToolCallFacts, ToolId};
pub use types::{
    FatigueConfig, FrameSnapshot, HealthVerdict, LoopConfig, LoopMatch, ProposedStep,
    RatioThreshold, ShingleSet, ShingleSimilarity, StepFingerprint, StepIndex, StepOutcome,
    StepRecord, UncheckedReason, WatchdogConfig,
};
