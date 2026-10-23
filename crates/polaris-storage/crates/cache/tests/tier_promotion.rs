//! Tier Promotion (descriptor: specs/descriptors/cache_tier_promotion.json).

use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use polaris_storage_cache::{CacheConfig, CacheError, CachedValue, Clock, TieredCache, key_for_s1};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory L2 test double (never shipped).
#[derive(Clone, Default)]
struct MemL2 {
    rows: Arc<Mutex<HashMap<[u8; 32], (CacheScope, Vec<u8>)>>>,
    refuse_writes: Arc<AtomicBool>,
}

impl MemL2 {
    fn raw(&self, key: &[u8; 32]) -> Option<(CacheScope, Vec<u8>)> {
        self.rows.lock().ok()?.get(key).cloned()
    }

    fn overwrite(&self, key: [u8; 32], value: Vec<u8>) {
        if let Ok(mut rows) = self.rows.lock() {
            rows.insert(key, (CacheScope::Global, value));
        }
    }
}

impl ExactCacheStore for MemL2 {
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        Ok(rows.get(key).map(|(_, v)| v.clone()))
    }

    fn get_bounded(
        &self,
        key: &[u8; 32],
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        let Some((_, value)) = rows.get(key) else {
            return Ok(None);
        };
        if value.len() > max_bytes {
            return Err(CacheStoreError::TooLarge {
                bytes: u64::try_from(value.len()).unwrap_or(u64::MAX),
                limit: u64::try_from(max_bytes).unwrap_or(u64::MAX),
            });
        }
        Ok(Some(value.clone()))
    }

    fn put(&self, key: [u8; 32], value: &[u8], scope: CacheScope) -> Result<(), CacheStoreError> {
        if self.refuse_writes.load(Ordering::SeqCst) {
            return Err(CacheStoreError::Io("disk full".into()));
        }
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        rows.insert(key, (scope, value.to_vec()));
        Ok(())
    }

    fn clear_scope(&self, scope: &CacheScope) -> Result<u64, CacheStoreError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        let before = rows.len();
        rows.retain(|_, (s, _)| s != scope);
        Ok(u64::try_from(before - rows.len()).unwrap_or(u64::MAX))
    }
}

#[derive(Clone, Default)]
struct FakeClock(Arc<AtomicU64>);

impl Clock for FakeClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn cache(l2: &MemL2) -> Result<TieredCache<MemL2, FakeClock>, CacheError> {
    TieredCache::with_clock(CacheConfig::default(), l2.clone(), FakeClock::default())
}

#[test]
fn miss_returns_none() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2)?;
    assert_eq!(
        c.get(&key_for_s1(CacheScope::Global, &[0; 32], b"x"))?,
        None
    );
    assert_eq!(c.stats().misses, 1);
    Ok(())
}

#[test]
fn put_writes_through_to_l2() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2)?;
    let session = CacheScope::Session {
        session: polaris_sdk::ids::SessionId::from_u128(5),
    };
    let k = key_for_s1(session, &[0; 32], b"x");
    c.put(&k, CachedValue::from(b"verdict".to_vec()), None)?;
    let (scope, _) = l2.raw(k.digest()).ok_or("not in L2")?;
    assert_eq!(scope, session);
    assert_eq!(
        c.get(&k)?.as_ref().map(CachedValue::as_bytes),
        Some(&b"verdict"[..])
    );
    assert_eq!(c.stats().l1_hits, 1);
    Ok(())
}

#[test]
fn l2_hit_promotes_to_l1() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2)?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    c.put(&k, CachedValue::from(b"v".to_vec()), None)?;
    c.purge_memory();
    assert_eq!(
        c.get(&k)?.as_ref().map(CachedValue::as_bytes),
        Some(&b"v"[..])
    );
    assert_eq!(c.stats().l2_hits, 1);
    assert_eq!(
        c.get(&k)?.as_ref().map(CachedValue::as_bytes),
        Some(&b"v"[..])
    );
    assert_eq!(
        c.stats().l1_hits,
        1,
        "second get is served by L1 after promotion"
    );
    Ok(())
}

#[test]
fn stats_count_each_tier() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2)?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    c.get(&k)?;
    c.put(&k, CachedValue::from(b"v".to_vec()), None)?;
    c.get(&k)?;
    c.purge_memory();
    c.get(&k)?;
    let s = c.stats();
    assert_eq!((s.misses, s.l1_hits, s.l2_hits), (1, 1, 1));
    Ok(())
}

#[test]
fn l2_error_leaves_nothing_cached() -> TestResult {
    let l2 = MemL2::default();
    l2.refuse_writes.store(true, Ordering::SeqCst);
    let c = cache(&l2)?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    assert!(matches!(
        c.put(&k, CachedValue::from(b"v".to_vec()), None),
        Err(CacheError::L2(_))
    ));
    assert_eq!(c.get(&k)?, None);
    Ok(())
}

#[test]
fn ttl_expiry_is_a_miss() -> TestResult {
    let l2 = MemL2::default();
    let clock = FakeClock::default();
    let c = TieredCache::with_clock(CacheConfig::default(), l2.clone(), clock.clone())?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    c.put(
        &k,
        CachedValue::from(b"v".to_vec()),
        Some(Duration::from_secs(10)),
    )?;
    clock.0.store(9_999, Ordering::SeqCst);
    assert!(c.get(&k)?.is_some(), "not yet expired");
    clock.0.store(10_000, Ordering::SeqCst);
    assert_eq!(c.get(&k)?, None, "expired in L1");
    c.purge_memory();
    assert_eq!(c.get(&k)?, None, "expired in L2 too");
    Ok(())
}

#[test]
fn corrupt_l2_entry_is_codec_error() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2)?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    l2.overwrite(*k.digest(), vec![0xFF, 1, 2]);
    assert!(matches!(c.get(&k), Err(CacheError::Codec(_))));
    l2.overwrite(*k.digest(), Vec::new());
    assert!(matches!(c.get(&k), Err(CacheError::Codec(_))));
    Ok(())
}

proptest! {
    #[test]
    fn prop_get_after_put_returns_value(
        input in proptest::collection::vec(any::<u8>(), 0..64),
        value in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let l2 = MemL2::default();
        let c = cache(&l2).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let k = key_for_s1(CacheScope::Global, &[1; 32], &input);
        c.put(&k, CachedValue::from(value.clone()), None).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let got = c.get(&k).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(got.as_ref().map(CachedValue::as_bytes), Some(&value[..]));
    }
}
