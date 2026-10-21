//! Bus errors.

use polaris_sdk::contracts::{Command, Unverified};
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::Seq;
use polaris_sdk::registry::EventStoreError;

/// Bus failures (publishing and subscribing).
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    /// The bus (or the other side of the channel) is gone.
    #[error("bus closed")]
    Closed,
    /// A subscriber fell behind; replay `resume_from ..= resume_from + missed - 1` from the event
    /// store, then keep reading.
    #[error("subscriber lagged: {missed} events missed, replay from {resume_from:?}")]
    Lagged {
        /// Events the subscriber did not receive.
        missed: u64,
        /// First missed sequence number.
        resume_from: Seq,
    },
    /// The event store refused the append; nothing was broadcast.
    #[error(transparent)]
    Store(#[from] EventStoreError),
    /// Invalid configuration.
    #[error("invalid bus configuration: {0}")]
    Config(&'static str),
    /// A publisher panicked while holding the ordering lock.
    #[error("bus publish lock poisoned")]
    Poisoned,
}

impl HasKind for BusError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Closed | Self::Config(_) => ErrorKind::Permanent,
            Self::Lagged { .. } => ErrorKind::Transient,
            Self::Store(e) => e.kind(),
            Self::Poisoned => ErrorKind::Fatal,
        }
    }
}

/// A command the queue did not accept; the command is handed back, never dropped silently.
#[derive(Debug, thiserror::Error)]
pub enum TrySendError {
    /// The queue is at capacity (backpressure).
    #[error("command queue full")]
    Full(Command<Unverified>),
    /// The receiver is gone.
    #[error("bus closed")]
    Closed(Command<Unverified>),
}

impl TrySendError {
    /// The rejected command.
    #[must_use]
    pub fn into_command(self) -> Command<Unverified> {
        match self {
            Self::Full(cmd) | Self::Closed(cmd) => cmd,
        }
    }
}

impl HasKind for TrySendError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Full(_) => ErrorKind::Transient,
            Self::Closed(_) => ErrorKind::Permanent,
        }
    }
}

/// `send` failed because the receiver is gone; the command is handed back.
#[derive(Debug, thiserror::Error)]
#[error("bus closed")]
pub struct SendError(pub Command<Unverified>);

/// Why `try_recv` returned no command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TryRecvError {
    /// Nothing queued right now.
    #[error("no command queued")]
    Empty,
    /// Every sender is gone.
    #[error("bus closed")]
    Closed,
}
