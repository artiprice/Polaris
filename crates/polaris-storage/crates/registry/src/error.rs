//! Registry errors and their classification.

use polaris_sdk::error::{ErrorKind, HasKind};
use rusqlite::ErrorCode;

/// Registry failures. Messages never echo user text beyond what SQLite itself reports.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The directory or database file could not be created or opened.
    #[error("cannot open session registry: {0}")]
    Open(String),
    /// Another connection (a CLI, the daemon, antivirus, ...) holds the write lock and the bounded
    /// retries ran out. Retrying later may succeed.
    #[error("session registry is busy (database locked by another connection)")]
    Busy,
    /// A schema migration step failed; the database was rolled back to `from`.
    #[error("migration from schema {from} to {to} failed: {reason}")]
    Migration {
        /// Version the database was at.
        from: u32,
        /// Version the failed step was moving to.
        to: u32,
        /// SQLite's explanation.
        reason: String,
    },
    /// The database was written by a newer Polaris.
    #[error("registry schema {found} is newer than the supported schema {supported}")]
    SchemaTooNew {
        /// Version found in the file.
        found: u32,
        /// Newest version this build reads.
        supported: u32,
    },
    /// The file is not a valid SQLite database or its pages are damaged.
    #[error("session registry is corrupt: {0}")]
    Corrupt(String),
    /// A value cannot be stored (for example a counter above `i64::MAX`, or a non-UTF-8 path).
    #[error("value cannot be stored in the registry: {0}")]
    BadValue(String),
    /// Any other SQLite failure.
    #[error("session registry SQL error: {0}")]
    Sql(String),
    /// The database thread is gone (the handle was closed or the thread stopped).
    #[error("session registry is closed")]
    Closed,
}

impl HasKind for RegistryError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Busy => ErrorKind::Transient,
            Self::Open(_) | Self::BadValue(_) | Self::Sql(_) | Self::Closed => ErrorKind::Permanent,
            Self::Migration { .. } | Self::SchemaTooNew { .. } | Self::Corrupt(_) => {
                ErrorKind::Fatal
            }
        }
    }
}

impl From<rusqlite::Error> for RegistryError {
    fn from(e: rusqlite::Error) -> Self {
        match e.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
                Self::Corrupt(e.to_string())
            }
            _ => Self::Sql(e.to_string()),
        }
    }
}
