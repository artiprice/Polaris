//! Composition failures distinguish rejected inputs from events already committed durably.
use crate::{
    bus::BusError,
    directives::DirectiveError,
    reaper::ReaperError,
    storage::{cas::CasError, eventlog::EventLogError, registry::RegistryError},
};
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{Seq, SessionId};

/// Kernel/session failures. Never log configuration documents or tokens through these errors.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// Context is shutting down or already closed.
    #[error("kernel closed")]
    Closed,
    /// Another mutable session owner exists in this context.
    #[error("session {0} is already open")]
    SessionAlreadyOpen(SessionId),
    /// Session id is already present in the immutable log.
    #[error("session {0} already exists; reopen it")]
    SessionExists(SessionId),
    /// No configuration for the requested id exists.
    #[error("session {0} does not exist")]
    UnknownSession(SessionId),
    /// Configuration committed, but startup never committed.
    #[error("session {0} was configured but never started")]
    NotStarted(SessionId),
    /// Crash recovery cannot safely release an old resource automatically.
    #[error("startup requires operator reconciliation of {count} resource(s)")]
    RecoveryRequired {
        /// Unresolved locators in the run journal.
        count: usize,
    },
    /// Invalid configuration, refused before event append.
    #[error("invalid session configuration: {0}")]
    Configuration(&'static str),
    /// Actual typed sandbox backend failure.
    #[error(transparent)]
    Sandbox(#[from] polaris_sdk::registry::SandboxError),
    /// Invalid event sequence, payload or internal ownership invariant.
    #[error("session replay integrity error: {0}")]
    Integrity(&'static str),
    /// The log commit succeeded; rebuilding the registry projection is required.
    #[error("event {seq:?} committed; registry projection failed: {source}")]
    ProjectionBehind {
        /// Durable sequence to replay.
        seq: Seq,
        /// Projection failure, never an event-log rollback.
        source: RegistryError,
    },
    /// Local filesystem failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Content store failure.
    #[error(transparent)]
    Cas(#[from] CasError),
    /// Event log failure.
    #[error(transparent)]
    EventLog(#[from] EventLogError),
    /// Registry failure before an event commit.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// Bus append/publication failed.
    #[error(transparent)]
    Bus(#[from] BusError),
    /// Invalid directive grammar.
    #[error(transparent)]
    Directive(#[from] DirectiveError),
    /// Resource recovery failure.
    #[error(transparent)]
    Reaper(#[from] ReaperError),
    /// Configuration encoding failed.
    #[error(transparent)]
    Codec(#[from] serde_json::Error),
    /// An owned blocking operation panicked or was cancelled by the runtime.
    #[error("kernel owner task failed: {0}")]
    Runtime(String),
    /// Shared result of an already-completed shutdown attempt.
    #[error("kernel shutdown failed: {0}")]
    Shutdown(std::sync::Arc<KernelError>),
    /// Internal state mutex was poisoned; do not continue mutation.
    #[error("kernel state poisoned")]
    Poisoned,
}
impl HasKind for KernelError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Closed
            | Self::SessionExists(_)
            | Self::UnknownSession(_)
            | Self::NotStarted(_)
            | Self::Configuration(_) => ErrorKind::Permanent,
            Self::SessionAlreadyOpen(_) | Self::Io(_) => ErrorKind::Transient,
            Self::RecoveryRequired { .. } => ErrorKind::Policy,
            Self::Cas(error) => error.kind(),
            Self::Bus(error) => error.kind(),
            Self::Directive(error) => error.kind(),
            Self::Registry(error) => error.kind(),
            Self::EventLog(error) => error.kind(),
            Self::Sandbox(error) => error.kind(),
            Self::Shutdown(error) => error.kind(),
            Self::Integrity(_)
            | Self::ProjectionBehind { .. }
            | Self::Reaper(_)
            | Self::Codec(_)
            | Self::Runtime(_)
            | Self::Poisoned => ErrorKind::Fatal,
        }
    }
}
