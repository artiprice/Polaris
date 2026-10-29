//! Bus Zero Alloc Hot Path (descriptor: specs/descriptors/bus_zero_alloc_hot_path.json; PLAN ADR-003).
//! One integration-test binary per zero-alloc check: dhat allows one profiler per process.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

use polaris_kernel_bus::{Bus, BusConfig};
use polaris_sdk::contracts::{
    Command, CommandBody, CommandId, Event, EventRecord, Timestamp, TokenBytes, Unverified,
};
use polaris_sdk::ids::{FrameId, Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// A store that keeps nothing (so it does not allocate): only hands out sequence numbers.
#[derive(Default)]
struct CountingStore(AtomicU64);

impl EventStore for CountingStore {
    fn append(&self, _record: &EventRecord) -> Result<Seq, EventStoreError> {
        Ok(Seq::new(
            self.0.fetch_add(1, Ordering::SeqCst).saturating_add(1),
        ))
    }
    fn read_range(&self, _from: Seq, _to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(Vec::new())
    }
}

fn command(i: u128) -> Command<Unverified> {
    Command::new(
        CommandId::from_u128(i),
        FrameId::from_u128(1),
        TokenBytes::from_static(b"capability"),
        CommandBody::CancelFrame {
            frame: FrameId::from_u128(2),
        },
    )
}

#[test]
fn hot_path_does_not_allocate_after_warmup() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let cfg = BusConfig {
        command_capacity: NonZeroUsize::new(64).ok_or("zero")?,
        event_capacity: NonZeroUsize::new(64).ok_or("zero")?,
    };
    let (bus, mut rx) = Bus::new(cfg, CountingStore::default())?;
    let mut stream = bus.subscribe();
    let record = EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(7),
        Event::SessionClosed,
    );
    let commands: Vec<_> = (0..2000).map(command).collect();
    let mut commands = commands.into_iter();

    // Warm-up: let the channels allocate and recycle their internal blocks.
    for cmd in commands.by_ref().take(1000) {
        bus.try_send(cmd).map_err(|_| "warm-up send failed")?;
        rx.try_recv().map_err(|_| "warm-up recv failed")?;
        bus.publish(record)?;
        stream.try_next()?;
    }

    let before = dhat::HeapStats::get();
    for cmd in commands {
        bus.try_send(cmd).map_err(|_| "send failed")?;
        rx.try_recv().map_err(|_| "recv failed")?;
        bus.publish(record)?;
        stream.try_next()?;
    }
    let after = dhat::HeapStats::get();
    assert_eq!(
        after.total_blocks, before.total_blocks,
        "bus hot path allocated after warm-up"
    );
    Ok(())
}
