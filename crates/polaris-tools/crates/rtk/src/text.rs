//! Text path: block segmentation of rustc output, the filter, and text-to-diagnostic parsing.

use std::borrow::Cow;

use crate::detect::{StreamFormat, detect_decoded, is_json_line};
use crate::diag::{Diagnostic, Level, Location, ThirdPartyMatcher, is_summary_message};
use crate::error::RtkError;
use crate::input::{check_size, decode_stream};
use crate::json::{dedupe, parse_json_lines};

/// One diagnostic block of rustc text output: a header line plus the lines up to the next blank
/// line or header.
pub(crate) struct Block<'a> {
    level: Level,
    code: Option<&'a str>,
    message: &'a str,
    header: &'a str,
    body: Vec<&'a str>,
}

/// Header of a block: `error[CODE]: msg`, `error: msg`, `warning[CODE]: msg` or `warning: msg`.
fn parse_header(line: &str) -> Option<(Level, Option<&str>, &str)> {
    for (label, level) in [("error", Level::Error), ("warning", Level::Warning)] {
        let Some(rest) = line.strip_prefix(label) else {
            continue;
        };
        if let Some(coded) = rest.strip_prefix('[') {
            let (code, message) = coded.split_once("]:")?;
            return Some((level, Some(code), message.trim()));
        }
        if let Some(message) = rest.strip_prefix(':')
            && (message.is_empty() || message.starts_with(char::is_whitespace))
        {
            return Some((level, None, message.trim()));
        }
    }
    None
}

/// Splits text into blocks; build summaries (`aborting due to`, `could not compile`, ...) are
/// recognised as blocks and then dropped, so they are never diagnostics.
pub(crate) fn segment(text: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut current: Option<Block<'_>> = None;
    for line in text.lines() {
        if let Some((level, code, message)) = parse_header(line) {
            blocks.extend(current.take());
            current = Some(Block {
                level,
                code,
                message,
                header: line,
                body: Vec::new(),
            });
        } else if current.is_some() && line.trim().is_empty() {
            blocks.extend(current.take());
        } else if let Some(block) = current.as_mut() {
            block.body.push(line);
        }
    }
    blocks.extend(current);
    blocks.retain(|b| !is_summary_message(b.message));
    blocks
}

/// A line of a block that survives filtering.
enum Kept<'a> {
    /// A `-->` location line.
    Location(&'a str),
    /// A `note:` / `help:` line (also in the `= note:` form).
    Detail(&'a str),
    /// An indented continuation of the preceding detail.
    Continuation(&'a str),
}

impl<'a> Kept<'a> {
    fn line(&self) -> &'a str {
        match self {
            Self::Location(l) | Self::Detail(l) | Self::Continuation(l) => l,
        }
    }
}

fn is_detail_start(trimmed: &str) -> bool {
    ["= note:", "= help:", "note:", "help:"]
        .iter()
        .any(|p| trimmed.starts_with(p))
}

/// An indented line that is not part of a source frame (no `|`, no `^`, no line-number gutter).
fn is_continuation(line: &str, trimmed: &str) -> bool {
    line.starts_with("  ")
        && !trimmed.is_empty()
        && !line.contains('|')
        && !line.contains('^')
        && !trimmed.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '~' | '-'))
}

/// The lines of `block` that are kept: locations, notes/helps and their continuations. Lines that
/// name a registry or git path are dropped, consecutive duplicates are suppressed, code frames
/// (`|`, `^`) and everything else are discarded.
fn kept_lines<'a>(block: &Block<'a>, matcher: &ThirdPartyMatcher) -> Vec<Kept<'a>> {
    let mut kept: Vec<Kept<'a>> = Vec::new();
    let mut after_detail = false;
    for &line in &block.body {
        let trimmed = line.trim_start();
        let kind = if trimmed.starts_with("-->") {
            Some(Kept::Location(line))
        } else if is_detail_start(trimmed) {
            Some(Kept::Detail(line))
        } else if after_detail && is_continuation(line, trimmed) {
            Some(Kept::Continuation(line))
        } else {
            None
        };
        let Some(kind) = kind else {
            after_detail = false;
            continue;
        };
        if matcher.is_third_party(line) {
            after_detail = false;
            continue;
        }
        after_detail = !matches!(kind, Kept::Location(_));
        if kept.last().map(Kept::line) != Some(line) {
            kept.push(kind);
        }
    }
    kept
}

/// Parses `file:line:column` (the text after `-->`); the file may contain `:` (`C:\...`).
fn parse_location(text: &str) -> Option<Location> {
    let mut parts = text.trim().rsplitn(3, ':');
    let column = parts.next()?.trim().parse().ok()?;
    let line = parts.next()?.trim().parse().ok()?;
    let file = parts.next()?.trim();
    (!file.is_empty()).then(|| Location {
        file: file.to_owned(),
        line,
        column,
    })
}

