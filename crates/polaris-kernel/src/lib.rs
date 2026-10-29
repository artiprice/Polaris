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
//! - `worker_lifecycle` (authenticated S1 worker bootstrap and connection lifecycle)
//!
//! ## Responsibilities
//!
//! - Instantiate concrete backends (enum dispatch for runtime-selected ones) and inject traits into domain crates.
//! - Reach document ingest and S1 inference only through worker processes (sdk DocParser/DecisionEngine traits over polaris-proto); never link polaris-ingest or polaris-system-one-engine.
//! - Model sessions as typestate (Created -> Configured -> Running -> Paused -> Archived).
//! - Run session and registry projections from the event log; UI consumers live above L5.
//!
//! All seven child modules are available through this facade:
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
//! - `S1WorkerLifecycle<P>` and `ReadyS1Worker` for supervised S1 process integration
//! - `KernelPromptAssembler<M>::run_frame` for actual model/tokenizer identity, masking, bounded
//!   context fitting, authorized archive retrieval and the existing durable frame/tool loop
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
pub mod s1_host;
pub mod worker_lifecycle;

mod agent_tools;
mod configured_frame;
mod configured_tools;
mod context;
pub mod docker_pool;
mod error;
mod evidence;
mod execution;
mod frame_cancellation;
mod frame_journal;
mod frame_mutation;
mod frame_prompt;
mod frame_tools;
mod frame_verification;
mod host_control;
pub mod human_requests;
pub use host_control::{HostControlError, serve_host_control, serve_host_control_directory};
pub mod model_lifecycle;
pub mod privacy_snapshot;
mod private_frame_checkpoint;
pub use private_frame_checkpoint::{PrivateCheckpointError, PrivateFrameCheckpointStore};
mod prompt_privacy;
pub use prompt_privacy::{SemanticPrivacyLimits, SemanticPrivacyReport};
mod runtime;
mod sandbox_recovery;
pub mod session;
mod skill_status;
mod tools;

pub use agent_tools::{AgentFramePermit, KernelAgentToolRunner};
pub use configured_frame::{
    ConfiguredFrameError, ConfiguredFrameOutcome, ConfiguredFramePolicy, ConfiguredFrameServices,
    ConfiguredWatchdogPolicy, FrameCompletionCheck, SystemOneCoverage,
};
pub use configured_tools::{
    ConfiguredAgentToolRunner, ConfiguredDecisionEngine, ConfiguredMutationSafety,
    ConfiguredToolError,
};
pub use context::KernelCtx;
pub use error::KernelError;
pub use evidence::KernelEvidence;
pub use execution::{ExecutionRequest, ExecutionReview, SandboxExecution};
pub use frame_journal::{KernelFrameJournal, KernelFrameJournalError};
pub use frame_mutation::{
    FrameMutationConfig, FrameMutationError, FrameMutationReceipt, FrameMutationRequest,
    FrameMutationService, PreparedFrameMutation,
};
pub use frame_prompt::{
    KernelPromptAssembler, KernelPromptError, PromptAssemblySettings, PromptContext, PromptLimits,
};
pub use frame_tools::{AuthorizedFrameTool, FrameToolAccess, KernelToolRunner};
pub use human_requests::{
    HumanRequestError, HumanRequestOutcome, HumanRequestService, HumanRequestSnapshot,
    HumanRequestTicket,
};
pub use runtime::{Kernel, KernelEvents, KernelHandle, LocalBackends, LocalKernelConfig};
pub use s1_host::{LocalS1Config, LocalS1Host, S1Host, S1Normalization};
pub use session::{Archived, Configured, Created, Paused, Running, Session, SessionConfig};
pub use skill_status::{
    SkillDiscoveryStatus, SkillEnablementStatus, SkillRequirementStatus, SkillStatusSnapshot,
    SkillToolRegistryScope, inspect_skill_status,
};
pub use tools::{KernelRecall, LocalTools};
pub use verifier::{HumanRequestVerifier, HumanRequestVerifierError};
pub use worker_lifecycle::{CurrentS1Engine, CurrentS1PiiDetector};
pub use worker_lifecycle::{ReadyS1Worker, S1ModelPause, S1WorkerLifecycle};

pub use frame_cancellation::{
    FrameCancellationError, FrameCancellationOutcome, FrameCancellationRegistration,
    FrameCancellationService, MAX_REGISTERED_FRAME_DEPTH, MAX_REGISTERED_FRAMES_PER_SESSION,
};
pub use frame_verification::{
    FrameVerificationError, KernelFrameVerifier, VerifiedFrameCompletion, VerifiedFrameStep,
};
