//! Log Then Publish (descriptor: specs/descriptors/bus_log_then_publish.json).

use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use polaris_kernel_bus::{Bus, BusConfig, BusError};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory store with gap-free sequence numbers and an injectable failure.
#[derive(Default)]
struct MemStore {
    records: Mutex<Vec<EventRecord>>,
    fail: AtomicBool,
}

impl MemStore {
    fn contains(&self, seq: Seq) -> bool {
        self.records
            .lock()
            .map(|r| r.iter().any(|e| e.seq() == seq))
            .unwrap_or(false)
    }
}

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(EventStoreError::Unavailable("injected failure".into()));
        }
        let mut records = self
            .records
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
            .records
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

fn config(events: usize) -> Result<BusConfig, Box<dyn Error>> {
    Ok(BusConfig {
        command_capacity: NonZeroUsize::MIN,
        event_capacity: NonZeroUsize::new(events).ok_or("zero")?,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publish_appends_before_fanout() -> TestResult {
    let store = Arc::new(MemStore::default());
    let (bus, _rx) = Bus::new(config(2048)?, Arc::clone(&store))?;
    let bus = Arc::new(bus);
    let mut stream = bus.subscribe();
    let watcher = {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            let mut checked = 0u32;
            while checked < 1000 {
                let ev = stream.next().await.map_err(|e| e.to_string())?;
                if !store.contains(ev.seq()) {
                    return Err(format!(
                        "event {:?} visible before it was persisted",
                        ev.seq()
                    ));
                }
                checked += 1;
            }
            Ok::<u32, String>(checked)
        })
    };
    let publisher = {
        let bus = Arc::clone(&bus);
        std::thread::spawn(move || (0..1000).try_for_each(|_| bus.publish(record()).map(|_| ())))
    };
    publisher.join().map_err(|_| "publisher panicked")??;
    assert_eq!(watcher.await??, 1000);
    Ok(())
}

#[test]
fn store_error_suppresses_fanout() -> TestResult {
    let store = Arc::new(MemStore::default());
    let (bus, _rx) = Bus::new(config(16)?, Arc::clone(&store))?;
    let mut stream = bus.subscribe();
    store.fail.store(true, Ordering::SeqCst);
    assert!(matches!(bus.publish(record()), Err(BusError::Store(_))));
    assert_eq!(
        stream.try_next()?,
        None,
        "a failed append must not be broadcast"
    );
    store.fail.store(false, Ordering::SeqCst);
    assert_eq!(bus.publish(record())?, Seq::new(1));
    assert_eq!(stream.try_next()?.map(|e| e.seq()), Some(Seq::new(1)));
    Ok(())
}

#[test]
fn seq_is_strictly_increasing() -> TestResult {
    let store = Arc::new(MemStore::default());
    let (bus, _rx) = Bus::new(config(8192)?, Arc::clone(&store))?;
    let bus = Arc::new(bus);
    let mut stream = bus.subscribe();
    let writers: Vec<_> = (0..8)
        .map(|_| {
            let bus = Arc::clone(&bus);
            std::thread::spawn(move || (0..500).try_for_each(|_| bus.publish(record()).map(|_| ())))
        })
        .collect();
    for w in writers {
        w.join().map_err(|_| "writer panicked")??;
    }
    let mut seen = Vec::new();
    while let Some(ev) = stream.try_next()? {
        seen.push(ev.seq().get());
    }
    let expected: Vec<u64> = (1..=4000).collect();
    assert_eq!(
        seen, expected,
        "fan-out order must equal persisted order, gap-free"
    );
    assert_eq!(store.read_range(Seq::new(1), Seq::new(4000))?.len(), 4000);
    assert_eq!(bus.stats().events_published, 4000);
    Ok(())
}

#[test]
fn no_receivers_is_not_an_error() -> TestResult {
    let (bus, _rx) = Bus::new(config(16)?, MemStore::default())?;
    assert_eq!(bus.publish(record())?, Seq::new(1));
    assert_eq!(bus.stats().subscribers, 0);
    assert_eq!(bus.stats().published_without_receivers, 1);
    Ok(())
}
