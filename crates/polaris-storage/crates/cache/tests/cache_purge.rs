//! Cache Purge (descriptor: specs/descriptors/cache_purge.json).

use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use polaris_sdk::ids::{Cid, SessionId};
use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use polaris_storage_cache::{
    CacheConfig, CacheError, CachedValue, SystemClock, TieredCache, key_for_s1,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory L2 test double (never shipped).
#[derive(Clone, Default)]
struct MemL2 {
    rows: Arc<Mutex<HashMap<[u8; 32], (CacheScope, Vec<u8>)>>>,
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

fn cache(l2: &MemL2, cfg: CacheConfig) -> Result<TieredCache<MemL2, SystemClock>, CacheError> {
    TieredCache::new(cfg, l2.clone())
}

const SCOPES: [CacheScope; 3] = [
    CacheScope::Global,
    CacheScope::Workspace {
        workspace: Cid::from_bytes([1; 32]),
    },
    CacheScope::Session {
        session: SessionId::from_u128(9),
    },
];

#[test]
fn purge_scope_removes_only_that_scope() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2, CacheConfig::default())?;
    let keys: Vec<_> = SCOPES
        .iter()
        .map(|s| key_for_s1(*s, &[0; 32], b"x"))
        .collect();
    for k in &keys {
        c.put(k, CachedValue::from(b"v".to_vec()), None)?;
    }
    let report = c.purge(SCOPES.get(1).ok_or("scope")?)?;
    assert_eq!((report.l1_removed, report.l2_removed), (1, 1));
    let present: Vec<bool> = keys
        .iter()
        .map(|k| c.get(k).map(|v| v.is_some()))
        .collect::<Result<_, _>>()?;
    assert_eq!(present, vec![true, false, true]);
    Ok(())
}

#[test]
fn purge_memory_keeps_l2() -> TestResult {
    let l2 = MemL2::default();
    let c = cache(&l2, CacheConfig::default())?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"x");
    c.put(&k, CachedValue::from(b"v".to_vec()), None)?;
    assert_eq!(c.purge_memory(), 1);
    assert_eq!(c.stats().l1_entries, 0);
    assert!(c.get(&k)?.is_some(), "L2 still serves the value");
    Ok(())
}

#[test]
fn eviction_is_counted() -> TestResult {
    let l2 = MemL2::default();
    let cfg = CacheConfig {
        l1_weight_capacity: 4 * 1024,
        l1_estimated_items: 16,
    };
    let c = cache(&l2, cfg)?;
    for i in 0u32..200 {
        let k = key_for_s1(CacheScope::Global, &[0; 32], &i.to_le_bytes());
        c.put(&k, CachedValue::from(vec![0u8; 256]), None)?;
    }
    let s = c.stats();
    assert!(
        s.evictions > 0,
        "a 4 KiB L1 must evict 200 x 256 B values: {s:?}"
    );
    assert!(s.l1_weight <= 4 * 1024, "{s:?}");
    Ok(())
}

proptest! {
    #[test]
    fn prop_purge_removes_exactly_scope(
        entries in proptest::collection::vec((0usize..3, any::<u16>()), 1..40),
        victim in 0usize..3,
    ) {
        let l2 = MemL2::default();
        let c = cache(&l2, CacheConfig::default()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut keys = Vec::new();
        for (scope, id) in &entries {
            let scope = SCOPES.get(*scope).copied().unwrap_or(CacheScope::Global);
            let k = key_for_s1(scope, &[0; 32], &id.to_le_bytes());
            c.put(&k, CachedValue::from(b"v".to_vec()), None).map_err(|e| TestCaseError::fail(e.to_string()))?;
            keys.push((scope, k));
        }
        let victim = SCOPES.get(victim).copied().unwrap_or(CacheScope::Global);
        c.purge(&victim).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for (scope, k) in &keys {
            let present = c.get(k).map_err(|e| TestCaseError::fail(e.to_string()))?.is_some();
            prop_assert_eq!(present, *scope != victim);
        }
    }
}
