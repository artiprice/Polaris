//! Command Backpressure (descriptor: specs/descriptors/bus_command_backpressure.json).

use std::collections::VecDeque;
use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use polaris_kernel_bus::{Bus, BusConfig, TryRecvError, TrySendError};
use polaris_sdk::contracts::{
    Command, CommandBody, CommandId, EventRecord, TokenBytes, Unverified,
};
use polaris_sdk::ids::{FrameId, Seq};
use polaris_sdk::registry::{EventStore, EventStoreError};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

/// Hands out sequence numbers only (commands never touch the store).
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

fn config(commands: usize, events: usize) -> Result<BusConfig, Box<dyn Error>> {
    Ok(BusConfig {
        command_capacity: NonZeroUsize::new(commands).ok_or("zero")?,
        event_capacity: NonZeroUsize::new(events).ok_or("zero")?,
    })
}

#[test]
fn try_send_returns_full_at_capacity() -> TestResult {
    let (bus, _rx) = Bus::new(config(2, 8)?, CountingStore::default())?;
    bus.try_send(command(1))?;
    bus.try_send(command(2))?;
    match bus.try_send(command(3)) {
        Err(TrySendError::Full(cmd)) => assert_eq!(
            cmd.id(),
            command(3).id(),
            "the rejected command is handed back"
        ),
        other => return Err(format!("expected Full, got {other:?}").into()),
    }
    assert_eq!(bus.stats().rejected_full, 1);
    assert_eq!(bus.stats().commands_sent, 2);
    Ok(())
}

#[test]
fn try_send_after_close_returns_closed() -> TestResult {
    let (bus, rx) = Bus::new(config(4, 8)?, CountingStore::default())?;
    drop(rx);
    assert!(
        matches!(bus.try_send(command(1)), Err(TrySendError::Closed(c)) if c.id() == command(1).id())
    );
    Ok(())
}

#[tokio::test]
async fn receiver_close_rejects_new_commands_and_drains_accepted_ones() -> TestResult {
    let (bus, mut rx) = Bus::new(config(4, 8)?, CountingStore::default())?;
    bus.try_send(command(1))?;
    bus.try_send(command(2))?;
    rx.close();

    assert!(matches!(
        bus.try_send(command(3)),
        Err(TrySendError::Closed(cmd)) if cmd.id() == command(3).id()
    ));
    assert_eq!(rx.recv().await.map(|cmd| cmd.id()), Some(command(1).id()));
    assert_eq!(rx.recv().await.map(|cmd| cmd.id()), Some(command(2).id()));
    assert_eq!(rx.recv().await, None);
    Ok(())
}

#[test]
fn try_send_never_blocks() -> TestResult {
    // No async runtime exists in this test: try_send/try_recv must work without one.
    let (bus, mut rx) = Bus::new(config(1, 8)?, CountingStore::default())?;
    bus.try_send(command(1))?;
    assert!(matches!(
        bus.try_send(command(2)),
        Err(TrySendError::Full(_))
    ));
    assert_eq!(rx.try_recv().map(|c| c.id()), Ok(command(1).id()));
    assert_eq!(rx.try_recv().map(|c| c.id()), Err(TryRecvError::Empty));
    Ok(())
}

#[tokio::test]
async fn send_waits_for_capacity() -> TestResult {
    let (bus, mut rx) = Bus::new(config(1, 8)?, CountingStore::default())?;
    let bus = Arc::new(bus);
    bus.try_send(command(1))?;
    let producer = {
        let bus = Arc::clone(&bus);
        tokio::spawn(async move { bus.send(command(2)).await.map_err(|e| e.0.id()) })
    };
    tokio::task::yield_now().await;
    assert_eq!(rx.recv().await.map(|c| c.id()), Some(command(1).id()));
    assert_eq!(producer.await?, Ok(()));
    assert_eq!(rx.recv().await.map(|c| c.id()), Some(command(2).id()));
    drop(bus);
    assert!(
        rx.recv().await.is_none(),
        "receiver ends when the bus is dropped"
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_commands_stay_fifo(ops in prop::collection::vec(any::<bool>(), 1..200)) {
        let cfg = BusConfig { command_capacity: NonZeroUsize::MIN.saturating_add(7), event_capacity: NonZeroUsize::MIN.saturating_add(7) };
        let created = Bus::new(cfg, CountingStore::default());
        prop_assert!(created.is_ok());
        if let Ok((bus, mut rx)) = created {
            let mut model: VecDeque<u128> = VecDeque::new();
            let mut next = 0u128;
            for send in ops {
                if send {
                    next += 1;
                    match bus.try_send(command(next)) {
                        Ok(()) => model.push_back(next),
                        Err(TrySendError::Full(_)) => prop_assert_eq!(model.len(), 8),
                        Err(TrySendError::Closed(_)) => prop_assert!(false, "closed"),
                    }
                } else {
                    let got = rx.try_recv().ok().map(|c| c.id());
                    let want = model.pop_front().map(|i| command(i).id());
                    prop_assert_eq!(got, want);
                }
            }
        }
    }
}
