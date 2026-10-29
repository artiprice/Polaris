//! Cache capacity pressure and opt-in hot-path measurement.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use polaris_storage_cache::{CacheConfig, CachedValue, TieredCache, key_for_s1};

#[derive(Clone, Default)]
struct MemL2(Arc<Mutex<HashMap<[u8; 32], Vec<u8>>>>);

impl ExactCacheStore for MemL2 {
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let rows = self
            .0
            .lock()
            .map_err(|_| CacheStoreError::Io("test L2 lock poisoned".into()))?;
        Ok(rows.get(key).cloned())
    }

    fn get_bounded(
        &self,
        key: &[u8; 32],
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let rows = self
            .0
            .lock()
            .map_err(|_| CacheStoreError::Io("test L2 lock poisoned".into()))?;
        let Some(value) = rows.get(key) else {
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

    fn put(&self, key: [u8; 32], value: &[u8], _scope: CacheScope) -> Result<(), CacheStoreError> {
        let mut rows = self
            .0
            .lock()
            .map_err(|_| CacheStoreError::Io("test L2 lock poisoned".into()))?;
        rows.insert(key, value.to_vec());
        Ok(())
    }

    fn clear_scope(&self, _scope: &CacheScope) -> Result<u64, CacheStoreError> {
        let mut rows = self
            .0
            .lock()
            .map_err(|_| CacheStoreError::Io("test L2 lock poisoned".into()))?;
        let count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        rows.clear();
        Ok(count)
    }
}

#[test]
fn capacity_pressure_keeps_weight_bounded_and_counts_evictions()
-> Result<(), Box<dyn std::error::Error>> {
    let cache = Arc::new(TieredCache::new(
        CacheConfig {
            l1_weight_capacity: 32 * 1024,
            l1_estimated_items: 512,
        },
        MemL2::default(),
    )?);
    let workers = 8usize;
    let operations_per_worker = 2_000usize;
    let mut joins = Vec::with_capacity(workers);

    for worker in 0..workers {
        let cache = Arc::clone(&cache);
        joins.push(thread::spawn(move || -> Result<(), String> {
            for op in 0..operations_per_worker {
                let id = u64::try_from(worker * operations_per_worker + op)
                    .map_err(|error| error.to_string())?;
                let key = key_for_s1(CacheScope::Global, &[0x3c; 32], &id.to_le_bytes());
                cache
                    .put(&key, CachedValue::from(vec![0x5a; 384]), None)
                    .map_err(|error| error.to_string())?;
                let _ = cache.get(&key).map_err(|error| error.to_string())?;
            }
            Ok(())
        }));
    }

    for join in joins {
        join.join().map_err(|_| "capacity worker panicked")??;
    }
    let stats = cache.stats();
    assert!(
        stats.l1_weight <= 32 * 1024,
        "L1 exceeded configured capacity: {stats:?}"
    );
    assert!(
        stats.evictions > 0,
        "pressure should cause capacity eviction: {stats:?}"
    );
    assert!(
        stats.l1_entries > 0,
        "recent values should remain resident: {stats:?}"
    );
    Ok(())
}

#[test]
#[ignore = "manual release measurement; reports rather than asserts a latency target"]
fn measure_parallel_l1_hits() -> Result<(), Box<dyn std::error::Error>> {
    let workers = 8usize;
    let operations_per_worker = 1_000_000usize;
    let cache = Arc::new(TieredCache::new(
        CacheConfig {
            l1_weight_capacity: 1024 * 1024,
            l1_estimated_items: workers,
        },
        MemL2::default(),
    )?);
    let keys: Vec<_> = (0..workers)
        .map(|index| {
            key_for_s1(
                CacheScope::Global,
                &[0x91; 32],
                &u64::try_from(index).unwrap_or(u64::MAX).to_le_bytes(),
            )
        })
        .collect();
    for key in &keys {
        cache.put(key, CachedValue::from(b"hot".to_vec()), None)?;
    }

    let start = Instant::now();
    let mut joins = Vec::with_capacity(workers);
    for index in 0..workers {
        let cache = Arc::clone(&cache);
        let key = keys.get(index).ok_or("missing prewarmed key")?.clone();
        joins.push(thread::spawn(move || -> Result<(), String> {
            for _ in 0..operations_per_worker {
                if cache
                    .get(&key)
                    .map_err(|error| error.to_string())?
                    .is_none()
                {
                    return Err("prewarmed L1 value disappeared".to_owned());
                }
            }
            Ok(())
        }));
    }
    for join in joins {
        join.join().map_err(|_| "measurement worker panicked")??;
    }
    let elapsed = start.elapsed();
    let operations = workers.saturating_mul(operations_per_worker);
    let ns_per_op = elapsed.as_nanos() / u128::try_from(operations)?;
    eprintln!(
        "CACHE_LOAD tier=L1 operation=get threads={workers} operations={operations} elapsed_ms={} ns_per_op={ns_per_op} profile={}",
        elapsed.as_millis(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    Ok(())
}
