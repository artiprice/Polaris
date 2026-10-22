//! Slow-subscriber isolation and UI subscribe/drop churn.

use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use polaris_kernel_bus::{Bus, BusConfig, BusError};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};

#[derive(Default)]
struct CountingStore(AtomicU64);

impl EventStore for CountingStore {
    fn append(&self, _record: &EventRecord) -> Result<Seq, EventStoreError> {
        Ok(Seq::new(
            self.0.fetch_add(1, Ordering::Relaxed).saturating_add(1),
        ))
    }

    fn read_range(&self, _from: Seq, _to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(Vec::new())
    }
}

fn record() -> EventRecord {
    EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(3),
        Event::SessionClosed,
    )
}

fn config(event_capacity: usize) -> Result<BusConfig, Box<dyn Error>> {
    Ok(BusConfig {
        command_capacity: NonZeroUsize::new(64).ok_or("zero command capacity")?,
        event_capacity: NonZeroUsize::new(event_capacity).ok_or("zero event capacity")?,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fast_subscriber_keeps_up_while_stalled_subscriber_lags() -> Result<(), Box<dyn Error>> {
    let (bus, _commands) = Bus::new(config(8)?, CountingStore::default())?;
    let bus = Arc::new(bus);
    let mut stalled = bus.subscribe();
    let mut fast = bus.subscribe();
    let (acknowledged_tx, mut acknowledged_rx) = tokio::sync::mpsc::channel(1);
    let fast_reader = tokio::spawn(async move {
        let mut received = 0u64;
        while received < 1_000 {
            match fast.next().await {
                Ok(_) => {
                    received = received.saturating_add(1);
                    if acknowledged_tx.send(received).await.is_err() {
                        return Err(BusError::Closed);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok::<u64, BusError>(received)
    });
    for _ in 0..1_000 {
        bus.publish(record())?;
        if acknowledged_rx.recv().await.is_none() {
            return Err("fast subscriber stopped before receiving the full stream".into());
        }
    }
    assert_eq!(fast_reader.await??, 1_000);
    assert!(matches!(stalled.next().await, Err(BusError::Lagged { .. })));
    assert_eq!(bus.stats().events_published, 1_000);
    assert_eq!(bus.stats().lagged, 1);
    Ok(())
}

#[test]
fn subscriber_churn_releases_all_receivers() -> Result<(), Box<dyn Error>> {
    let (bus, _commands) = Bus::new(config(16)?, CountingStore::default())?;
    for _ in 0..1_000 {
        drop(bus.subscribe());
        bus.publish(record())?;
    }
    assert_eq!(bus.stats().subscribers, 0);
    assert_eq!(bus.stats().published_without_receivers, 1_000);
    assert_eq!(bus.stats().events_published, 1_000);
    Ok(())
}
