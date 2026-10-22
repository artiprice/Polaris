//! One-million-event flood with 1, 8, and 64 active subscribers.

use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use hdrhistogram::Histogram;
use polaris_kernel_bus::{Bus, BusConfig};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};
use sysinfo::{Pid, ProcessesToUpdate, System};

const EVENTS: u64 = 1_000_000;
const EVENT_CAPACITY: usize = 1_048_576;
const MAX_RSS_DELTA_BYTES: u64 = 512 * 1024 * 1024;
static FLOOD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Default)]
struct CountingStore(AtomicU64);

impl EventStore for CountingStore {
    fn append(&self, _record: &EventRecord) -> Result<Seq, EventStoreError> {
        let sequence = self.0.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        Ok(Seq::new(sequence))
    }

    fn read_range(&self, _from: Seq, _to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(Vec::new())
    }
}

fn config() -> Result<BusConfig, Box<dyn Error>> {
    Ok(BusConfig {
        command_capacity: NonZeroUsize::new(1_024).ok_or("zero command capacity")?,
        event_capacity: NonZeroUsize::new(EVENT_CAPACITY).ok_or("zero event capacity")?,
    })
}

fn event() -> EventRecord {
    EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    )
}

fn resident_bytes() -> Option<u64> {
    let mut system = System::new();
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}

async fn flood(subscriber_count: usize) -> Result<(), Box<dyn Error>> {
    let _exclusive = FLOOD_LOCK.lock().map_err(|_| "flood lock poisoned")?;
    let rss_before = resident_bytes();
    let (bus, _commands) = Bus::new(config()?, CountingStore::default())?;
    let bus = Arc::new(bus);
    let mut readers = Vec::with_capacity(subscriber_count);
    for _ in 0..subscriber_count {
        let mut stream = bus.subscribe();
        readers.push(tokio::spawn(async move {
            let mut received = 0u64;
            while received < EVENTS {
                match stream.next().await {
                    Ok(_) => received = received.saturating_add(1),
                    Err(error) => return Err(error),
                }
            }
            Ok::<u64, polaris_kernel_bus::BusError>(received)
        }));
    }

    let publishing_bus = Arc::clone(&bus);
    let publish = tokio::task::spawn_blocking(move || {
        let mut histogram = Histogram::<u64>::new(3).map_err(|err| err.to_string())?;
        let record = event();
        let started = Instant::now();
        for _ in 0..EVENTS {
            let each = Instant::now();
            publishing_bus
                .publish(record)
                .map_err(|err| err.to_string())?;
            let elapsed_ns = match u64::try_from(each.elapsed().as_nanos()) {
                Ok(value) => value.max(1),
                Err(_) => u64::MAX,
            };
            histogram
                .record(elapsed_ns)
                .map_err(|err| err.to_string())?;
        }
        Ok::<(std::time::Duration, u64, u64), String>((
            started.elapsed(),
            histogram.value_at_quantile(0.50),
            histogram.value_at_quantile(0.99),
        ))
    });
    let (elapsed, p50_ns, p99_ns) = publish.await??;
    for reader in readers {
        assert_eq!(reader.await??, EVENTS);
    }
    assert_eq!(bus.stats().events_published, EVENTS);
    let rss_after = resident_bytes();
    let rss_delta = match (rss_before, rss_after) {
        (Some(before), Some(after)) => Some(after.saturating_sub(before)),
        _ => None,
    };
    if let Some(delta) = rss_delta {
        assert!(
            delta <= MAX_RSS_DELTA_BYTES,
            "resident-memory delta {delta} exceeded the configured bounded-flood budget"
        );
    }
    eprintln!(
        "bus flood: subscribers={subscriber_count} events={EVENTS} elapsed={elapsed:?} publish_p50={p50_ns}ns publish_p99={p99_ns}ns rss_delta_bytes={rss_delta:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn million_event_flood_with_one_subscriber() -> Result<(), Box<dyn Error>> {
    flood(1).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn million_event_flood_with_eight_subscribers() -> Result<(), Box<dyn Error>> {
    flood(8).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn million_event_flood_with_sixty_four_subscribers() -> Result<(), Box<dyn Error>> {
    flood(64).await
}
