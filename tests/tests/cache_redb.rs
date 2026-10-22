//! Cross-crate cache persistence through the real redb EventLog adapter.

use std::error::Error;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use polaris_sdk::ids::Cid;
use polaris_sdk::registry::CacheScope;
use polaris_storage::cache::{CacheConfig, CachedValue, TieredCache, key_for_s1};
use polaris_storage::eventlog::{EventLog, EventLogConfig};

type TestResult = Result<(), Box<dyn Error>>;

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
    fn new() -> Result<Self, std::io::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?;
        let name = format!(
            "polaris-cache-redb-{}-{}.redb",
            std::process::id(),
            now.as_nanos()
        );
        Ok(Self(std::env::temp_dir().join(name)))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn cache_uses_real_redb_store_across_reopen_and_purge() -> TestResult {
    let file = TemporaryDatabase::new()?;
    let scope = CacheScope::Workspace {
        workspace: Cid::from_bytes([0x3a; 32]),
    };
    let key = key_for_s1(scope, &[0x61; 32], b"same request");
    let global_key = key_for_s1(CacheScope::Global, &[0x61; 32], b"same request");

    {
        let log = EventLog::open(EventLogConfig::new(file.path()))?;
        let cache = TieredCache::new(CacheConfig::default(), log)?;
        cache.put(&key, CachedValue::from(b"workspace result".to_vec()), None)?;
        cache.put(
            &global_key,
            CachedValue::from(b"global result".to_vec()),
            None,
        )?;
        assert_eq!(cache.purge_memory(), 2);
    }

    let log = EventLog::open(EventLogConfig::new(file.path()))?;
    let cache = TieredCache::new(CacheConfig::default(), log)?;
    assert_eq!(
        cache.get(&key)?.as_ref().map(CachedValue::as_bytes),
        Some(&b"workspace result"[..]),
        "L2 must survive dropping and reopening the redb owner"
    );
    assert_eq!(cache.stats().l2_hits, 1);
    assert_eq!(
        cache.purge(&scope)?.l2_removed,
        1,
        "workspace purge must remove its exact redb row"
    );
    assert_eq!(cache.get(&key)?, None);
    assert_eq!(
        cache.get(&global_key)?.as_ref().map(CachedValue::as_bytes),
        Some(&b"global result"[..]),
        "purging one scope must preserve other redb rows"
    );
    Ok(())
}
