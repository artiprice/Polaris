//! Lock Table Recovery (descriptor: specs/descriptors/eventlog_lock_table_recovery.json).

use std::error::Error;
use std::path::Path;

use polaris_sdk::ids::FrameId;
use polaris_storage_eventlog::{EventLog, EventLogConfig, EventLogError};

type TestResult = Result<(), Box<dyn Error>>;

fn open_as(path: &Path, owner_pid: u32) -> Result<EventLog, EventLogError> {
    let mut cfg = EventLogConfig::new(path);
    cfg.owner_pid = owner_pid;
    EventLog::open(cfg)
}

#[test]
fn stale_locks_listed_on_open() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("events.redb");
    {
        let crashed = open_as(&path, 111)?;
        crashed.acquire_lock("src/lib.rs", FrameId::from_u128(1))?;
        crashed.acquire_lock("src/main.rs::run", FrameId::from_u128(2))?;
    }
    let recovered = open_as(&path, 222)?;
    let stale = recovered.stale_locks()?;
    assert_eq!(
        stale.len(),
        2,
        "locks of the dead owner are listed, not dropped"
    );
    assert!(stale.iter().all(|l| l.owner_pid == 111));
    let mine = recovered.acquire_lock("README.md", FrameId::from_u128(3))?;
    assert_eq!(mine.owner_pid, 222);
    assert_eq!(recovered.stale_locks()?.len(), 2, "own locks are not stale");
    Ok(())
}

#[test]
fn acquire_conflict_is_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = open_as(&dir.path().join("events.redb"), 1)?;
    log.acquire_lock("src/lib.rs", FrameId::from_u128(1))?;
    assert!(matches!(
        log.acquire_lock("src/lib.rs", FrameId::from_u128(2)),
        Err(EventLogError::LockConflict { holder, .. }) if holder == FrameId::from_u128(1)
    ));
    Ok(())
}

#[test]
fn release_removes_row() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = open_as(&dir.path().join("events.redb"), 1)?;
    let lease = log.acquire_lock("src/lib.rs", FrameId::from_u128(1))?;
    assert!(matches!(
        log.release_lock("src/lib.rs", lease.lease + 100),
        Err(EventLogError::UnknownLease { .. })
    ));
    log.release_lock("src/lib.rs", lease.lease)?;
    assert!(log.locks()?.is_empty());
    log.acquire_lock("src/lib.rs", FrameId::from_u128(2))?;
    Ok(())
}

#[test]
fn lock_survives_reopen() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("events.redb");
    let lease = {
        let log = open_as(&path, 1)?;
        log.acquire_lock("src/lib.rs", FrameId::from_u128(1))?
    };
    let log = open_as(&path, 1)?;
    assert_eq!(log.locks()?, vec![lease.clone()]);
    let next = log.acquire_lock("other.rs", FrameId::from_u128(1))?;
    assert!(
        next.lease > lease.lease,
        "lease numbers keep increasing across reopen"
    );
    Ok(())
}
