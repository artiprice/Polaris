//! Bounded research decisions and conservative recovery accounting.
//!
//! This crate implements the deterministic experiment state machine. The caller owns measurement,
//! sandbox execution, durable checkpoint storage, and actual workspace mutations. A decision is
//! advisory until [`ResearchState::settle`] records the mutation receipt. Restoring an in-flight
//! checkpoint requires an explicit workspace recovery receipt before another experiment can start.

mod metric;
mod state;

pub use metric::{MetricParseError, MetricParser};

pub use state::{
    Budget, Checkpoint, Decision, Direction, ExperimentId, MetricValue, Observation, ResearchError,
    ResearchState, Usage,
};
