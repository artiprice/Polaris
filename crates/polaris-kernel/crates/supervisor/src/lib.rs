//! # polaris-kernel-supervisor
//!
//! One-for-one Tokio task supervision with bounded restart intensity, circuit breakers,
//! observable dead letters, and explicit joined shutdown. Native worker processes require
//! the platform process adapter and are not implemented by this task supervisor.

mod breaker;
mod dlq;
mod intensity;
mod task;

pub use breaker::{BreakerConfig, BreakerHandle, BreakerState, CircuitBreaker};
pub use dlq::{DeadLetter, DeadLetterQueue};
pub use intensity::{RestartIntensity, RestartVerdict, RestartWindow};
pub use task::{
    BackoffSpec, BreakerSnapshot, ChildId, ChildName, ChildSnapshot, ChildStatus, ExitClass,
    OnExhausted, RestartPolicy, StopOutcome, Supervisor, SupervisorConfig, SupervisorError,
    SupervisorSnapshot, TaskError, TaskSpec,
};
