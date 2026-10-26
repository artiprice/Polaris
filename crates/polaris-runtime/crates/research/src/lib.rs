//! Bounded research decisions and conservative recovery accounting.
//!
//! This crate implements the deterministic experiment state machine. The caller owns measurement,
//! sandbox execution, durable checkpoint storage, and actual workspace mutations. A decision is
//! advisory until [`ResearchState::settle`] records the mutation receipt. Restoring an in-flight
//! checkpoint requires an explicit workspace recovery receipt before another experiment can start.

mod r#loop;
mod metric;
mod program;
mod state;
mod store;
mod workspace;

pub use r#loop::{
    FailureReason, IterationStatus, LoopError, ModelUsage, ResearchEdit, ResearchLoop,
    ResearchStatus, RunStop, RunSummary, ScopedToolCall, StepReport, StopCondition, ToolContext,
};
pub use metric::{MetricParseError, MetricParser};
pub use program::{MetricSpec, ProgramError, ProgramMd};
pub use store::{CheckpointStore, LedgerEntry, LedgerKind, StoreError, StoredCheckpoint};

pub use state::{
    Budget, Checkpoint, Decision, Direction, ExperimentId, MetricValue, Observation, ResearchError,
    ResearchState, Usage,
};
pub use workspace::{ExperimentWorkspace, VcsWorkspace, WorkspaceError, WorkspaceReceipt};
