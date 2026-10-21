//! Replay To Step (descriptor: specs/descriptors/eventlog_replay_to_step.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_storage_eventlog::{EVENTS_TABLE, EventLog, EventLogConfig, EventLogError};
use redb::TableDefinition;

type TestResult = Result<(), Box<dyn Error>>;

fn record(i: u8) -> EventRecord {
    EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::DeadLettered {
            message: Cid::from_bytes([i; 32]),
        },
    )
}

#[test]
fn replay_stops_at_until() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    for i in 0..20 {
        log.append(&record(i))?;
    }
    let replayed = log.replay(Seq::new(7))?;
    assert_eq!(
        replayed.iter().map(|r| r.seq().get()).collect::<Vec<_>>(),
        (1..=7).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn replay_from_checkpoint_matches_full_replay() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    for i in 0..30 {
        log.append(&record(i))?;
        if i == 9 || i == 19 {
            log.checkpoint(log.head()?, Cid::from_bytes([i; 32]))?;
        }
    }
    let until = Seq::new(25);
    let ckpt = log.latest_checkpoint(until)?.ok_or("no checkpoint")?;
    assert_eq!(ckpt.at, Seq::new(20));
    assert_eq!(ckpt.snapshot, Cid::from_bytes([19; 32]));
    let tail = log.read_range(ckpt.at.next().ok_or("seq")?, until)?;
    let full = log.replay(until)?;
    assert_eq!(
        tail.as_slice(),
        full.get(20..).ok_or("short replay")?,
        "snapshot + tail equals the full replay"
    );
    assert_eq!(
        log.latest_checkpoint(Seq::new(15))?.map(|c| c.at),
        Some(Seq::new(10))
    );
    assert_eq!(log.latest_checkpoint(Seq::new(5))?, None);
    Ok(())
}

#[test]
fn replay_empty_log_is_empty() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    assert!(log.replay(Seq::new(100))?.is_empty());
    assert_eq!(log.head()?, Seq::ZERO);
    Ok(())
}

#[test]
fn replay_rejects_newer_schema() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("events.redb");
    {
        let log = EventLog::open(EventLogConfig::new(&path))?;
        log.append(&record(1))?;
    }
    {
        // A record written by a future version: schema prefix 3.
        let db = redb::Database::open(&path)?;
        let txn = db.begin_write()?;
        {
            let mut table = txn.open_table(TableDefinition::<u64, &[u8]>::new(EVENTS_TABLE))?;
            let mut future = 3u16.to_le_bytes().to_vec();
            future.extend_from_slice(b"{\"from\":\"the future\"}");
            table.insert(2u64, future.as_slice())?;
        }
        txn.commit()?;
    }
    let log = EventLog::open(EventLogConfig::new(&path))?;
    assert!(matches!(
        log.replay(Seq::new(2)),
        Err(EventLogError::SchemaTooNew {
            found: 3,
            supported: 2
        })
    ));
    assert_eq!(
        log.replay(Seq::new(1))?.len(),
        1,
        "older records still decode"
    );
    Ok(())
}

#[test]
fn checkpoint_beyond_head_is_rejected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))?;
    log.append(&record(1))?;
    assert!(matches!(
        log.checkpoint(Seq::new(2), Cid::from_bytes([0; 32])),
        Err(EventLogError::CheckpointBeyondHead { .. })
    ));
    Ok(())
}
