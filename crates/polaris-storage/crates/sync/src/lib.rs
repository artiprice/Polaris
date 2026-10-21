//! # polaris-storage-sync
//!
//! Mirrors the Windows workspace (authoritative) into a sandbox volume (L1, P2):
//!
//! 1. [`scan`] walks the workspace without following links, applies [`IgnoreRules`]
//!    (`.polarisignore`, gitignore syntax, then a **non-overridable** secret deny-list
//!    [`DEFAULT_DENY`]) and hashes files with blake3, reusing a [`ScanCache`] for unchanged files;
//! 2. [`plan`] diffs the local manifest against the last known remote one;
//! 3. [`apply`] transfers byte-exact through the sdk `SandboxBackend` with bounded concurrency,
//!    re-hashing each file at upload time and reporting per-path failures so a re-plan resumes.
//!
//! [`detect_conflicts`] reports paths that cannot round-trip between Windows and Linux (case-only
//! pairs, device names, trailing dots, ADS syntax, invalid characters, over-long names).
//! Measured transport behaviour (1 MiB gateway limit, EOL and exec-bit fidelity) is in spike S0-7.
//!
//! Test-first descriptors: `specs/descriptors/sync_*.json`.

mod apply;
mod ignore;
mod path;
mod plan;
mod scan;

pub use apply::{ApplyOpts, SyncReport, apply};
pub use ignore::{DEFAULT_DENY, Exclusion, IgnoreRules, POLARISIGNORE};
pub use path::{RelPath, SyncError};
pub use plan::{PathConflict, SyncPlan, detect_conflicts, plan};
pub use scan::{EntryKind, FileEntry, HashMode, Manifest, ScanCache, ScanOutcome, ScanStats, scan};
