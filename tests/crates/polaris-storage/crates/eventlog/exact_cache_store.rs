//! Exact Cache Store (descriptor: specs/descriptors/eventlog_exact_cache_store.json).

use std::error::Error;

use polaris_sdk::ids::{Cid, SessionId};
use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use polaris_storage_eventlog::{EventLog, EventLogConfig};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn put_get_roundtrip() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    log.put([1; 32], b"cached completion", CacheScope::Global)?;
    assert_eq!(log.get(&[1; 32])?, Some(b"cached completion".to_vec()));
    assert_eq!(log.get(&[2; 32])?, None);
    log.put([1; 32], b"replaced", CacheScope::Global)?;
    assert_eq!(log.get(&[1; 32])?, Some(b"replaced".to_vec()));
    Ok(())
}

#[test]
fn clear_scope_removes_only_that_scope() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    let session = CacheScope::Session {
        session: SessionId::from_u128(5),
    };
    let workspace = CacheScope::Workspace {
        workspace: Cid::from_bytes([7; 32]),
    };
    log.put([1; 32], b"a", session)?;
    log.put([2; 32], b"b", session)?;
    log.put([3; 32], b"c", workspace)?;
    log.put([4; 32], b"d", CacheScope::Global)?;
    assert_eq!(log.clear_scope(&session)?, 2);
    assert_eq!(log.get(&[1; 32])?, None);
    assert_eq!(log.get(&[3; 32])?, Some(b"c".to_vec()));
    assert_eq!(log.get(&[4; 32])?, Some(b"d".to_vec()));
    assert_eq!(log.clear_scope(&session)?, 0);
    Ok(())
}

#[test]
fn oversized_value_is_rejected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut cfg = EventLogConfig::new(dir.path().join("events.redb"));
    cfg.max_cache_value_bytes = 16;
    let log = EventLog::open(cfg)?;
    assert!(matches!(
        log.put([1; 32], &[0u8; 17], CacheScope::Global),
        Err(CacheStoreError::TooLarge {
            bytes: 17,
            limit: 16
        })
    ));
    assert!(log.put([1; 32], &[0u8; 16], CacheScope::Global).is_ok());
    Ok(())
}

#[test]
fn bounded_get_rejects_before_copying_oversized_payload() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    log.put([9; 32], &[7; 32], CacheScope::Global)?;
    assert!(matches!(
        log.get_bounded(&[9; 32], 31),
        Err(CacheStoreError::TooLarge {
            bytes: 32,
            limit: 31
        })
    ));
    assert_eq!(log.get_bounded(&[9; 32], 32)?, Some(vec![7; 32]));
    assert_eq!(log.get_bounded(&[8; 32], 32)?, None);
    Ok(())
}
