//! # polaris-kernel-reaper
//!
//! Deterministic, deadline-bounded teardown with durable recovery locators.
//!
//! Call [`Reaper::shutdown_all`] explicitly: Drop can cancel and release local owned handles,
//! but cannot await remote resources. PID-only orphan records are reported for operator review,
//! never killed: the v1 journal cannot prove process start-time identity after PID reuse.

mod journal;
mod resources;
mod shutdown;
mod types;

pub use shutdown::Reaper;
pub use types::{
    Disposition, JOURNAL_FILE_NAME, LeakEntry, LeakReport, Locator, OrphanAction, OrphanReport,
    Outcome, ReapId, ReaperError, ResourceKind, ShutdownConfig, ShutdownEntry, ShutdownReport,
    SweepReport, TEMP_DIR_PREFIX, Teardown, TeardownContext, TeardownError, TeardownReason,
};
