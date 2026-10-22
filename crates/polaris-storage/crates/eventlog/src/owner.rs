//! Bounded async admission to a dedicated blocking database owner, with opportunistic group commit.
use crate::{EventLog, EventLogError};
use polaris_sdk::{
    contracts::EventRecord,
    error::{ErrorKind, HasKind},
    ids::Seq,
};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch};

/// Explicit asynchronous journal failures.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OwnerError {
    /// Invalid channel capacity (must be 1..=65536).
    #[error("invalid event log queue capacity")]
    Capacity,
    /// Admission is closed or the owner stopped.
    #[error("event log owner closed")]
    Closed,
    /// Actual database failure, shared between every member of a failed atomic batch.
    #[error(transparent)]
    Storage(Arc<EventLogError>),
    /// The dedicated thread could not start.
    #[error(transparent)]
    Spawn(Arc<std::io::Error>),
    /// Owner thread or joining task panicked.
    #[error("event log owner task failed")]
    Runtime,
}
impl HasKind for OwnerError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Capacity | Self::Closed => ErrorKind::Permanent,
            Self::Storage(error) => error.kind(),
            Self::Spawn(_) => ErrorKind::Transient,
            Self::Runtime => ErrorKind::Fatal,
        }
    }
}
enum Request {
    Append(EventRecord, oneshot::Sender<Result<Seq, OwnerError>>),
    Stop(oneshot::Sender<()>),
}
struct Admission {
    tx: mpsc::Sender<Request>,
    closed: bool,
}
type ShutdownState = Option<Result<(), OwnerError>>;
struct Shared {
    admission: tokio::sync::Mutex<Admission>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    shutdown: tokio::sync::Mutex<Option<watch::Receiver<ShutdownState>>>,
}
/// Cloneable async API; accepted writes survive cancellation of their receipt future.
/// Drop closes admission when the final clone disappears and the owner drains queued requests.
#[derive(Clone)]
pub struct AsyncEventLog {
    shared: Arc<Shared>,
}
impl std::fmt::Debug for AsyncEventLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncEventLog").finish_non_exhaustive()
    }
}
impl AsyncEventLog {
    /// Start one blocking owner. Queue memory is bounded by `capacity`; batches contain at most 128 events.
    /// # Errors
    /// Invalid capacity or native thread spawn failure.
    pub fn new(log: Arc<EventLog>, capacity: usize) -> Result<Self, OwnerError> {
        if capacity == 0 || capacity > 65_536 {
            return Err(OwnerError::Capacity);
        }
        let (tx, rx) = mpsc::channel(capacity);
        let thread = std::thread::Builder::new()
            .name("polaris-eventlog".to_owned())
            .spawn(move || own(log, rx))
            .map_err(|error| OwnerError::Spawn(Arc::new(error)))?;
        Ok(Self {
            shared: Arc::new(Shared {
                admission: tokio::sync::Mutex::new(Admission { tx, closed: false }),
                thread: Mutex::new(Some(thread)),
                shutdown: tokio::sync::Mutex::new(None),
            }),
        })
    }
    /// Append without blocking a runtime worker; success means the configured durability committed.
    /// # Errors
    /// Closed owner, join failure or actual storage error.
    pub async fn append(&self, record: EventRecord) -> Result<Seq, OwnerError> {
        let (tx, rx) = oneshot::channel();
        {
            let admission = self.shared.admission.lock().await;
            if admission.closed {
                return Err(OwnerError::Closed);
            }
            admission
                .tx
                .send(Request::Append(record, tx))
                .await
                .map_err(|_| OwnerError::Closed)?;
        }
        rx.await.map_err(|_| OwnerError::Closed)?
    }
    /// Close admission atomically, drain accepted records, then join the owner off runtime workers.
    /// # Errors
    /// Closed owner or thread failure. Repeated calls share the same result; caller cancellation
    /// does not interrupt draining or joining the native owner.
    pub async fn shutdown(&self) -> Result<(), OwnerError> {
        let mut receiver = {
            let mut shutdown = self.shared.shutdown.lock().await;
            match &*shutdown {
                Some(receiver) => receiver.clone(),
                None => {
                    let (tx, receiver) = watch::channel(None);
                    *shutdown = Some(receiver.clone());
                    let shared = Arc::clone(&self.shared);
                    let _owner = tokio::spawn(async move {
                        let result = stop_owner(shared).await;
                        let _notified = tx.send(Some(result));
                    });
                    receiver
                }
            }
        };
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver.changed().await.map_err(|_| OwnerError::Closed)?;
        }
    }
}
async fn stop_owner(shared: Arc<Shared>) -> Result<(), OwnerError> {
    let (tx, rx) = oneshot::channel();
    {
        let mut admission = shared.admission.lock().await;
        admission.closed = true;
        admission
            .tx
            .send(Request::Stop(tx))
            .await
            .map_err(|_| OwnerError::Closed)?;
    }
    rx.await.map_err(|_| OwnerError::Closed)?;
    let thread = shared
        .thread
        .lock()
        .map_err(|_| OwnerError::Runtime)?
        .take()
        .ok_or(OwnerError::Closed)?;
    tokio::task::spawn_blocking(move || thread.join().map_err(|_| OwnerError::Runtime))
        .await
        .map_err(|_| OwnerError::Runtime)?
}
fn own(log: Arc<EventLog>, mut rx: mpsc::Receiver<Request>) {
    let mut records = Vec::with_capacity(128);
    let mut replies = Vec::with_capacity(128);
    while let Some(request) = rx.blocking_recv() {
        let Request::Append(record, reply) = request else {
            if let Request::Stop(reply) = request {
                let _delivered = reply.send(());
            }
            break;
        };
        records.push(record);
        replies.push(reply);
        let mut stop = None;
        while records.len() < 128 {
            match rx.try_recv() {
                Ok(Request::Append(record, reply)) => {
                    records.push(record);
                    replies.push(reply);
                }
                Ok(Request::Stop(reply)) => {
                    stop = Some(reply);
                    break;
                }
                Err(_) => break,
            }
        }
        let result = log
            .append_batch(&records)
            .map_err(|error| OwnerError::Storage(Arc::new(error)));
        let count = match u64::try_from(records.len()) {
            Ok(count) => count,
            Err(_) => {
                for reply in replies.drain(..) {
                    let _delivered = reply.send(Err(OwnerError::Runtime));
                }
                break;
            }
        };
        match result {
            Ok(last) => {
                let start = last
                    .get()
                    .checked_sub(count)
                    .and_then(|value| value.checked_add(1));
                for (offset, reply) in (0_u64..).zip(replies.drain(..)) {
                    let result = start
                        .and_then(|value| value.checked_add(offset))
                        .map(Seq::new)
                        .ok_or(OwnerError::Runtime);
                    let _delivered = reply.send(result);
                }
            }
            Err(error) => {
                for reply in replies.drain(..) {
                    let _delivered = reply.send(Err(error.clone()));
                }
            }
        }
        records.clear();
        if let Some(reply) = stop {
            let _delivered = reply.send(());
            break;
        }
    }
}
