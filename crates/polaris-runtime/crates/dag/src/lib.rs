//! Bounded DAG execution with named semaphore pools and first-valid speculative races.
//!
//! Operators are injected implementations; this crate has no fake tool or model backend. A run
//! tracks each node as ready, started, and completed, and aborts/drains child tasks when canceled.

mod graph;
mod pools;
mod race;

pub use graph::{
    Dag, DagBuilder, DagLimits, DagReport, DagRunStatus, FunctionalOperator, MAX_DAG_EDGES,
    MAX_DAG_NODES, NodeCtx, NodeError, NodeId, NodeResult, NodeTask, OnFailure, RunOptions, run,
};
pub use pools::{Pool, PoolCounters, PoolLimits, PoolPermit, PoolStats, Pools};
pub use race::{Accept, Branch, MAX_RACE_BRANCHES, RaceOutcome, race};

/// Scheduler and graph errors. Node failures remain in the per-node report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DagError {
    /// A graph references an id that was never added.
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    /// An id was added more than once.
    #[error("duplicate node {0}")]
    DuplicateNode(NodeId),
    /// An edge was added more than once.
    #[error("duplicate dependency edge")]
    DuplicateEdge,
    /// The validated graph contains a cycle.
    #[error("task graph contains a cycle")]
    Cycle,
    /// The configured graph or concurrency bounds are invalid.
    #[error("invalid DAG limits")]
    InvalidLimits,
    /// A semaphore has been closed.
    #[error("pool is closed")]
    Closed,
    /// The caller canceled the run or race.
    #[error("DAG execution was canceled")]
    Cancelled,
    /// A pool cannot be resized while it has active or queued work.
    #[error("pool is busy")]
    PoolBusy,
    /// A required pool permit counter overflowed.
    #[error("pool counter overflow")]
    CounterOverflow,
    /// A scheduler task ended outside its structured result path.
    #[error("scheduler task join failed")]
    JoinFailed,
    /// Internal graph indices violated the validated graph invariant.
    #[error("validated graph index invariant failed")]
    GraphInvariant,
}

impl polaris_sdk::error::HasKind for DagError {
    fn kind(&self) -> polaris_sdk::error::ErrorKind {
        use polaris_sdk::error::ErrorKind;
        match self {
            Self::Closed | Self::Cancelled | Self::PoolBusy => ErrorKind::Transient,
            Self::UnknownNode(_)
            | Self::DuplicateNode(_)
            | Self::DuplicateEdge
            | Self::Cycle
            | Self::InvalidLimits => ErrorKind::Policy,
            Self::CounterOverflow | Self::JoinFailed | Self::GraphInvariant => ErrorKind::Fatal,
        }
    }
}
