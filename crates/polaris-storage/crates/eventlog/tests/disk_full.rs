//! Atomic rollback under an actual limited-filesystem append failure.
use polaris_sdk::{
    contracts::{Event, EventRecord, Timestamp},
    ids::{Seq, SessionId},
};
use polaris_storage_eventlog::{EventLog, EventLogConfig, EventLogError};
#[test]
#[ignore = "requires EVENTLOG_TEST_FULL_DIRECTORY on a disposable 16MiB filesystem"]
fn append_failure_preserves_committed_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("EVENTLOG_TEST_FULL_DIRECTORY")
        .ok_or("missing explicit limited test filesystem")?;
    let directory = tempfile::tempdir_in(root)?;
    let log = EventLog::open(EventLogConfig::new(directory.path().join("events.redb")))?;
    let record = EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    );
    let batch = vec![record; 1000];
    let mut committed = Seq::ZERO;
    let mut exhausted = false;
    for _ in 0..1000 {
        match log.append_batch(&batch) {
            Ok(head) => committed = head,
            Err(error) => {
                eprintln!("actual limited-filesystem append failure: {error}");
                assert!(matches!(error, EventLogError::Db(_)));
                exhausted = true;
                break;
            }
        }
    }
    assert!(
        exhausted,
        "limited filesystem must exhaust before test bound"
    );
    assert_eq!(
        log.head()?,
        committed,
        "failed batch cannot advance the durable head"
    );
    let records = log.replay_iter(committed)?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(u64::try_from(records.len())?, committed.get());
    for (expected, record) in (1_u64..).zip(records) {
        assert_eq!(record.seq().get(), expected);
    }
    Ok(())
}
