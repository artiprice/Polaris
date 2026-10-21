//! Logging errors.

use polaris_sdk::error::{ErrorKind, HasKind};

/// Failures of [`init`](crate::init) / [`start`](crate::start).
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// A global tracing subscriber is already installed in this process (by [`init`](crate::init)
    /// or by someone else). It can never be replaced, not even after the first guard is dropped.
    #[error("logging is already initialised in this process")]
    AlreadyInitialised,
    /// An operating-system error (for example spawning the drain thread).
    #[error("log I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The `RUST_LOG`-style filter could not be parsed.
    #[error("invalid log filter: {0}")]
    BadFilter(String),
    /// A [`LogConfig`](crate::LogConfig) field is out of range.
    #[error("invalid log configuration: {0}")]
    BadConfig(String),
    /// A sink could not be created (for example the log directory is not writable).
    #[error("log sink initialisation failed: {0}")]
    SinkInit(String),
}

impl HasKind for LogError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::AlreadyInitialised | Self::BadFilter(_) | Self::BadConfig(_) => {
                ErrorKind::Permanent
            }
            Self::Io(_) | Self::SinkInit(_) => ErrorKind::Transient,
        }
    }
}
