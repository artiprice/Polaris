//! Cargo JSON path (`--message-format=json`): diagnostics from `compiler-message` lines and the
//! structured report.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::detect::StreamFormat;
use crate::diag::{Diagnostic, Level, Location, ThirdPartyMatcher, is_summary_message};
use crate::error::RtkError;
use crate::input::{check_size, decode_stream};
use crate::text::collect_diagnostics;

/// Longest `expansion.span` chain followed to find the outer call site of a macro-expanded error.
pub const MAX_EXPANSION_DEPTH: usize = 32;

#[derive(Deserialize)]
struct CargoLine {
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    message: Option<RustcMessage>,
}

#[derive(Deserialize)]
struct RustcMessage {
    #[serde(default)]
    message: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    code: Option<RustcCode>,
    #[serde(default)]
    spans: Vec<RustcSpan>,
    #[serde(default)]
    children: Vec<RustcChild>,
}

#[derive(Deserialize)]
struct RustcCode {
    #[serde(default)]
    code: String,
}

#[derive(Deserialize, Default)]
struct RustcSpan {
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    line_start: u32,
    #[serde(default)]
    column_start: u32,
    #[serde(default)]
    is_primary: bool,
    #[serde(default)]
    expansion: Option<Box<RustcExpansion>>,
}

#[derive(Deserialize, Default)]
struct RustcExpansion {
    #[serde(default)]
    span: RustcSpan,
}

#[derive(Deserialize)]
struct RustcChild {
    #[serde(default)]
    message: String,
    #[serde(default)]
    level: String,
}

/// Follows `expansion.span` from the primary span to the outermost call site, at most
/// [`MAX_EXPANSION_DEPTH`] hops (the walk is iterative, so hostile input cannot recurse).
fn call_site(primary: &RustcSpan) -> &RustcSpan {
    let mut span = primary;
    for _ in 0..MAX_EXPANSION_DEPTH {
        match span.expansion.as_deref() {
            Some(expansion) => span = &expansion.span,
            None => break,
        }
    }
    span
}

/// Parses one JSON line into a diagnostic; `None` for anything that is not an error or warning
/// `compiler-message` (artifacts, notes, summaries, malformed or too deeply nested lines).
fn parse_line(line: &str, matcher: &ThirdPartyMatcher) -> Option<Diagnostic> {
    if !line.contains("\"compiler-message\"") {
        return None;
    }
    // serde_json's default recursion limit (128) rejects hostile nesting instead of overflowing.
    let parsed: CargoLine = serde_json::from_str(line.trim()).ok()?;
    if parsed.reason.as_deref() != Some("compiler-message") {
        return None;
    }
    let message = parsed.message?;
    let level = match message.level.as_str() {
        "error" | "error: internal compiler error" => Level::Error,
        "warning" => Level::Warning,
        _ => return None,
    };
    if is_summary_message(&message.message) {
        return None;
    }
    let code = message.code.map(|c| c.code).filter(|c| !c.is_empty());
    let mut diagnostic = Diagnostic::new(level, code.as_deref(), &message.message);
    let primary = message
        .spans
        .iter()
        .find(|s| s.is_primary)
        .or_else(|| message.spans.first());
    if let Some(span) = primary.map(call_site).filter(|s| !s.file_name.is_empty()) {
        diagnostic.first_party = !matcher.is_third_party(&span.file_name);
        diagnostic.primary = Some(Location {
            file: span.file_name.clone(),
            line: span.line_start,
            column: span.column_start,
        });
    }
    diagnostic.details = message
        .children
        .iter()
        .filter(|c| !c.message.is_empty())
        .map(|c| format!("{}: {}", c.level, c.message))
        .collect();
    Some(diagnostic)
}

/// Parses JSON lines into diagnostics, skipping what is not a diagnostic.
pub(crate) fn parse_json_lines(lines: &[&str], matcher: &ThirdPartyMatcher) -> Vec<Diagnostic> {
    lines
        .iter()
        .filter_map(|line| parse_line(line, matcher))
        .collect()
}

/// Removes diagnostics that repeat an earlier headline plus primary location, keeping order.
pub(crate) fn dedupe(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut seen: HashSet<(String, Option<Location>)> = HashSet::new();
    diagnostics
        .into_iter()
        .filter(|d| seen.insert((d.headline(), d.primary.clone())))
        .collect()
}

/// Extracts the diagnostics of a stream.
///
/// - [`StreamFormat::CargoJson`]: `compiler-message` lines with level `error` or `warning`; other
///   lines (text, malformed or absurdly nested JSON) are skipped, not fatal. The primary span is
///   followed through `expansion.span` to the outer call site (at most [`MAX_EXPANSION_DEPTH`]
///   hops).
/// - [`StreamFormat::Text`]: rustc human output, cut into blocks (see [`crate::filter_text`]).
/// - [`StreamFormat::Mixed`]: both; JSON lines first, then the text blocks.
///
/// Identical headline-plus-location pairs appear once. Build summaries are not diagnostics.
/// Errors in registry or git code are returned with `first_party == false` and still count as
/// issues.
///
/// # Errors
/// [`RtkError::StreamTooLarge`] for streams over [`crate::MAX_STREAM_BYTES`].
pub fn parse_diagnostics(stream: &[u8], fmt: StreamFormat) -> Result<Vec<Diagnostic>, RtkError> {
    parse_diagnostics_with(stream, fmt, &ThirdPartyMatcher::from_env())
}

/// [`parse_diagnostics`] with an explicit third-party matcher (instead of reading `CARGO_HOME`).
///
/// # Errors
/// [`RtkError::StreamTooLarge`] for streams over [`crate::MAX_STREAM_BYTES`].
pub fn parse_diagnostics_with(
    stream: &[u8],
    fmt: StreamFormat,
    matcher: &ThirdPartyMatcher,
) -> Result<Vec<Diagnostic>, RtkError> {
    check_size(stream)?;
    Ok(collect_diagnostics(&decode_stream(stream), fmt, matcher))
}

#[derive(Serialize)]
struct Report<'a> {
    status: &'static str,
    error_count: usize,
    errors: Vec<ReportError<'a>>,
}

#[derive(Serialize)]
struct ReportError<'a> {
    code: Option<&'a str>,
    message: &'a str,
    location: Option<&'a Location>,
    details: &'a [String],
}

/// Renders the structured report of `--json`: `{status, error_count, errors[{code, message,
/// location{file, line, column}, details[]}]}`. Only issues (errors) are listed. The reference
/// engine names the count `vulnerabilities_count`; this port uses `error_count`.
///
/// # Errors
/// [`RtkError::Json`] if serialisation fails (not expected for these types).
pub fn render_json_report(diagnostics: &[Diagnostic]) -> Result<String, RtkError> {
    let errors: Vec<ReportError<'_>> = diagnostics
        .iter()
        .filter(|d| d.is_issue())
        .map(|d| ReportError {
            code: d.code.as_deref(),
            message: &d.message,
            location: d.primary.as_ref(),
            details: &d.details,
        })
        .collect();
    let report = Report {
        status: if errors.is_empty() {
            "SUCCESS"
        } else {
            "FAILED"
        },
        error_count: errors.len(),
        errors,
    };
    Ok(serde_json::to_string_pretty(&report)?)
}
