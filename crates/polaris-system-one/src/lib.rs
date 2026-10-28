//! Kernel-side System-One schema, routing, verification and watchdog boundaries.
//! Native inference belongs in worker processes and is injected through SDK traits.
//! Worker IPC, semantic spans and explicit consumer failure policy have focused fixture evidence.
//! Live model parity, labelled calibration and complete phase acceptance remain separate gates.

pub use polaris_system_one_client as client;
pub use polaris_system_one_router as router;
pub use polaris_system_one_schemas as schemas;
pub use polaris_system_one_verifier as verifier;
pub use polaris_system_one_watchdog as watchdog;

/// Consumer failure policy, bounded admission and worker epoch fencing.
pub mod policy;
