//! Event-log errors and their mapping onto the sdk store errors.

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{FrameId, Seq};
use polaris_sdk::registry::{CacheStoreError, EventStoreError};

/// Event-log failures.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    /// The database file could not be opened or created.
    #[error("cannot open event log: {0}")]
    Open(String),
    /// Another handle (this or another process) owns the database.
    #[error("event log is already open by another handle or process")]
    Locked,
    /// A stored value could not be encoded or decoded.
    #[error("event codec error: {0}")]
    Codec(String),
    /// A record or the file layout comes from a newer Polaris.
    #[error("event schema {found} is newer than the supported schema {supported}")]
    SchemaTooNew {
        /// Version found.
        found: u16,
        /// Newest version this build reads.
        supported: u16,
    },
    /// redb storage, transaction or commit failure.
    #[error("event log storage error: {0}")]
    Db(String),
    /// A checkpoint cannot point past the journal head.
    #[error("checkpoint at {at:?} is beyond the head {head:?}")]
    CheckpointBeyondHead {
        /// Requested position.
        at: Seq,
        /// Current head.
        head: Seq,
    },
    /// The resource is leased by another holder.
    #[error("{resource} is leased by {holder}")]
    LockConflict {
        /// Leased resource.
        resource: String,
        /// Current holder.
        holder: FrameId,
    },
    /// No such lease on the resource.
    #[error("no lease {lease} on {resource}")]
    UnknownLease {
        /// Resource.
        resource: String,
        /// Lease number given.
        lease: u64,
    },
}

impl HasKind for EventLogError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Locked | Self::Db(_) | Self::LockConflict { .. } => ErrorKind::Transient,
            Self::Open(_) | Self::CheckpointBeyondHead { .. } | Self::UnknownLease { .. } => {
                ErrorKind::Permanent
            }
            Self::Codec(_) | Self::SchemaTooNew { .. } => ErrorKind::Fatal,
        }
    }
}

macro_rules! from_redb {
    ($($t:ty),+) => {
        $(impl From<$t> for EventLogError {
            fn from(e: $t) -> Self {
                Self::Db(e.to_string())
            }
        })+
    };
}

from_redb!(
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError,
    redb::SetDurabilityError
);

impl From<EventLogError> for EventStoreError {
    fn from(e: EventLogError) -> Self {
        match e {
            EventLogError::Codec(m) => Self::Codec(m),
            EventLogError::SchemaTooNew { found, supported } => Self::Codec(format!(
                "schema {found} is newer than the supported {supported}"
            )),
            other => Self::Unavailable(other.to_string()),
        }
    }
}

impl From<EventLogError> for CacheStoreError {
    fn from(e: EventLogError) -> Self {
        match e {
            EventLogError::Codec(m) => Self::Corrupt(m),
            other => Self::Io(other.to_string()),
        }
    }
}
