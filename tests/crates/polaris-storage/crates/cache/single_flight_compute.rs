//! Single Flight Compute (descriptor: specs/descriptors/cache_single_flight_compute.json).

use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use polaris_storage_cache::{CacheConfig, CacheError, CachedValue, Clock, TieredCache, key_for_s1};

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory L2 test double (never shipped).
#[derive(Clone, Default)]
struct MemL2 {
    rows: Arc<Mutex<HashMap<[u8; 32], Vec<u8>>>>,
}

impl ExactCacheStore for MemL2 {
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        Ok(rows.get(key).cloned())
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
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        rows.insert(key, value.to_vec());
        Ok(())
    }

    fn clear_scope(&self, _scope: &CacheScope) -> Result<u64, CacheStoreError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| CacheStoreError::Io("poisoned".into()))?;
        let n = rows.len();
        rows.clear();
        Ok(u64::try_from(n).unwrap_or(u64::MAX))
    }
}

#[derive(Clone, Default)]
struct FakeClock(Arc<AtomicU64>);

impl Clock for FakeClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The computation's own error type; absorbs cache errors.
#[derive(Debug, PartialEq)]
enum ComputeError {
    Upstream(&'static str),
    Cache(String),
}

impl From<CacheError> for ComputeError {
    fn from(e: CacheError) -> Self {
        Self::Cache(e.to_string())
    }
}

type Cache = TieredCache<MemL2, FakeClock>;

fn cache() -> Result<(Arc<Cache>, MemL2, FakeClock), CacheError> {
    let l2 = MemL2::default();
    let clock = FakeClock::default();
    let c = TieredCache::with_clock(CacheConfig::default(), l2.clone(), clock.clone())?;
    Ok((Arc::new(c), l2, clock))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_run_f_once() -> TestResult {
    let (c, _, _) = cache()?;
    let runs = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let (c, runs, start) = (Arc::clone(&c), Arc::clone(&runs), Arc::clone(&start));
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            let k = key_for_s1(CacheScope::Global, &[0; 32], b"same");
            c.get_or_compute(&k, None, || async {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<_, ComputeError>(CachedValue::from(b"answer".to_vec()))
            })
            .await
        }));
    }
    for task in tasks {
        let value = task.await?.map_err(|e| format!("{e:?}"))?;
        assert_eq!(value.as_bytes(), b"answer");
    }
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the computation ran more than once"
    );
    assert_eq!(c.stats().computes, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leader_cancel_wakes_waiters() -> TestResult {
    let (c, _, _) = cache()?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"slow");
    let leader = {
        let (c, k) = (Arc::clone(&c), k.clone());
        tokio::spawn(async move {
            c.get_or_compute(&k, None, || async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok::<_, ComputeError>(CachedValue::from(b"never".to_vec()))
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let waiter = {
        let (c, k) = (Arc::clone(&c), k.clone());
        tokio::spawn(async move {
            c.get_or_compute(&k, None, || async {
                Ok::<_, ComputeError>(CachedValue::from(b"waiter".to_vec()))
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    leader.abort();
    let value = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await??
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(
        value.as_bytes(),
        b"waiter",
        "the waiter took over after the leader was cancelled"
    );
    Ok(())
}

#[tokio::test]
async fn error_not_cached() -> TestResult {
    let (c, _, _) = cache()?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"flaky");
    let first = c
        .get_or_compute(&k, None, || async {
            Err::<CachedValue, _>(ComputeError::Upstream("503"))
        })
        .await;
    assert_eq!(first, Err(ComputeError::Upstream("503")));
    assert_eq!(c.get(&k)?, None, "a failure is not cached");
    let second = c
        .get_or_compute(&k, None, || async {
            Ok::<_, ComputeError>(CachedValue::from(b"ok".to_vec()))
        })
        .await
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(second.as_bytes(), b"ok");
    Ok(())
}

#[tokio::test]
async fn ttl_expiry_recomputes() -> TestResult {
    let (c, _, clock) = cache()?;
    let runs = AtomicUsize::new(0);
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"ttl");
    let compute = || async {
        runs.fetch_add(1, Ordering::SeqCst);
        Ok::<_, ComputeError>(CachedValue::from(b"v".to_vec()))
    };
    c.get_or_compute(&k, Some(Duration::from_secs(1)), compute)
        .await
        .map_err(|e| format!("{e:?}"))?;
    c.get_or_compute(&k, Some(Duration::from_secs(1)), compute)
        .await
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "fresh value is reused");
    clock.0.store(1_000, Ordering::SeqCst);
    c.get_or_compute(&k, Some(Duration::from_secs(1)), compute)
        .await
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "expired value is recomputed"
    );
    Ok(())
}

#[tokio::test]
async fn l2_hit_skips_compute() -> TestResult {
    let (c, _, _) = cache()?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"persisted");
    c.put(&k, CachedValue::from(b"from-l2".to_vec()), None)?;
    c.purge_memory();
    let value = c
        .get_or_compute(&k, None, || async {
            Err::<CachedValue, _>(ComputeError::Upstream("must not run"))
        })
        .await
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(value.as_bytes(), b"from-l2");
    assert_eq!(c.stats().computes, 0);
    Ok(())
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn get_or_compute_future_is_send() -> TestResult {
    let (c, _, _) = cache()?;
    let k = key_for_s1(CacheScope::Global, &[0; 32], b"send");
    let fut = c.get_or_compute(&k, None, || async {
        Ok::<_, ComputeError>(CachedValue::from(Vec::new()))
    });
    assert_send(&fut);
    Ok(())
}
