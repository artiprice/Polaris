//! The tiered cache: L1 `quick_cache` in RAM, L2 an injected [`ExactCacheStore`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use polaris_sdk::registry::{CacheScope, ExactCacheStore};
use quick_cache::sync::Cache;
use quick_cache::{DefaultHashBuilder, OptionsBuilder, Weighter};

use crate::error::CacheError;
use crate::keys::CacheKey;
use crate::value::{CachedValue, Clock, SystemClock, decode, encode};

/// Fixed per-entry weight added to the value length (key, scope, bookkeeping).
const ENTRY_OVERHEAD: u64 = 64;

/// L1 sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheConfig {
    /// L1 capacity in bytes (value bytes plus a fixed per-entry overhead).
    pub l1_weight_capacity: u64,
    /// Expected number of L1 entries (sizing hint).
    pub l1_estimated_items: usize,
}

impl Default for CacheConfig {
    /// 64 MiB, ~16k entries.
    fn default() -> Self {
        Self {
            l1_weight_capacity: 64 * 1024 * 1024,
            l1_estimated_items: 16 * 1024,
        }
    }
}

/// Counters for `/cache stats` and telemetry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Served from L1.
    pub l1_hits: u64,
    /// Served from L2 (and promoted to L1).
    pub l2_hits: u64,
    /// Found in no tier.
    pub misses: u64,
    /// Computations run by `get_or_compute`.
    pub computes: u64,
    /// L1 evictions (including values too large to admit).
    pub evictions: u64,
    /// Current L1 entries.
    pub l1_entries: u64,
    /// Current L1 weight in bytes.
    pub l1_weight: u64,
}

/// What `purge` removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// Entries removed from L1.
    pub l1_removed: u64,
    /// Entries removed from L2.
    pub l2_removed: u64,
}

/// An L1 entry.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) value: CachedValue,
    scope: CacheScope,
    expires_at: Option<u64>,
}

