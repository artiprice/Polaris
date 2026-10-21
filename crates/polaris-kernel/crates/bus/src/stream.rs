//! Event subscriptions with resumable lag reports.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use polaris_sdk::contracts::EventRecord;
use polaris_sdk::ids::Seq;
use tokio::sync::broadcast;

use crate::error::BusError;

/// A subscription to persisted events.
///
/// When the subscriber falls behind the retained window, it gets one [`BusError::Lagged`] with the
/// first missed sequence number; it replays the gap from the event store and keeps reading — the
/// next call returns the first retained event, so every event is seen exactly once.
#[derive(Debug)]
pub struct EventStream {
    rx: broadcast::Receiver<EventRecord>,
    /// First retained event after a lag report, returned by the next call.
    pending: Option<EventRecord>,
    last_seen: Option<Seq>,
    lagged: Arc<AtomicU64>,
}

impl EventStream {
    pub(crate) const fn new(rx: broadcast::Receiver<EventRecord>, lagged: Arc<AtomicU64>) -> Self {
        Self {
            rx,
            pending: None,
            last_seen: None,
            lagged,
        }
    }

    fn seen(&mut self, record: EventRecord) -> EventRecord {
        self.last_seen = Some(record.seq());
        record
    }

    /// Builds the lag report; `next` is the first retained event, if one was available.
    fn lag(&mut self, missed: u64, next: Option<EventRecord>) -> BusError {
        self.lagged.fetch_add(1, Ordering::Relaxed);
        let resume_from = match (next, self.last_seen) {
            (Some(first), _) => Seq::new(first.seq().get().saturating_sub(missed)),
            (None, Some(last)) => last.next().unwrap_or(last),
            (None, None) => Seq::new(1),
        };
        self.pending = next;
        tracing::warn!(
            missed,
            resume_from = resume_from.get(),
            "event subscriber lagged"
        );
        BusError::Lagged {
            missed,
            resume_from,
        }
    }

    /// The next event, waiting if necessary.
    ///
    /// # Errors
    /// [`BusError::Lagged`] once per fall-behind (replay, then call again); [`BusError::Closed`] when
    /// the bus is gone and the retained events are drained.
    pub async fn next(&mut self) -> Result<EventRecord, BusError> {
        if let Some(record) = self.pending.take() {
            return Ok(self.seen(record));
        }
        match self.rx.recv().await {
            Ok(record) => Ok(self.seen(record)),
            Err(broadcast::error::RecvError::Closed) => Err(BusError::Closed),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                let next = self.rx.recv().await.ok();
                Err(self.lag(missed, next))
            }
        }
    }

    /// The next event if one is ready. Hot path: no allocation.
    ///
    /// # Errors
    /// As [`EventStream::next`].
    pub fn try_next(&mut self) -> Result<Option<EventRecord>, BusError> {
        if let Some(record) = self.pending.take() {
            return Ok(Some(self.seen(record)));
        }
        match self.rx.try_recv() {
            Ok(record) => Ok(Some(self.seen(record))),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => Err(BusError::Closed),
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                let next = self.rx.try_recv().ok();
                Err(self.lag(missed, next))
            }
        }
    }
}
