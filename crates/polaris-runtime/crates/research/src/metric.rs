//! Fixed-harness metric parsing, bounded independently of the experiment's configured limits.
use crate::MetricValue;

const MAX_OUTPUT: usize = 65_536;
const MAX_NAME: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Format {
    Scalar,
    Named(Box<str>),
}

/// Validated output parser for an immutable metric harness.
///
/// Parsing output never proves that its command succeeded. The runner must check exit status,
/// cancellation, timeout and source integrity before submitting the result to the decision state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricParser {
    format: Format,
    max_bytes: usize,
}
impl MetricParser {
    /// Require the complete trimmed output to contain exactly one finite number.
    ///
    /// # Errors
    /// Rejects zero or over-64-KiB output limits.
    pub fn scalar(max_bytes: usize) -> Result<Self, MetricParseError> {
        Self::new(Format::Scalar, max_bytes)
    }
    /// Require one exact `name=value` line, permitting unrelated harness progress lines.
    ///
    /// Names are case-sensitive ASCII identifiers. Matching lines cannot be duplicated, even
    /// when the two values agree. An invalid matching line cannot be hidden by a later valid line.
    ///
    /// # Errors
    /// Rejects invalid/oversized names or output limits.
    pub fn named(name: &str, max_bytes: usize) -> Result<Self, MetricParseError> {
        if name.is_empty()
            || name.len() > MAX_NAME
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_digit())
        {
            return Err(MetricParseError::Configuration);
        }
        Self::new(Format::Named(name.into()), max_bytes)
    }
    fn new(format: Format, max_bytes: usize) -> Result<Self, MetricParseError> {
        if max_bytes == 0 || max_bytes > MAX_OUTPUT {
            return Err(MetricParseError::Configuration);
        }
        Ok(Self { format, max_bytes })
    }
    /// Decode an unambiguous finite result without allocating intermediate output strings.
    ///
    /// # Errors
    /// Rejects oversized/invalid-UTF-8 output, forbidden control bytes, missing/duplicate metrics
    /// and nonnumeric/non-finite values. CRLF and surrounding ASCII whitespace are accepted.
    pub fn parse(&self, output: &[u8]) -> Result<MetricValue, MetricParseError> {
        if output.len() > self.max_bytes {
            return Err(MetricParseError::Limit);
        }
        let text = std::str::from_utf8(output).map_err(|_| MetricParseError::Encoding)?;
        if text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
        {
            return Err(MetricParseError::Encoding);
        }
        match &self.format {
            Format::Scalar => number(text),
            Format::Named(name) => {
                let mut result = None;
                for line in text.lines() {
                    let line = line.trim_ascii();
                    let Some(value) = line
                        .strip_prefix(name.as_ref())
                        .and_then(|tail| tail.strip_prefix('='))
                    else {
                        continue;
                    };
                    if result.is_some() {
                        return Err(MetricParseError::Duplicate);
                    }
                    result = Some(number(value)?);
                }
                result.ok_or(MetricParseError::Missing)
            }
        }
    }
}

fn number(value: &str) -> Result<MetricValue, MetricParseError> {
    let value = value.trim_ascii();
    if value.is_empty() || !value.is_ascii() {
        return Err(MetricParseError::Number);
    }
    let value = value.parse::<f64>().map_err(|_| MetricParseError::Number)?;
    MetricValue::new(value).map_err(|_| MetricParseError::Number)
}

/// Metric rejection never includes potentially sensitive harness output in its error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MetricParseError {
    /// Invalid parser limits or metric identifier.
    #[error("invalid metric parser configuration")]
    Configuration,
    /// Harness output exceeds its configured bound.
    #[error("metric output exceeds byte limit")]
    Limit,
    /// Output is not valid textual harness output.
    #[error("metric output has invalid encoding")]
    Encoding,
    /// No exact named metric line exists.
    #[error("metric output is missing the configured result")]
    Missing,
    /// More than one configured metric line exists.
    #[error("metric output contains duplicate results")]
    Duplicate,
    /// Value is empty, malformed, ambiguous, or non-finite.
    #[error("metric must be a single finite number")]
    Number,
}