impl Entry {
    pub(crate) fn is_live(&self, now_ms: u64) -> bool {
        self.expires_at.is_none_or(|t| now_ms < t)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ByteWeighter;

impl Weighter<[u8; 32], Entry> for ByteWeighter {
    fn weight(&self, _key: &[u8; 32], entry: &Entry) -> u64 {
        u64::try_from(entry.value.len())
            .unwrap_or(u64::MAX)
            .saturating_add(ENTRY_OVERHEAD)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvictionCounter(Arc<AtomicU64>);

impl quick_cache::Lifecycle<[u8; 32], Entry> for EvictionCounter {
    type RequestState = ();

    fn on_evict(&self, _state: &mut (), _key: [u8; 32], _entry: Entry) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) type L1 = Cache<[u8; 32], Entry, ByteWeighter, DefaultHashBuilder, EvictionCounter>;

#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub(crate) l1_hits: AtomicU64,
    pub(crate) l2_hits: AtomicU64,
    pub(crate) misses: AtomicU64,
    pub(crate) computes: AtomicU64,
}

pub(crate) fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// L1 (RAM) over an injected L2 exact store. The semantic L3 tier (sdk `VectorStore`) arrives with
/// embeddings in P5 and is restricted to non-safety kinds.
///
/// L2 calls are synchronous (redb read/write transactions are short); async callers that cannot
/// afford that use `spawn_blocking`.
pub struct TieredCache<L2, C = SystemClock> {
    pub(crate) l1: L1,
    pub(crate) l2: L2,
    pub(crate) clock: C,
    pub(crate) counters: Counters,
    evictions: Arc<AtomicU64>,
}

impl<L2: ExactCacheStore> TieredCache<L2, SystemClock> {
    /// A cache over `l2` with the system clock.
    ///
    /// # Errors
    /// [`CacheError::Config`] for a zero capacity.
    pub fn new(cfg: CacheConfig, l2: L2) -> Result<Self, CacheError> {
        Self::with_clock(cfg, l2, SystemClock)
    }
}

impl<L2: ExactCacheStore, C: Clock> TieredCache<L2, C> {
    /// A cache over `l2` with an explicit clock (tests use a fake one).
    ///
    /// # Errors
    /// [`CacheError::Config`] for a zero capacity.
    pub fn with_clock(cfg: CacheConfig, l2: L2, clock: C) -> Result<Self, CacheError> {
        if cfg.l1_weight_capacity == 0 || cfg.l1_estimated_items == 0 {
            return Err(CacheError::Config("L1 capacities must be non-zero".into()));
        }
        let options = OptionsBuilder::new()
            .estimated_items_capacity(cfg.l1_estimated_items)
            .weight_capacity(cfg.l1_weight_capacity)
            .build()
            .map_err(|e| CacheError::Config(e.to_string()))?;
        let evictions = Arc::new(AtomicU64::new(0));
        let l1 = Cache::with_options(
            options,
            ByteWeighter,
            DefaultHashBuilder::default(),
            EvictionCounter(Arc::clone(&evictions)),
        );
        Ok(Self {
            l1,
            l2,
            clock,
            counters: Counters::default(),
            evictions,
        })
    }

    /// Absolute expiry for a TTL from now.
    pub(crate) fn expiry(&self, ttl: Option<Duration>) -> Option<u64> {
        ttl.map(|d| {
            self.clock
                .now_unix_ms()
                .saturating_add(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        })
    }

    /// Looks `key` up in L2; an expired entry is a miss (it stays in L2 until overwritten or
    /// purged — the sdk trait has no single-key delete).
    pub(crate) fn l2_lookup(
        &self,
        key: &CacheKey,
        now_ms: u64,
    ) -> Result<Option<Entry>, CacheError> {
        let Some(bytes) = self.l2.get(key.digest())? else {
            return Ok(None);
        };
        let (expires_at, payload) = decode(&bytes)?;
        let entry = Entry {
            value: CachedValue::from(payload),
            scope: *key.scope(),
            expires_at,
        };
        Ok(entry.is_live(now_ms).then_some(entry))
    }

    /// Writes through to L2, then L1. On an L2 error nothing is cached.
    pub(crate) fn store(
        &self,
        key: &CacheKey,
        value: CachedValue,
        ttl: Option<Duration>,
    ) -> Result<Entry, CacheError> {
        let expires_at = self.expiry(ttl);
        self.l2.put(
            *key.digest(),
            &encode(expires_at, value.as_bytes()),
            *key.scope(),
        )?;
        Ok(Entry {
            value,
            scope: *key.scope(),
            expires_at,
        })
    }

    /// L1, then L2 (promoting a hit into L1). Expired entries are misses.
    ///
    /// # Errors
    /// [`CacheError::L2`] or [`CacheError::Codec`] from the L2 tier.
    pub fn get(&self, key: &CacheKey) -> Result<Option<CachedValue>, CacheError> {
        let now = self.clock.now_unix_ms();
        if let Some(entry) = self.l1.get(key.digest()) {
            if entry.is_live(now) {
                bump(&self.counters.l1_hits);
                return Ok(Some(entry.value));
            }
            self.l1.remove_if(key.digest(), |e| !e.is_live(now));
        }
        match self.l2_lookup(key, now)? {
            Some(entry) => {
                bump(&self.counters.l2_hits);
                let value = entry.value.clone();
                self.l1.insert(*key.digest(), entry);
                Ok(Some(value))
            }
            None => {
                bump(&self.counters.misses);
                Ok(None)
            }
        }
    }

    /// Stores `value` in L2 and L1, optionally expiring after `ttl`.
    ///
    /// # Errors
    /// [`CacheError::L2`] when the L2 write fails; then L1 is not updated either.
    pub fn put(
        &self,
        key: &CacheKey,
        value: CachedValue,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let entry = self.store(key, value, ttl)?;
        self.l1.insert(*key.digest(), entry);
        Ok(())
    }

    /// Removes every entry of `scope` from both tiers (`/cache purge`).
    ///
    /// # Errors
    /// [`CacheError::L2`] when L2 cannot clear the scope (L1 is already purged then).
    pub fn purge(&self, scope: &CacheScope) -> Result<PurgeReport, CacheError> {
        let removed = AtomicU64::new(0);
        self.l1.retain(|_, entry| {
            let keep = entry.scope != *scope;
            if !keep {
                removed.fetch_add(1, Ordering::Relaxed);
            }
            keep
        });
        let l2_removed = self.l2.clear_scope(scope)?;
        Ok(PurgeReport {
            l1_removed: removed.into_inner(),
            l2_removed,
        })
    }

    /// Empties L1 only; returns how many entries it held.
    pub fn purge_memory(&self) -> u64 {
        let held = u64::try_from(self.l1.len()).unwrap_or(u64::MAX);
        self.l1.clear();
        held
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        CacheStats {
            l1_hits: load(&self.counters.l1_hits),
            l2_hits: load(&self.counters.l2_hits),
            misses: load(&self.counters.misses),
            computes: load(&self.counters.computes),
            evictions: load(&self.evictions),
            l1_entries: u64::try_from(self.l1.len()).unwrap_or(u64::MAX),
            l1_weight: self.l1.weight(),
        }
    }
}
