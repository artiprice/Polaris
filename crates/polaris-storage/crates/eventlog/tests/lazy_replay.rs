//! Bounded replay captures one immutable prefix on real redb.
use polaris_sdk::{
    contracts::{Event, EventRecord, Timestamp},
    ids::{Seq, SessionId},
};
use polaris_storage_eventlog::{EventLog, EventLogConfig};

#[test]
fn lazy_replay_crosses_pages_and_excludes_new_appends() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let log = EventLog::open(EventLogConfig::new(root.path().join("events.redb")))?;
    let record = EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    );
    log.append_batch(&vec![record; 1000])?;
    let replay = log.replay_iter(Seq::new(u64::MAX))?;
    log.append(&record)?;
    let records = replay.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(records.len(), 1000);
    for (record, expected) in records.iter().zip(1_u64..) {
        assert_eq!(record.seq().get(), expected);
    }
    assert_eq!(
        log.replay_iter(Seq::new(500))?
            .collect::<Result<Vec<_>, _>>()?
            .len(),
        500
    );
    Ok(())
}
