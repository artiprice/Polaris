//! Dead Letter Queue (descriptor: specs/descriptors/supervisor_dead_letter_queue.json).

use std::error::Error;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use polaris_kernel_bus::{Bus, BusConfig};
use polaris_kernel_supervisor::{
    ChildName, DeadLetter, DeadLetterQueue, ExitClass, Supervisor, SupervisorConfig,
};
use polaris_sdk::contracts::{Event, EventRecord};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};
use proptest::prelude::*;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory store with gap-free sequence numbers.
#[derive(Default)]
struct MemStore {
    records: Mutex<Vec<EventRecord>>,
}

impl MemStore {
    fn events(&self) -> Vec<Event> {
        self.records
            .lock()
            .map(|r| r.iter().map(|rec| *rec.event()).collect())
            .unwrap_or_default()
    }
}

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
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

fn cid(n: u8) -> Cid {
    Cid::from_bytes([n; 32])
}

fn letter(item: u32) -> DeadLetter<u32> {
    DeadLetter {
        item,
        child: ChildName::new("test-child", cid(1)),
        reason: ExitClass::Panicked,
        attempts: 3,
    }
}

fn cap(n: usize) -> Result<NonZeroUsize, Box<dyn Error>> {
    NonZeroUsize::new(n).ok_or_else(|| "zero capacity".into())
}

fn supervisor(
    dlq_capacity: usize,
) -> Result<(Supervisor<Arc<MemStore>>, Arc<MemStore>), Box<dyn Error>> {
    let store = Arc::new(MemStore::default());
    let (bus, _commands) = Bus::new(BusConfig::default(), Arc::clone(&store))?;
    let config = SupervisorConfig {
        dlq_capacity: cap(dlq_capacity)?,
        ..SupervisorConfig::default()
    };
    let sup = Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(9),
        CancellationToken::new(),
        config,
    );
    Ok((sup, store))
}

#[test]
fn fifo_order_within_capacity() -> TestResult {
    let mut q = DeadLetterQueue::new(cap(4)?);
    for i in 0..3 {
        assert!(q.push(letter(i)).is_none());
    }
    let items: Vec<u32> = q.drain().into_iter().map(|l| l.item).collect();
    assert_eq!(items, vec![0, 1, 2]);
    Ok(())
}

#[test]
fn bounded_drops_oldest_and_counts() -> TestResult {
    let mut q = DeadLetterQueue::new(cap(2)?);
    assert!(q.push(letter(0)).is_none());
    assert!(q.push(letter(1)).is_none());
    let evicted = q.push(letter(2));
    assert_eq!(evicted.map(|l| l.item), Some(0));
    assert_eq!(q.len(), 2);
    assert_eq!(q.capacity(), 2);
    assert_eq!(q.dropped(), 1);
    let evicted = q.push(letter(3));
    assert_eq!(evicted.map(|l| l.item), Some(1));
    assert_eq!(q.dropped(), 2);
    let items: Vec<u32> = q.drain().into_iter().map(|l| l.item).collect();
    assert_eq!(items, vec![2, 3]);
    Ok(())
}

#[test]
fn drain_empties_and_keeps_drop_counter() -> TestResult {
    let mut q = DeadLetterQueue::new(cap(1)?);
    q.push(letter(0));
    q.push(letter(1));
    assert_eq!(q.dropped(), 1);
    assert_eq!(q.drain().len(), 1);
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    assert_eq!(q.dropped(), 1);
    assert!(q.drain().is_empty());
    Ok(())
}

proptest! {
    #[test]
    fn prop_len_bounded_and_accounted(capacity in 1_usize..16, pushes in 0_usize..100) {
        let mut q = DeadLetterQueue::new(NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::MIN));
        for i in 0..pushes {
            q.push(letter(u32::try_from(i).unwrap_or(u32::MAX)));
            prop_assert!(q.len() <= capacity);
        }
        prop_assert_eq!(
            u64::try_from(q.len()).unwrap_or(u64::MAX) + q.dropped(),
            u64::try_from(pushes).unwrap_or(u64::MAX)
        );
    }
}

#[tokio::test]
async fn supervisor_dead_letter_publishes_event() -> TestResult {
    let (sup, store) = supervisor(4)?;
    let message = cid(7);
    sup.dead_letter(
        message,
        ChildName::new("indexer", cid(2)),
        ExitClass::Failed,
        4,
    )
    .await;
    assert_eq!(store.events(), vec![Event::DeadLettered { message }]);
    let letters = sup.drain_dead_letters();
    assert_eq!(letters.len(), 1);
    let first = letters.first().ok_or("no letter")?;
    assert_eq!(first.item, message);
    assert_eq!(first.attempts, 4);
    assert_eq!(first.reason, ExitClass::Failed);
    assert_eq!(first.child.label(), "indexer");
    assert!(sup.drain_dead_letters().is_empty());
    Ok(())
}

#[tokio::test]
async fn supervisor_reports_drops_in_snapshot() -> TestResult {
    let (sup, store) = supervisor(2)?;
    for n in 0..5_u8 {
        sup.dead_letter(
            cid(n),
            ChildName::new("indexer", cid(2)),
            ExitClass::Failed,
            1,
        )
        .await;
    }
    let snap = sup.snapshot();
    assert_eq!(snap.dlq_len, 2);
    assert_eq!(snap.dlq_dropped, 3);
    // Every dead letter is still announced, even those later evicted from the queue.
    assert_eq!(store.events().len(), 5);
    let kept: Vec<Cid> = sup
        .drain_dead_letters()
        .into_iter()
        .map(|l| l.item)
        .collect();
    assert_eq!(kept, vec![cid(3), cid(4)]);
    Ok(())
}
