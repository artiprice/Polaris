//! The bus: a bounded command queue in, a broadcast of persisted events out.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use polaris_sdk::contracts::{Command, EventRecord, Unverified};
use polaris_sdk::ids::Seq;
use polaris_sdk::registry::EventStore;
use tokio::sync::{broadcast, mpsc};

use crate::error::{BusError, SendError, TryRecvError, TrySendError};
use crate::stream::EventStream;

/// Channel capacities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    /// Commands that may wait for the kernel before senders see backpressure.
    pub command_capacity: NonZeroUsize,
    /// Events retained for slow subscribers before they lag (rounded up to a power of two).
    pub event_capacity: NonZeroUsize,
}

impl Default for BusConfig {
    /// 1,024 commands, 4,096 events.
    fn default() -> Self {
        Self {
            command_capacity: NonZeroUsize::MIN.saturating_add(1023),
            event_capacity: NonZeroUsize::MIN.saturating_add(4095),
        }
    }
}

/// Well below tokio's own channel limits (which panic when exceeded).
const MAX_CAPACITY: usize = usize::MAX >> 4;

/// A snapshot of bus counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BusStats {
    /// Commands accepted.
    pub commands_sent: u64,
    /// Commands rejected because the queue was full.
    pub rejected_full: u64,
    /// Events persisted and broadcast.
    pub events_published: u64,
    /// Events persisted while nobody was subscribed.
    pub published_without_receivers: u64,
    /// Lag reports delivered to subscribers.
    pub lagged: u64,
    /// Current subscribers.
    pub subscribers: usize,
}

#[derive(Debug, Default)]
struct Counters {
    commands_sent: AtomicU64,
    rejected_full: AtomicU64,
    events_published: AtomicU64,
    published_without_receivers: AtomicU64,
    lagged: Arc<AtomicU64>,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// The receiving end of the command queue (owned by the kernel).
#[derive(Debug)]
pub struct CommandReceiver {
    rx: mpsc::Receiver<Command<Unverified>>,
}

impl CommandReceiver {
    /// The next command; `None` once the bus is dropped and the queue is drained.
    pub async fn recv(&mut self) -> Option<Command<Unverified>> {
        self.rx.recv().await
    }

    /// The next command without waiting. Hot path: no allocation.
    ///
    /// # Errors
    /// [`TryRecvError::Empty`] when nothing is queued, [`TryRecvError::Closed`] when the bus is gone.
    pub fn try_recv(&mut self) -> Result<Command<Unverified>, TryRecvError> {
        self.rx.try_recv().map_err(|e| match e {
            mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            mpsc::error::TryRecvError::Disconnected => TryRecvError::Closed,
        })
    }
}

/// CQRS bus: commands in (bounded, explicit backpressure), persisted events out (log-then-publish).
#[derive(Debug)]
pub struct Bus<S> {
    commands: mpsc::Sender<Command<Unverified>>,
    events: broadcast::Sender<EventRecord>,
    store: S,
    /// Serialises append + fan-out so subscribers see events in persisted order.
    publish_lock: Mutex<()>,
    counters: Counters,
}

/// What a fan-out reached.
enum FanoutOutcome {
    Delivered,
    NoReceivers,
}

impl<S: EventStore> Bus<S> {
    /// Creates a bus over `store` and returns the kernel's command receiver.
    ///
    /// # Errors
    /// [`BusError::Config`] when a capacity is unreasonably large.
    pub fn new(cfg: BusConfig, store: S) -> Result<(Self, CommandReceiver), BusError> {
        if cfg.command_capacity.get() > MAX_CAPACITY || cfg.event_capacity.get() > MAX_CAPACITY {
            return Err(BusError::Config("capacity too large"));
        }
        let (commands, rx) = mpsc::channel(cfg.command_capacity.get());
        let (events, _initial) = broadcast::channel(cfg.event_capacity.get());
        let bus = Self {
            commands,
            events,
            store,
            publish_lock: Mutex::new(()),
            counters: Counters::default(),
        };
        Ok((bus, CommandReceiver { rx }))
    }

    /// Queues a command without waiting. Hot path: no allocation after warm-up.
    ///
    /// # Errors
    /// [`TrySendError::Full`] (backpressure) or [`TrySendError::Closed`], each carrying the command.
    pub fn try_send(&self, cmd: Command<Unverified>) -> Result<(), TrySendError> {
        match self.commands.try_send(cmd) {
            Ok(()) => {
                bump(&self.counters.commands_sent);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(cmd)) => {
                bump(&self.counters.rejected_full);
                Err(TrySendError::Full(cmd))
            }
            Err(mpsc::error::TrySendError::Closed(cmd)) => Err(TrySendError::Closed(cmd)),
        }
    }

    /// Queues a command, waiting for capacity (cancel-safe: dropping the future drops the command).
    ///
    /// # Errors
    /// [`SendError`] carrying the command when the receiver is gone.
    pub async fn send(&self, cmd: Command<Unverified>) -> Result<(), SendError> {
        self.commands.send(cmd).await.map_err(|e| SendError(e.0))?;
        bump(&self.counters.commands_sent);
        Ok(())
    }

    /// Persists `record` (the store assigns its sequence number), then broadcasts it. Subscribers
    /// never see an event that is not durable, and see events in persisted order. Blocking: the
    /// append is store I/O, so async callers run this on a dedicated writer task or
    /// `spawn_blocking`.
    ///
    /// # Errors
    /// [`BusError::Store`] when the append fails (nothing is broadcast), [`BusError::Poisoned`].
    pub fn publish(&self, record: EventRecord) -> Result<Seq, BusError> {
        let _ordered = self.publish_lock.lock().map_err(|_| BusError::Poisoned)?;
        let seq = self.store.append(&record)?;
        bump(&self.counters.events_published);
        if let FanoutOutcome::NoReceivers = self.fanout(record.with_seq(seq)) {
            bump(&self.counters.published_without_receivers);
        }
        Ok(seq)
    }

    /// Broadcasts an already-persisted record. Private: only `publish` may fan out, so every
    /// broadcast event is durable. Hot path: no allocation (`EventRecord` is `Copy`).
    fn fanout(&self, record: EventRecord) -> FanoutOutcome {
        match self.events.send(record) {
            Ok(_) => FanoutOutcome::Delivered,
            Err(_) => FanoutOutcome::NoReceivers,
        }
    }

    /// A new subscription starting at the next published event (replay older ones from the store).
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        EventStream::new(self.events.subscribe(), Arc::clone(&self.counters.lagged))
    }

    /// Counter snapshot.
    #[must_use]
    pub fn stats(&self) -> BusStats {
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        BusStats {
            commands_sent: load(&self.counters.commands_sent),
            rejected_full: load(&self.counters.rejected_full),
            events_published: load(&self.counters.events_published),
            published_without_receivers: load(&self.counters.published_without_receivers),
            lagged: load(&self.counters.lagged),
            subscribers: self.events.receiver_count(),
        }
    }

    /// The event store (for replay after a lag report).
    #[must_use]
    pub const fn store(&self) -> &S {
        &self.store
    }
}
