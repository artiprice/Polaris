//! Command flood: every offered command is accepted or explicitly returned by backpressure.

use std::error::Error;
use std::num::NonZeroUsize;

use polaris_kernel_bus::{Bus, BusConfig, TrySendError};
use polaris_sdk::contracts::{
    Command, CommandBody, CommandId, EventRecord, TokenBytes, Unverified,
};
use polaris_sdk::ids::{FrameId, Seq};
use polaris_sdk::registry::{EventStore, EventStoreError};
use std::sync::atomic::{AtomicU64, Ordering};

const OFFERED: u128 = 100_000;
const CAPACITY: usize = 256;

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

fn command(id: u128) -> Command<Unverified> {
    Command::new(
        CommandId::from_u128(id),
        FrameId::from_u128(1),
        TokenBytes::from_static(b"spike-capability"),
        CommandBody::CancelFrame {
            frame: FrameId::from_u128(2),
        },
    )
}

#[tokio::test]
async fn bounded_command_flood_accounts_for_every_offer() -> Result<(), Box<dyn Error>> {
    let config = BusConfig {
        command_capacity: NonZeroUsize::new(CAPACITY).ok_or("zero command capacity")?,
        event_capacity: NonZeroUsize::new(16).ok_or("zero event capacity")?,
    };
    let (bus, mut receiver) = Bus::new(config, CountingStore::default())?;
    let mut accepted = 0u128;
    let mut rejected = 0u128;
    for id in 0..OFFERED {
        match bus.try_send(command(id)) {
            Ok(()) => accepted = accepted.saturating_add(1),
            Err(TrySendError::Full(returned)) => {
                assert_eq!(returned.id(), CommandId::from_u128(id));
                rejected = rejected.saturating_add(1);
            }
            Err(TrySendError::Closed(_)) => {
                return Err("receiver closed before the explicit close".into());
            }
        }
    }
    receiver.close();
    let mut drained = 0u128;
    while receiver.recv().await.is_some() {
        drained = drained.saturating_add(1);
    }
    assert_eq!(accepted, u128::try_from(CAPACITY)?);
    assert_eq!(drained, accepted);
    assert_eq!(accepted.saturating_add(rejected), OFFERED);
    assert_eq!(bus.stats().rejected_full, u64::try_from(rejected)?);
    Ok(())
}
