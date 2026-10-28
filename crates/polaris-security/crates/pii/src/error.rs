//! Typed failures for PII masking and sandbox-bound unmasking.

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_security_automata::AutomataError;

/// A PII operation failed without echoing scanned or vaulted text.
#[derive(Debug, thiserror::Error)]
pub enum PiiError {
    /// Snapshot authentication, format or session/frame binding failed.
    #[error("PII snapshot authentication or format is invalid")]
    Snapshot,
    /// The deterministic scanner failed or exceeded its bounded match buffer.
    #[error(transparent)]
    Scan(#[from] AutomataError),
    /// A masking or unmasking input exceeded its configured byte bound.
    #[error("PII input exceeds the configured byte limit")]
    InputLimit,
    /// The infallible SDK byte-redactor received non-UTF-8 input.
    #[error("PII redaction requires valid UTF-8 input")]
    InvalidUtf8,
    /// The match set or reversible vault reached its configured capacity.
    #[error("PII vault capacity exceeded")]
    Capacity,
    /// Semantic PII model returned an invalid, oversized, or malformed byte range.
    #[error("semantic PII span response is invalid")]
    InvalidNerSpans,
    /// A checked counter or length calculation overflowed.
    #[error("PII vault counter overflow")]
    CounterOverflow,
    /// No trusted sandbox authorization was granted for this session and frame.
    #[error("sandbox-bound unmasking was denied")]
    BoundaryDenied,
    /// The vault's internal synchronization state was poisoned.
    #[error("PII vault state is unavailable")]
    StateUnavailable,
    /// Secure random generation for the session marker tag failed.
    #[error("secure random generation failed")]
    RandomUnavailable,
}

impl HasKind for PiiError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Scan(error) => error.kind(),
            Self::InputLimit
            | Self::InvalidUtf8
            | Self::Capacity
            | Self::InvalidNerSpans
            | Self::CounterOverflow
            | Self::RandomUnavailable => ErrorKind::Permanent,
            Self::Snapshot => ErrorKind::Policy,
            Self::BoundaryDenied => ErrorKind::Policy,
            Self::StateUnavailable => ErrorKind::Fatal,
        }
    }
}
