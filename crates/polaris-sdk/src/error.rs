//! Error taxonomy shared by every crate (`ErrorKind` + `HasKind`) and the sdk's own parse errors.

use serde::{Deserialize, Serialize};

/// How a failure is handled by supervisors, retry policies and circuit breakers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Retrying (with backoff) may succeed: timeouts, 429 / 5xx, locked or busy resources.
    Transient,
    /// Retrying the same input fails again: bad request, invalid data, missing resource.
    Permanent,
    /// Refused by policy (sandbox, capability token, path guard); never retried automatically.
    Policy,
    /// Integrity is in doubt (corrupt log, broken invariant); stop the affected component.
    Fatal,
}

/// Implemented by every error enum in the workspace so failures can be classified uniformly.
pub trait HasKind {
    /// The handling class of this error.
    fn kind(&self) -> ErrorKind;
}

/// Longest input fragment kept inside a [`ParseError`] (inputs may be user-provided).
const MAX_ECHO: usize = 32;

/// Copies at most [`MAX_ECHO`] characters of `input` for an error message.
pub(crate) fn echo(input: &str) -> Box<str> {
    input
        .chars()
        .take(MAX_ECHO)
        .collect::<String>()
        .into_boxed_str()
}

/// Parse and validation failures of sdk value types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Hex text of the wrong length or with a non-hex character.
    #[error("invalid hex: expected {expected} hex characters, got {found} bytes")]
    InvalidHex {
        /// Number of hex characters required.
        expected: usize,
        /// Length of the rejected input in bytes.
        found: usize,
    },
    /// Not one of `low|medium|high|xhigh|max` or `1`–`5`.
    #[error("invalid effort {0:?}: expected low|medium|high|xhigh|max or 1-5")]
    InvalidEffort(Box<str>),
    /// A numeric effort outside `1`–`5`.
    #[error("invalid effort level {0}: expected 1-5")]
    InvalidEffortLevel(u8),
    /// NaN, infinite or outside `0.0..=1.0`.
    #[error("probability must be finite and within 0..=1")]
    InvalidProbability,
    /// A validated string newtype rejected its input.
    #[error("invalid {what}: {reason}")]
    InvalidValue {
        /// Name of the value type.
        what: &'static str,
        /// Which rule was violated.
        reason: &'static str,
    },
    /// Text that is not a UUID.
    #[error("invalid uuid {0:?}")]
    InvalidUuid(Box<str>),
    /// An event record from an unsupported schema version.
    #[error("unsupported event schema version {0}")]
    UnsupportedSchema(u16),
}

impl HasKind for ParseError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}
