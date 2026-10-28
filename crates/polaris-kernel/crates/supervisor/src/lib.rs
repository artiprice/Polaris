//! # polaris-kernel-supervisor
//!
//! One-for-one Tokio task supervision with bounded restart intensity, circuit breakers,
//! observable dead letters, explicit joined shutdown, and native worker supervision through the
//! SDK `PlatformOps` contract.

mod breaker;
mod dlq;
mod intensity;
mod task;
mod worker;

pub use breaker::{BreakerConfig, BreakerHandle, BreakerState, CircuitBreaker};
pub use dlq::{DeadLetter, DeadLetterQueue};
pub use intensity::{RestartIntensity, RestartVerdict, RestartWindow};
pub use task::{
    BackoffSpec, BreakerSnapshot, ChildId, ChildName, ChildSnapshot, ChildStatus, ExitClass,
    OnExhausted, RestartPolicy, StopOutcome, Supervisor, SupervisorConfig, SupervisorError,
    SupervisorSnapshot, TaskError, TaskSpec,
};
pub use worker::{HealthPolicy, HealthProbe, WorkerLifecycle, WorkerSpec};
