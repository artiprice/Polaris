//! Random bounded bus operations compared with FIFO and durable-sequence models.
#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_kernel_bus::{Bus, BusConfig, BusError, TryRecvError, TrySendError};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event, EventRecord, Timestamp, TokenBytes},
    ids::{CommandId, FrameId, Seq, SessionId},
    registry::{EventStore, EventStoreError},
};
use std::{collections::VecDeque, num::NonZeroUsize, sync::Mutex};

// Test-only in-memory persistence seam; production continues to use real redb.
#[derive(Default)]
struct ModelStore(Mutex<Vec<EventRecord>>);
impl EventStore for ModelStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        let mut records = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Codec("model poisoned".to_owned()))?;
        let next = u64::try_from(records.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| EventStoreError::Codec("model count overflow".to_owned()))?;
        let seq = Seq::new(next);
        records.push(record.with_seq(seq));
        Ok(seq)
    }
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        let records = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Codec("model poisoned".to_owned()))?;
        Ok(records
            .iter()
            .filter(|record| record.seq() >= from && record.seq() <= to)
            .copied()
            .collect())
    }
}
fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let capacity = NonZeroUsize::MIN.saturating_add(3);
    let Ok((bus, mut receiver)) = Bus::new(
        BusConfig {
            command_capacity: capacity,
            event_capacity: capacity,
        },
        ModelStore::default(),
    ) else {
        return;
    };
    let mut expected = VecDeque::new();
    let mut streams = Vec::new();
    let mut closed = false;
    let mut published = 0_u64;
    for (index, operation) in (0_u128..).zip(data.iter().copied()) {
        match operation % 6 {
            0 => {
                let id = CommandId::from_u128(index);
                let command = Command::new(
                    id,
                    FrameId::from_u128(1),
                    TokenBytes::from(vec![1]),
                    CommandBody::CancelFrame {
                        frame: FrameId::from_u128(1),
                    },
                );
                match bus.try_send(command) {
                    Ok(()) => {
                        assert!(!closed && expected.len() < capacity.get());
                        expected.push_back(id);
                    }
                    Err(TrySendError::Full(returned)) => {
                        assert_eq!(returned.id(), id);
                        assert!(!closed && expected.len() == capacity.get());
                    }
                    Err(TrySendError::Closed(returned)) => {
                        assert_eq!(returned.id(), id);
                        assert!(closed);
                    }
                }
            }
            1 => match receiver.try_recv() {
                Ok(command) => assert_eq!(expected.pop_front(), Some(command.id())),
                Err(TryRecvError::Empty) => {
                    assert!(expected.is_empty());
                    assert!(!closed);
                }
                Err(TryRecvError::Closed) => {
                    assert!(expected.is_empty());
                    assert!(closed);
                }
            },
            2 => {
                let record = EventRecord::new(
                    Seq::ZERO,
                    Timestamp::UNIX_EPOCH,
                    SessionId::from_u128(1),
                    Event::SessionClosed,
                );
                let result = bus.publish(record);
                published = published.saturating_add(1);
                assert_eq!(result.ok(), Some(Seq::new(published)));
                let durable = bus
                    .store()
                    .read_range(Seq::new(published), Seq::new(published));
                assert_eq!(
                    durable
                        .ok()
                        .and_then(|records| records.first().copied())
                        .map(|record| record.seq()),
                    Some(Seq::new(published))
                );
            }
            3 if streams.len() < 8 => streams.push((bus.subscribe(), 0_u64)),
            4 => {
                for (stream, seen) in &mut streams {
                    match stream.try_next() {
                        Ok(Some(record)) => {
                            assert!(record.seq().get() > *seen);
                            assert!(record.seq().get() <= published);
                            *seen = record.seq().get();
                        }
                        Ok(None) => {}
                        Err(BusError::Lagged { resume_from, .. }) => {
                            assert!(resume_from.get() <= published)
                        }
                        Err(error) => assert!(
                            matches!(&error, BusError::Lagged { .. }),
                            "unexpected model bus error: {error}"
                        ),
                    }
                }
            }
            5 => {
                receiver.close();
                closed = true;
            }
            _ => {}
        }
    }
});
