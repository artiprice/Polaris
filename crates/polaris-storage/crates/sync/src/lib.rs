//! # polaris-storage-sync
//!
//! Mirrors the Windows workspace (authoritative) into a sandbox volume (L1, P2):
//!
//! 1. [`scan`] walks the workspace without following links, applies [`IgnoreRules`]
//!    (`.polarisignore`, gitignore syntax, then a **non-overridable** secret deny-list
//!    [`DEFAULT_DENY`]) and hashes files with blake3, reusing a [`ScanCache`] for unchanged files;
//! 2. [`plan`] diffs the local manifest against the last known remote one;
//! 3. [`apply`] re-hashes each file at transfer time and uses bounded tar batches for ordinary
//!    files plus a typed single-file CAS path for larger files (up to the 256 MiB scanner limit);
//!    the guest revalidates remote state under a workspace lock.
//!
//! [`detect_conflicts`] reports paths that cannot round-trip between Windows and Linux (case-only
//! pairs, device names, trailing dots, ADS syntax, invalid characters, over-long names).
//! Measured transport behaviour (1 MiB gateway limit, EOL and exec-bit fidelity) is in spike S0-7.
//!
//! Test-first descriptors: `specs/descriptors/sync_*.json`.

mod apply;
mod batch;
mod ignore;
mod path;
mod plan;
mod remote;
mod scan;

pub use apply::{ApplyOpts, SecretPrecheck, SyncReport, apply, apply_with_precheck};
pub use batch::encode_batch_archive;
pub use ignore::{DEFAULT_DENY, Exclusion, IgnoreRules, MAX_POLARISIGNORE_BYTES, POLARISIGNORE};
pub use path::{RelPath, SyncError};
pub use plan::{PathConflict, SyncConflict, SyncPlan, detect_conflicts, plan, plan_two_way};
pub use remote::SandboxSyncBackend;
pub use scan::{
    EntryKind, FileEntry, HashMode, Manifest, ScanCache, ScanLimits, ScanOutcome, ScanStats, scan,
    scan_with_limits,
};
