//! # polaris-storage
//!
//! Facade re-exporting the storage children (cas, eventlog, registry, cache, vector, sync).
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
