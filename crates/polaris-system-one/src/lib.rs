//! Kernel-side System-One schema, routing, verification and watchdog boundaries.
//! Native inference belongs in worker processes and is injected through SDK traits.
//! New production APIs are unverified; concrete worker assembly and calibration remain pending.

pub use polaris_system_one_client as client;
pub use polaris_system_one_router as router;
pub use polaris_system_one_schemas as schemas;
pub use polaris_system_one_verifier as verifier;
pub use polaris_system_one_watchdog as watchdog;