/// Converts a block into a [`Diagnostic`].
fn block_to_diagnostic(block: &Block<'_>, matcher: &ThirdPartyMatcher) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(block.level, block.code, block.message);
    let location = block
        .body
        .iter()
        .find_map(|l| l.trim_start().strip_prefix("-->"))
        .map(str::trim);
    diagnostic.primary = location.and_then(parse_location);
    diagnostic.first_party = !location.is_some_and(|l| matcher.is_third_party(l));
    for kept in kept_lines(block, matcher) {
        match kept {
            Kept::Location(_) => {}
            Kept::Detail(line) => {
                let text = line.trim();
                diagnostic
                    .details
                    .push(text.strip_prefix("= ").unwrap_or(text).to_owned());
            }
            Kept::Continuation(line) => {
                if let Some(last) = diagnostic.details.last_mut() {
                    last.push(' ');
                    last.push_str(line.trim());
                }
            }
        }
    }
    diagnostic
}

/// Diagnostics found in text (blocks of rustc human output).
pub(crate) fn text_diagnostics(text: &str, matcher: &ThirdPartyMatcher) -> Vec<Diagnostic> {
    segment(text)
        .iter()
        .map(|b| block_to_diagnostic(b, matcher))
        .collect()
}

/// The input split by line kind.
pub(crate) struct Split<'a> {
    /// One-line JSON objects.
    pub(crate) json_lines: Vec<&'a str>,
    /// Everything else, to be read as rustc text.
    pub(crate) text: Cow<'a, str>,
}

/// Splits decoded text according to the stream format. `Text` treats everything as text,
/// `CargoJson` only reads JSON lines (other lines are skipped), `Mixed` reads both.
pub(crate) fn split(decoded: &str, format: StreamFormat) -> Split<'_> {
    match format {
        StreamFormat::Text => Split {
            json_lines: Vec::new(),
            text: Cow::Borrowed(decoded),
        },
        StreamFormat::CargoJson | StreamFormat::Mixed => {
            let mut json_lines = Vec::new();
            let mut text = String::new();
            for line in decoded.lines() {
                if is_json_line(line.trim()) {
                    json_lines.push(line);
                } else if format == StreamFormat::Mixed {
                    text.push_str(line);
                    text.push('\n');
                }
            }
            Split {
                json_lines,
                text: Cow::Owned(text),
            }
        }
    }
}

/// Everything a stream contributes, as diagnostics (JSON first, then text), deduplicated.
pub(crate) fn collect_diagnostics(
    decoded: &str,
    format: StreamFormat,
    matcher: &ThirdPartyMatcher,
) -> Vec<Diagnostic> {
    let parts = split(decoded, format);
    let mut all = parse_json_lines(&parts.json_lines, matcher);
    all.extend(text_diagnostics(&parts.text, matcher));
    dedupe(all)
}

/// Dedup key shared by the JSON and text views of one error.
fn key(d: &Diagnostic) -> (String, Option<&Location>) {
    (d.headline(), d.primary.as_ref())
}

/// Filters decoded text for the view; see [`filter_text`].
pub(crate) fn filter_decoded(
    decoded: &str,
    format: StreamFormat,
    matcher: &ThirdPartyMatcher,
) -> String {
    let parts = split(decoded, format);
    let json = dedupe(parse_json_lines(&parts.json_lines, matcher));
    let blocks = segment(&parts.text);
    let has_error =
        json.iter().any(Diagnostic::is_issue) || blocks.iter().any(|b| b.level == Level::Error);

    let mut rendered: Vec<Vec<String>> = Vec::new();
    for d in json
        .iter()
        .filter(|d| !has_error || d.level == Level::Error)
    {
        rendered.push(d.render().lines().map(str::to_owned).collect());
    }
    for block in blocks
        .iter()
        .filter(|b| !has_error || b.level == Level::Error)
    {
        let as_diagnostic = block_to_diagnostic(block, matcher);
        if json.iter().any(|d| key(d) == key(&as_diagnostic)) {
            continue;
        }
        let mut lines = vec![block.header.to_owned()];
        lines.extend(
            kept_lines(block, matcher)
                .iter()
                .map(|k| k.line().to_owned()),
        );
        rendered.push(lines);
    }
    rendered.dedup();

    let mut out = String::new();
    for (i, block) in rendered.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for line in block {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Cleans a compiler or linter stream into the diagnostics that matter.
///
/// The format is detected; text is cut into blocks, JSON lines are rendered as blocks. Registry
/// and git paths, code frames, `Compiling`/`Downloaded` noise, build summaries and consecutive
/// duplicates are removed, and warning blocks are dropped whenever an error exists. Kept text
/// lines are copied verbatim, never rewritten.
///
/// # Errors
/// [`RtkError::StreamTooLarge`] for streams over [`crate::MAX_STREAM_BYTES`].
pub fn filter_text(stream: &[u8]) -> Result<String, RtkError> {
    filter_text_with(stream, &ThirdPartyMatcher::from_env())
}

/// [`filter_text`] with an explicit third-party matcher (instead of reading `CARGO_HOME`).
///
/// # Errors
/// [`RtkError::StreamTooLarge`] for streams over [`crate::MAX_STREAM_BYTES`].
pub fn filter_text_with(stream: &[u8], matcher: &ThirdPartyMatcher) -> Result<String, RtkError> {
    check_size(stream)?;
    let decoded = decode_stream(stream);
    let format = detect_decoded(&decoded);
    Ok(filter_decoded(&decoded, format, matcher))
}
