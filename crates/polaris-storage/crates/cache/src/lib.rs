//! # polaris-storage-cache
//!
//! Tiered caches for S1 decisions, S2 completions and embeddings (L1, P1):
//!
//! - **L1** — in-RAM [`quick_cache`] weighted by bytes (replaces the chatlog's `DashMap`, D8);
//! - **L2** — the exact persistent tier behind the sdk [`ExactCacheStore`] trait (physically the
//!   `l2_cache` table of `events.redb`, wired at the composition root);
//! - **L3** (semantic, sdk `VectorStore`) — deferred to P5 with embeddings, non-safety kinds only.
//!
//! Keys ([`key_for_completion`], [`key_for_s1`], [`key_for_embedding`]) are domain-separated
//! blake3 digests that include the scope, so `$storage-level(isolated)` scopes never share entries.
//! [`TieredCache::get_or_compute`] is single-flight and never caches errors.
//!
//! Test-first descriptors: `specs/descriptors/cache_*.json`.
//!
//! [`ExactCacheStore`]: polaris_sdk::registry::ExactCacheStore

mod error;
mod keys;
mod singleflight;
mod tiers;
mod value;

pub use error::CacheError;
pub use keys::{CacheKey, KeyKind, key_for_completion, key_for_embedding, key_for_s1};
pub use tiers::{CacheConfig, CacheStats, PurgeReport, TieredCache};
pub use value::{CachedValue, Clock, SystemClock};
