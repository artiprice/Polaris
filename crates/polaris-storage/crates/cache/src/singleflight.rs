//! Single-flight `get_or_compute`: concurrent callers for one key run the computation once.

use std::future::Future;
use std::time::Duration;

use polaris_sdk::registry::ExactCacheStore;

use crate::error::CacheError;
use crate::keys::CacheKey;
use crate::tiers::{TieredCache, bump};
use crate::value::{CachedValue, Clock};

impl<L2: ExactCacheStore, C: Clock> TieredCache<L2, C> {
    /// The value for `key` from L1 or L2, or computed by `compute` and stored with `ttl`.
    ///
    /// Concurrent callers for the same key wait for one leader (a `quick_cache` placeholder). If the
    /// leader fails or is cancelled (its future dropped), the placeholder is released, a waiter
    /// becomes the new leader, and nothing is cached — errors are never cached.
    ///
    /// # Errors
    /// The computation's error, or a [`CacheError`] (converted into `E`) from the L2 tier.
    pub async fn get_or_compute<F, Fut, E>(
        &self,
        key: &CacheKey,
        ttl: Option<Duration>,
        compute: F,
    ) -> Result<CachedValue, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CachedValue, E>>,
        E: From<CacheError>,
    {
        let digest = key.digest();
        let guard = loop {
            let now = self.clock.now_unix_ms();
            match self.l1.get_value_or_guard_async(digest).await {
                Ok(entry) if entry.is_live(now) => {
                    bump(&self.counters.l1_hits);
                    return Ok(entry.value);
                }
                // Expired: drop it and ask again (the next round yields a guard or a fresh value).
                Ok(_) => {
                    self.l1.remove_if(digest, |e| !e.is_live(now));
                }
                Err(guard) => break guard,
            }
        };
        let now = self.clock.now_unix_ms();
        if let Some(entry) = self.l2_lookup(key, now)? {
            bump(&self.counters.l2_hits);
            let value = entry.value.clone();
            // Err only if the placeholder was removed meanwhile (e.g. a purge): the value is still
            // correct for this caller.
            let _ = guard.insert(entry);
            return Ok(value);
        }
        bump(&self.counters.misses);
        bump(&self.counters.computes);
        let value = compute().await?;
        let entry = self.store(key, value.clone(), ttl)?;
        let _ = guard.insert(entry);
        Ok(value)
    }
}
