//! Lag Recovery (descriptor: specs/descriptors/bus_lag_recovery.json).

use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use polaris_kernel_bus::{Bus, BusConfig, BusError};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory store with gap-free sequence numbers.
#[derive(Default)]
struct MemStore(Mutex<Vec<EventRecord>>);

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        let mut records = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        let seq = Seq::new(
            u64::try_from(records.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        );
        records.push(record.with_seq(seq));
        Ok(seq)
    }
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        let records = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        Ok(records
            .iter()
            .filter(|r| r.seq() >= from && r.seq() <= to)
            .copied()
            .collect())
    }
}

fn record() -> EventRecord {
    EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(7),
        Event::SessionClosed,
    )
}

fn small() -> Result<BusConfig, Box<dyn Error>> {
    Ok(BusConfig {
        command_capacity: NonZeroUsize::MIN,
        event_capacity: NonZeroUsize::new(4).ok_or("zero")?,
    })
}

#[tokio::test]
async fn lagged_reports_missed_and_resume_seq() -> TestResult {
    let (bus, _rx) = Bus::new(small()?, MemStore::default())?;
    let mut stream = bus.subscribe();
    for _ in 0..10 {
        bus.publish(record())?;
    }
    match stream.next().await {
        Err(BusError::Lagged {
            missed,
            resume_from,
        }) => {
            assert_eq!(missed, 6);
            assert_eq!(resume_from, Seq::new(1));
        }
        other => return Err(format!("expected Lagged, got {other:?}").into()),
    }
    assert_eq!(
        stream.next().await?.seq(),
        Seq::new(7),
        "the first retained event follows the lag report"
    );
    assert_eq!(bus.stats().lagged, 1);
    Ok(())
}

#[tokio::test]
async fn resume_from_store_replays_missed_events() -> TestResult {
    let store = Arc::new(MemStore::default());
    let (bus, _rx) = Bus::new(small()?, Arc::clone(&store))?;
    let mut stream = bus.subscribe();
    for _ in 0..10 {
        bus.publish(record())?;
    }
    let mut seen: Vec<u64> = Vec::new();
    loop {
        match stream.try_next() {
            Ok(Some(ev)) => seen.push(ev.seq().get()),
            Ok(None) => break,
            Err(BusError::Lagged {
                missed,
                resume_from,
            }) => {
                let to = Seq::new(resume_from.get() + missed - 1);
                seen.extend(
                    store
                        .read_range(resume_from, to)?
                        .iter()
                        .map(|e| e.seq().get()),
                );
            }
            Err(other) => return Err(other.into()),
        }
    }
    assert_eq!(
        seen,
        (1..=10).collect::<Vec<_>>(),
        "every event exactly once, in order"
    );
    Ok(())
}

#[tokio::test]
async fn closed_stream_returns_closed() -> TestResult {
    let (bus, rx) = Bus::new(small()?, MemStore::default())?;
    let mut stream = bus.subscribe();
    drop(bus);
    drop(rx);
    assert!(matches!(stream.next().await, Err(BusError::Closed)));
    assert!(matches!(stream.try_next(), Err(BusError::Closed)));
    Ok(())
}
