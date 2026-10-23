//! # polaris-kernel
//!
//! Composition root: wires Backends, session typestate, orchestrator, projections; re-exports kernel children.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L5 — composition root & RPC |
//! | Kind | facade |
//! | Phase | P1 |
//! | Parent facade | — |
//! | Status | P1 local session, command, tool and shutdown composition implemented |
//!
//! ## Children
//!
//! - `polaris-kernel-bus`
//! - `polaris-kernel-directives`
//! - `polaris-kernel-supervisor`
//! - `polaris-kernel-reaper`
//! - `polaris-kernel-operators`
//! - `polaris-kernel-verifier`
//!
//! ## Responsibilities
//!
//! - Instantiate concrete backends (enum dispatch for runtime-selected ones) and inject traits into domain crates.
//! - Reach document ingest and S1 inference only through worker processes (sdk DocParser/DecisionEngine traits over polaris-proto); never link polaris-ingest or polaris-system-one-engine.
//! - Model sessions as typestate (Created -> Configured -> Running -> Paused -> Archived).
//! - Run session and registry projections from the event log; UI consumers live above L5.
//!
//! All six child modules are available through this facade:
//! ```
//! use polaris_kernel::{bus::BusConfig, directives::ParseLimits, supervisor::SupervisorConfig,
//!     reaper::ShutdownConfig, operators::RequestStage, verifier::VerifierConfig};
//! let _configs = (BusConfig::default(), ParseLimits::default(), SupervisorConfig::default(),
//!     ShutdownConfig::default(), VerifierConfig::default());
//! let _stage = RequestStage::Directives;
//! ```
//!
//! ## API
//!
//! - `struct Kernel` with the concrete P1 `LocalBackends` profile
//! - `fn start(config) -> KernelHandle`
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-proto`, `polaris-config`, `polaris-log`, `polaris-telemetry`, `polaris-ratelimit`, `polaris-kernel-bus`, `polaris-kernel-directives`, `polaris-kernel-supervisor`, `polaris-kernel-reaper`, `polaris-kernel-operators`, `polaris-kernel-verifier`, `polaris-storage`, `polaris-security`, `polaris-platform`, `polaris-vcs`, `polaris-openshell`, `polaris-nebius-cloud`, `polaris-system-one`, `polaris-models`, `polaris-tools`, `polaris-search`, `polaris-ext`, `polaris-nvidia-docs`, `polaris-browser`, `polaris-devenv`, `polaris-hacker`, `polaris-runtime`
//!
//! ## Planned external dependencies
//!
//! `tokio`, `tracing`

pub use polaris_kernel_bus as bus;
pub use polaris_kernel_directives as directives;
pub use polaris_kernel_operators as operators;
pub use polaris_kernel_reaper as reaper;
pub use polaris_kernel_supervisor as supervisor;
pub use polaris_kernel_verifier as verifier;
pub use polaris_storage as storage;

mod context;
pub mod docker_pool;
mod error;
mod evidence;
mod execution;
mod runtime;
mod sandbox_recovery;
pub mod session;
mod skill_status;
mod tools;

pub use context::KernelCtx;
pub use error::KernelError;
pub use evidence::KernelEvidence;
pub use execution::{ExecutionRequest, ExecutionReview, SandboxExecution};
pub use runtime::{Kernel, KernelEvents, KernelHandle, LocalBackends, LocalKernelConfig};
pub use session::{Archived, Configured, Created, Paused, Running, Session, SessionConfig};
pub use skill_status::{
    SkillDiscoveryStatus, SkillEnablementStatus, SkillRequirementStatus, SkillStatusSnapshot,
    SkillToolRegistryScope, inspect_skill_status,
};
pub use tools::{KernelRecall, LocalTools};
