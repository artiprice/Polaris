//! Bounded PII sanitization operator: raw text is consumed and only `MaskedText` is returned.
use polaris_sdk::{
    error::{ErrorKind, HasKind},
    registry::{FunctionalOperator, RedactStats, Redactor},
};

/// Hard maximum input accepted by this operator.
pub const MAX_SANITIZE_BYTES: usize = 1_048_576;

/// Model-bound text produced by a redactor. Construction is private to this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskedText {
    value: String,
    stats: RedactStats,
}
impl MaskedText {
    /// Borrow the sanitized UTF-8 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
    /// Redaction counters reported by the injected redactor.
    #[must_use]
    pub const fn stats(&self) -> RedactStats {
        self.stats
    }
}

/// Sanitization failure blocks model-bound processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SanitizeError {
    /// Input exceeds the configured maximum before redaction begins.
    #[error("PII input exceeds the configured byte limit")]
    TooLarge,
    /// Redactor reports a byte count inconsistent with the supplied input.
    #[error("redactor byte count did not match input length")]
    InvalidStats,
    /// Redactor output could not be represented as UTF-8 model input.
    #[error("redactor produced invalid UTF-8")]
    InvalidUtf8,
}
impl HasKind for SanitizeError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::TooLarge => ErrorKind::Permanent,
            Self::InvalidStats | Self::InvalidUtf8 => ErrorKind::Fatal,
        }
    }
}

/// Invalid bound supplied when constructing the sanitizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("PII byte limit must be within 1..=MAX_SANITIZE_BYTES")]
pub struct SanitizeConfigError;

/// Production sanitizer adapter over an injected SDK redactor.
#[derive(Debug)]
pub struct SanitizePiiOp<R> {
    redactor: R,
    max_input_bytes: usize,
}
impl<R> SanitizePiiOp<R> {
    /// Use an actual configured redactor and explicit positive input bound; there is no no-op fallback.
    pub fn new(redactor: R, max_input_bytes: usize) -> Result<Self, SanitizeConfigError> {
        if max_input_bytes == 0 || max_input_bytes > MAX_SANITIZE_BYTES {
            return Err(SanitizeConfigError);
        }
        Ok(Self {
            redactor,
            max_input_bytes,
        })
    }
}
impl<R: Redactor> FunctionalOperator for SanitizePiiOp<R> {
    type In = String;
    type Out = MaskedText;
    type Err = SanitizeError;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        if input.len() > self.max_input_bytes {
            return Err(SanitizeError::TooLarge);
        }
        let mut output = Vec::with_capacity(input.len());
        let stats = self.redactor.redact_into(input.as_bytes(), &mut output);
        if stats.bytes_in != input.len() {
            return Err(SanitizeError::InvalidStats);
        }
        let value = String::from_utf8(output).map_err(|_| SanitizeError::InvalidUtf8)?;
        Ok(MaskedText { value, stats })
    }
}
