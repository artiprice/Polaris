//! Event Append And Read (descriptor: specs/descriptors/eventlog_event_append_and_read.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, Seq, SessionId};
use polaris_sdk::registry::EventStore;
use polaris_storage_eventlog::{EventLog, EventLogConfig, EventLogError};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn record(i: u8) -> EventRecord {
    let event = Event::DiffApproved {
        frame: FrameId::from_u128(9),
        diff: Cid::from_bytes([i; 32]),
    };
    // A stale seq on input proves the log assigns its own.
    EventRecord::new(
        Seq::new(999),
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        event,
    )
}

fn open(dir: &tempfile::TempDir) -> Result<EventLog, EventLogError> {
    EventLog::open(EventLogConfig::new(dir.path().join("events.redb")))
}

#[test]
fn append_assigns_gap_free_seq() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = open(&dir)?;
    assert_eq!(log.head()?, Seq::ZERO);
    for expected in 1..=5u64 {
        assert_eq!(log.append(&record(1))?, Seq::new(expected));
    }
    assert_eq!(log.head()?, Seq::new(5));
    Ok(())
}

#[test]
fn read_range_is_ordered() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = open(&dir)?;
    for i in 1..=10u8 {
        log.append(&record(i))?;
    }
    let got = log.read_range(Seq::new(3), Seq::new(6))?;
    let seqs: Vec<u64> = got.iter().map(|r| r.seq().get()).collect();
    assert_eq!(seqs, vec![3, 4, 5, 6]);
    assert!(
        matches!(got.first().map(EventRecord::event), Some(Event::DiffApproved { diff, .. }) if *diff == Cid::from_bytes([3; 32]))
    );
    assert!(log.read_range(Seq::new(11), Seq::new(20))?.is_empty());
    assert!(log.read_range(Seq::new(6), Seq::new(3))?.is_empty());
    Ok(())
}

#[test]
fn batch_is_atomic() -> TestResult {
    let dir = tempfile::tempdir()?;
    let log = open(&dir)?;
    log.append(&record(1))?;
    assert_eq!(
        log.append_batch(&[record(2), record(3), record(4)])?,
        Seq::new(4)
    );
    assert_eq!(
        log.append_batch(&[])?,
        Seq::new(4),
        "an empty batch changes nothing"
    );
    assert_eq!(log.read_range(Seq::new(1), Seq::new(10))?.len(), 4);
    Ok(())
}

#[test]
fn reopen_preserves_head() -> TestResult {
    let dir = tempfile::tempdir()?;
    {
        let log = open(&dir)?;
        log.append_batch(&[record(1), record(2), record(3)])?;
    }
    let log = open(&dir)?;
    assert_eq!(log.head()?, Seq::new(3));
    assert_eq!(log.append(&record(4))?, Seq::new(4));
    assert_eq!(log.read_range(Seq::new(1), Seq::new(4))?.len(), 4);
    Ok(())
}

#[test]
fn second_open_is_locked() -> TestResult {
    let dir = tempfile::tempdir()?;
    let _first = open(&dir)?;
    assert!(matches!(open(&dir), Err(EventLogError::Locked)));
    Ok(())
}

#[test]
fn path_with_spaces_and_non_ascii_works() -> TestResult {
    let dir = tempfile::tempdir()?;
    let nested = dir.path().join("dir with space");
    std::fs::create_dir_all(&nested)?;
    let log = EventLog::open(EventLogConfig::new(nested.join("événements.redb")))?;
    log.append(&record(7))?;
    assert_eq!(log.read_range(Seq::new(1), Seq::new(1))?.len(), 1);
    Ok(())
}

#[test]
fn event_store_trait_is_implemented() -> TestResult {
    fn through_trait<S: EventStore>(store: &S) -> Result<Vec<EventRecord>, Box<dyn Error>> {
        store.append(&record(1))?;
        store.append(&record(2))?;
        Ok(store.read_range(Seq::new(1), Seq::new(2))?)
    }
    let dir = tempfile::tempdir()?;
    let log = open(&dir)?;
    assert_eq!(through_trait(&log)?.len(), 2);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn prop_appends_read_back_identically(diffs in prop::collection::vec(any::<u8>(), 0..40)) {
        let dir = tempfile::tempdir();
        prop_assert!(dir.is_ok());
        if let Ok(dir) = dir {
            let log = open(&dir);
            prop_assert!(log.is_ok());
            if let Ok(log) = log {
                for d in &diffs {
                    prop_assert!(log.append(&record(*d)).is_ok());
                }
                let back = log.read_range(Seq::new(1), Seq::new(1000));
                prop_assert!(back.is_ok());
                if let Ok(back) = back {
                    let got: Vec<u8> = back.iter().filter_map(|r| match r.event() {
                        Event::DiffApproved { diff, .. } => diff.as_bytes().first().copied(),
                        _ => None,
                    }).collect();
                    prop_assert_eq!(got, diffs);
                }
            }
        }
    }
}
