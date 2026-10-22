//! Telemetry failures.

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{Seq, SessionId};

/// Failures of the telemetry aggregates. Counter overflow is deliberately not an error: counters
/// saturate and the snapshot carries a `saturated` flag (see the crate documentation).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TelemetryError {
    /// The latency histogram could not be created or two histograms could not be merged.
    #[error("latency histogram error: {0}")]
    Histogram(Box<str>),
    /// An event record of another session was offered to this session's telemetry.
    #[error("event record of session {found} offered to telemetry of session {expected}")]
    SessionMismatch {
        /// The session this telemetry belongs to.
        expected: SessionId,
        /// The session named by the rejected record.
        found: SessionId,
    },
    /// Two different records claimed the same sequence number as the latest folded record.
    #[error("conflicting event records at sequence {0:?}")]
    ConflictingSequence(Seq),
    /// An event older than the latest folded record was submitted.
    #[error("event sequence {found:?} follows newer sequence {previous:?}")]
    OutOfOrder {
        /// The latest successfully folded sequence.
        previous: Seq,
        /// The older sequence that was rejected.
        found: Seq,
    },
    /// A warn threshold outside `1..=100` percent.
    #[error("warn percent {0} is outside 1..=100")]
    InvalidWarnPercent(u8),
}

impl HasKind for TelemetryError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Histogram(_)
            | Self::SessionMismatch { .. }
            | Self::ConflictingSequence(_)
            | Self::OutOfOrder { .. }
            | Self::InvalidWarnPercent(_) => ErrorKind::Permanent,
        }
    }
}
