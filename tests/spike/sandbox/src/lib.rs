//! # polaris-spike-sandbox
//!
//! Spike tests for sandboxes: concurrent create/exec/destroy bursts, git-checkout file-event floods during sync.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L8 — tests, spikes & tooling |
//! | Kind | spike |
//! | Phase | P2 |
//! | Parent facade | — |
//! | Status | lifecycle driver implemented; live acceptance must be explicitly run |
//!
//! ## Responsibilities
//!
//! - Assert semaphore limits, reaper cleanup and sync consistency under bursts (requires WSL2 gateway).
//!
//! ## Internal dependencies
//!
//! `polaris-openshell`
//!
//! ## Planned external dependencies
//!
//! `tokio`

mod burst;
pub use burst::{BurstLimits, BurstReport, CycleFailure, SpikeError, run_sandboxes};
