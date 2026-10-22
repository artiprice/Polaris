//! # polaris-storage
//!
//! Facade re-exporting the storage children (cas, eventlog, registry, cache, vector, sync).
//!
//! ## Durable layout and scope decision
//!
//! The caller selects a state root (`~/.polaris` by default in the CLI). One root owns one
//! `events.redb` journal and `registry.db` catalog. A repository does not silently create a second
//! journal: an isolated deployment explicitly supplies `.polaris/state` as its state root.
//! Cache keys additionally contain global/workspace scope; isolated keys cannot share L1/L2 data.
//!
//! | Path under the state root | Owner / contents |
//! |---|---|
//! | `registry.db`, `-wal`, `-shm` | SQLite session catalog and redacted history FTS5 |
//! | `events.redb` | Durable events, checkpoints, lock table and exact L2 cache |
//! | `cas/ab/cd/<blake3>` | Versioned raw or zstd content-addressed blobs |
//! | `run/reaper.journal` | Durable cleanup locators; unresolved identities stay visible |
//! | RAM | Byte-weighted quick_cache L1 |
//! | `vectors/` | P5 vector provider, not part of P1 |
//!
//! Invariant/rule mirrors and research checkpoints join the selected journal with their P4/P7
//! owners. Registry rows are rebuildable projections; the event journal remains authoritative.
//!
//! ```
//! use polaris_storage::{cache, cas, eventlog, registry};
//! use polaris_storage::{sync as _, vector as _};
//! let _ = core::mem::size_of::<cache::CacheConfig>();
//! let _ = core::mem::size_of::<cas::CasConfig>();
//! let _ = core::mem::size_of::<eventlog::EventLogConfig>();
//! let _ = core::mem::size_of::<registry::RegistryConfig>();
//! ```
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L1 — foundations |
//! | Kind | facade |
//! | Phase | P1 |
//! | Parent facade | — |
//! | Status | facade exports implemented; child completion is tracked separately |
//!
//! ## Children
//!
//! - `polaris-storage-cas`
//! - `polaris-storage-eventlog`
//! - `polaris-storage-registry`
//! - `polaris-storage-cache`
//! - `polaris-storage-vector`
//! - `polaris-storage-sync`
//!
//! ## Responsibilities
//!
//! - Re-export storage children; own the on-disk layout documentation (~/.polaris, .polaris).
//!
//! ## Planned API
//!
//! - `pub use polaris_storage_*`
//!
//! ## Internal dependencies
//!
//! `polaris-storage-cas`, `polaris-storage-eventlog`, `polaris-storage-registry`, `polaris-storage-cache`, `polaris-storage-vector`, `polaris-storage-sync`
//!
//! ## Planned external dependencies
//!
//! none (facade)

pub use polaris_storage_cache as cache;
pub use polaris_storage_cas as cas;
pub use polaris_storage_eventlog as eventlog;
pub use polaris_storage_registry as registry;
pub use polaris_storage_sync as sync;
pub use polaris_storage_vector as vector;
