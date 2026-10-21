//! Stream format detection.

use crate::input::decode_stream;

/// What a piped stream contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamFormat {
    /// Only one-line JSON objects (`cargo --message-format=json`).
    CargoJson,
    /// Human-readable compiler text.
    Text,
    /// JSON lines interleaved with text lines (for example `2>&1` of a JSON build, or a cargo
    /// `Compiling ...` line before the first message).
    Mixed,
}

/// `true` for a trimmed line that looks like a one-line JSON object.
pub(crate) fn is_json_line(trimmed: &str) -> bool {
    trimmed.starts_with('{') && trimmed.ends_with('}')
}

/// Classifies already decoded text.
pub(crate) fn detect_decoded(text: &str) -> StreamFormat {
    let mut json = 0usize;
    let mut other = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_json_line(trimmed) {
            json += 1;
        } else {
            other += 1;
        }
        if json > 0 && other > 0 {
            return StreamFormat::Mixed;
        }
    }
    match (json, other) {
        (0, _) => StreamFormat::Text,
        _ => StreamFormat::CargoJson,
    }
}

/// Detects whether `stream` is cargo JSON, plain text, or a mixture of both.
///
/// The reference engine decides on the first non-whitespace character only; this port looks at
/// every non-blank line, so a stderr `Compiling ...` line before the first JSON line (or a text
/// summary after the last) yields [`StreamFormat::Mixed`] instead of a misclassification. Empty
/// input is [`StreamFormat::Text`].
#[must_use]
pub fn detect_format(stream: &[u8]) -> StreamFormat {
    detect_decoded(&decode_stream(stream))
}
