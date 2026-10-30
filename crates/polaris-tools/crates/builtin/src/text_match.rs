//! Tiered location of model-supplied `old_text`: exact bytes first, then whole lines with
//! trailing whitespace ignored, then indentation ignored, then common Unicode punctuation folded
//! to ASCII. A relaxed level is used only when it finds exactly one match (or `replace_all`), and
//! the level is reported so callers can surface how much fuzz was applied.
//!
//! Failures are written for the model: an ambiguous match names the lines, a missing one shows
//! the closest source lines so the next attempt can copy them exactly.
//!
//! [`apply_hunks`] applies a model-written unified diff the same way: each hunk is found by its
//! old lines at the strictest level that matches, not by its line numbers.

/// How strictly `old_text` matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchLevel {
    /// Byte-exact substring.
    Exact,
    /// Whole lines equal after removing trailing whitespace (also absorbs CRLF vs LF).
    TrailingWhitespace,
    /// Whole lines equal after removing leading and trailing whitespace.
    Indentation,
    /// As [`Self::Indentation`], with typographic quotes, dashes, ellipses and no-break spaces
    /// folded to ASCII.
    Punctuation,
}

impl MatchLevel {
    /// Stable name used in receipts and UI.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::TrailingWhitespace => "trailing_whitespace",
            Self::Indentation => "indentation",
            Self::Punctuation => "punctuation",
        }
    }
}

/// The edited text and how it was located.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// Complete new source.
    pub text: String,
    /// Strictest level that located the target.
    pub level: MatchLevel,
    /// Number of replaced locations.
    pub count: usize,
}

/// Why `old_text` could not be applied; messages are addressed to the model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextMatchError {
    /// `old_text` is empty.
    #[error("old_text must not be empty")]
    Empty,
    /// More than one location matched and `replace_all` is false.
    #[error(
        "old_text matches {count} places (lines {lines}); include more surrounding lines to make it unique, or set replace_all"
    )]
    Ambiguous {
        /// Matching locations.
        count: usize,
        /// One-based start lines of the first matches.
        lines: String,
    },
    /// Nothing matched at any level.
    #[error("old_text was not found in the file{hint}")]
    NotFound {
        /// The closest source lines, or empty.
        hint: String,
    },
    /// A patch is not a usable single-file unified diff, or one of several hunks failed.
    #[error("{0}")]
    Malformed(String),
}

const MAX_REPORTED_LINES: usize = 8;
const MAX_HINT_LINES: usize = 12;
const MAX_HINT_BYTES: usize = 1200;

/// Replace `old` with `new` in `source` using the strictest level that matches.
/// # Errors
/// Empty `old`, an ambiguous match without `replace_all`, or no match.
pub fn replace_text(
    source: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Replacement, TextMatchError> {
    if old.is_empty() {
        return Err(TextMatchError::Empty);
    }
    let exact: Vec<usize> = source.match_indices(old).map(|(at, _)| at).collect();
    if !exact.is_empty() {
        if exact.len() > 1 && !replace_all {
            return Err(ambiguous(source, &exact));
        }
        let text = if replace_all {
            source.replace(old, new)
        } else {
            source.replacen(old, new, 1)
        };
        return Ok(Replacement {
            text,
            level: MatchLevel::Exact,
            count: exact.len(),
        });
    }
    let needle: Vec<&str> = old.lines().collect();
    if needle.iter().all(|line| line.trim().is_empty()) {
        return Err(TextMatchError::NotFound {
            hint: String::new(),
        });
    }
    let lines = source_lines(source);
    for level in [
        MatchLevel::TrailingWhitespace,
        MatchLevel::Indentation,
        MatchLevel::Punctuation,
    ] {
        let windows = matching_windows(source, &lines, &needle, level);
        if windows.is_empty() {
            continue;
        }
        if windows.len() > 1 && !replace_all {
            let starts: Vec<usize> = windows
                .iter()
                .filter_map(|start| lines.get(*start).map(|line| line.start))
                .collect();
            return Err(ambiguous(source, &starts));
        }
        let replacement = new.strip_suffix('\n').unwrap_or(new);
        let mut text = String::with_capacity(source.len().saturating_add(new.len()));
        let mut cursor = 0_usize;
        for start in &windows {
            let (Some(first), Some(last)) = (
                lines.get(*start),
                lines.get(start.saturating_add(needle.len()).saturating_sub(1)),
            ) else {
                continue;
            };
            text.push_str(source.get(cursor..first.start).unwrap_or_default());
            if first.crlf {
                text.push_str(&replacement.replace("\r\n", "\n").replace('\n', "\r\n"));
            } else {
                text.push_str(replacement);
            }
            cursor = last.content_end;
        }
        text.push_str(source.get(cursor..).unwrap_or_default());
        return Ok(Replacement {
            text,
            level,
            count: windows.len(),
        });
    }
    Err(TextMatchError::NotFound {
        hint: nearest_hint(source, &lines, &needle),
    })
}

#[derive(Debug, Clone, Copy)]
struct SourceLine {
    start: usize,
    /// End of the line's text, before `\r\n` or `\n`.
    content_end: usize,
    crlf: bool,
}

fn source_lines(source: &str) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    for line in source.split_inclusive('\n') {
        let end = start.saturating_add(line.len());
        let (content_end, crlf) = if line.ends_with("\r\n") {
            (end.saturating_sub(2), true)
        } else if line.ends_with('\n') {
            (end.saturating_sub(1), false)
        } else {
            (end, false)
        };
        lines.push(SourceLine {
            start,
            content_end,
            crlf,
        });
        start = end;
    }
    lines
}

fn normalize(line: &str, level: MatchLevel) -> std::borrow::Cow<'_, str> {
    match level {
        MatchLevel::Exact => std::borrow::Cow::Borrowed(line),
        MatchLevel::TrailingWhitespace => std::borrow::Cow::Borrowed(line.trim_end()),
        MatchLevel::Indentation => std::borrow::Cow::Borrowed(line.trim()),
        MatchLevel::Punctuation => {
            let mut folded = String::with_capacity(line.len());
            for ch in line.trim().chars() {
                match ch {
                    '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => folded.push('\''),
                    '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => folded.push('"'),
                    '\u{2010}'..='\u{2015}' | '\u{2212}' => folded.push('-'),
                    '\u{2026}' => folded.push_str("..."),
                    '\u{00A0}' | '\u{2007}' | '\u{202F}' => folded.push(' '),
                    other => folded.push(other),
                }
            }
            std::borrow::Cow::Owned(folded)
        }
    }
}

/// Non-overlapping start indexes of line windows equal to `needle` at `level`.
fn matching_windows(
    source: &str,
    lines: &[SourceLine],
    needle: &[&str],
    level: MatchLevel,
) -> Vec<usize> {
    let wanted: Vec<std::borrow::Cow<'_, str>> =
        needle.iter().map(|line| normalize(line, level)).collect();
    let mut found = Vec::new();
    let mut index = 0_usize;
    while index.saturating_add(needle.len()) <= lines.len() {
        let equal = wanted.iter().enumerate().all(|(offset, want)| {
            lines
                .get(index.saturating_add(offset))
                .and_then(|line| source.get(line.start..line.content_end))
                .is_some_and(|text| normalize(text, level) == *want)
        });
        if equal {
            found.push(index);
            index = index.saturating_add(needle.len().max(1));
        } else {
            index = index.saturating_add(1);
        }
    }
    found
}

fn line_of(source: &str, byte: usize) -> usize {
    source
        .get(..byte)
        .map_or(0, |prefix| prefix.matches('\n').count())
        .saturating_add(1)
}

fn ambiguous(source: &str, starts: &[usize]) -> TextMatchError {
    let mut lines: Vec<String> = starts
        .iter()
        .take(MAX_REPORTED_LINES)
        .map(|at| line_of(source, *at).to_string())
        .collect();
    if starts.len() > MAX_REPORTED_LINES {
        lines.push("…".to_owned());
    }
    TextMatchError::Ambiguous {
        count: starts.len(),
        lines: lines.join(", "),
    }
}

fn words(line: &str) -> std::collections::BTreeSet<&str> {
    line.split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .filter(|word| word.len() > 1)
        .collect()
}

/// The source lines that share the most words with the needle's first meaningful line.
fn nearest_hint(source: &str, lines: &[SourceLine], needle: &[&str]) -> String {
    let Some((needle_offset, anchor)) = needle
        .iter()
        .enumerate()
        .find(|(_, line)| !line.trim().is_empty())
    else {
        return String::new();
    };
    let wanted = words(anchor);
    if wanted.is_empty() {
        return String::new();
    }
    let best = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let text = source.get(line.start..line.content_end)?;
            let score = words(text).intersection(&wanted).count();
            (score > 0).then_some((score, index))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then(right.1.cmp(&left.1)));
    let Some((_, anchor_index)) = best else {
        return String::new();
    };
    let first = anchor_index.saturating_sub(needle_offset);
    let count = needle.len().clamp(1, MAX_HINT_LINES);
    let mut hint = format!(
        "; the closest text starts at line {} (copy it exactly, including indentation):",
        first.saturating_add(1)
    );
    for (offset, line) in lines.iter().skip(first).take(count).enumerate() {
        let text = source.get(line.start..line.content_end).unwrap_or_default();
        let row = format!(
            "\n{:>5}| {text}",
            first.saturating_add(offset).saturating_add(1)
        );
        if hint.len().saturating_add(row.len()) > MAX_HINT_BYTES {
            hint.push_str("\n    …");
            break;
        }
        hint.push_str(&row);
    }
    hint
}

/// One hunk of a model-written unified diff, located by content rather than by line numbers.
#[derive(Debug, Default)]
struct Hunk<'a> {
    /// One-based old start claimed by the header; it only breaks ties.
    claimed: usize,
    /// Hunk body in order: `' '` context, `'-'` deletion, `'+'` addition.
    lines: Vec<(u8, &'a str)>,
    /// Final-newline state a `\ No newline at end of file` marker set: after a deletion the new
    /// file ends with a newline (`Some(true)`), after context or an addition it does not.
    final_newline: Option<bool>,
}

impl Hunk<'_> {
    fn old(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter(|(marker, _)| *marker != b'+')
            .map(|(_, text)| *text)
            .collect()
    }
}

/// Parse the hunks of a single-file unified diff. Headers are optional, counts are ignored and a
/// bare empty line inside a hunk is blank context (models often drop its leading space).
fn parse_hunks(patch: &str) -> Result<Vec<Hunk<'_>>, TextMatchError> {
    let mut hunks: Vec<Hunk<'_>> = Vec::new();
    let mut last_marker = b' ';
    for line in patch.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(header) = line.strip_prefix("@@") {
            let claimed = header
                .split_whitespace()
                .next()
                .and_then(|range| range.strip_prefix('-'))
                .and_then(|range| range.split(',').next())
                .and_then(|start| start.parse().ok())
                .unwrap_or(0);
            hunks.push(Hunk {
                claimed,
                ..Hunk::default()
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            // Only headers may precede the first hunk.
            if line.is_empty()
                || [
                    "diff ", "index ", "--- ", "+++ ", "new file", "old mode", "new mode",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            {
                continue;
            }
            return Err(TextMatchError::Malformed(format!(
                "unexpected line before the first @@ hunk header: {line:?}"
            )));
        };
        if line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("diff ") {
            return Err(TextMatchError::Malformed(
                "apply_patch changes one file; call it once per file".to_owned(),
            ));
        }
        if line.starts_with('\\') {
            hunk.final_newline = Some(last_marker == b'-');
            continue;
        }
        let (marker, text) = match line.as_bytes().first() {
            None => (b' ', ""),
            Some(marker @ (b' ' | b'-' | b'+')) => (*marker, line.get(1..).unwrap_or_default()),
            Some(_) => {
                return Err(TextMatchError::Malformed(format!(
                    "hunk lines must start with ' ', '-' or '+': {line:?}"
                )));
            }
        };
        last_marker = marker;
        hunk.lines.push((marker, text));
    }
    if hunks.is_empty() {
        return Err(TextMatchError::Malformed(
            "the patch has no @@ hunk".to_owned(),
        ));
    }
    if hunks
        .iter()
        .all(|hunk| hunk.lines.iter().all(|(marker, _)| *marker == b' '))
    {
        return Err(TextMatchError::Malformed(
            "the patch adds and removes nothing".to_owned(),
        ));
    }
    Ok(hunks)
}

/// Apply a model-written single-file unified diff by locating each hunk's old lines in
/// `source`: exactly first, then with trailing whitespace, indentation and punctuation relaxed.
/// Hunks apply in order and must each match one place after the previous hunk; header line
/// numbers only choose between equal candidates, and hunk counts are ignored. Untouched and
/// context lines keep their exact bytes and line endings, added lines take the ending of the
/// text around them, and the file's final-newline state is kept unless a
/// `\ No newline at end of file` marker changes it.
/// # Errors
/// A malformed patch, an ambiguous hunk, or a hunk whose old lines are not in the file.
pub fn apply_hunks(source: &str, patch: &str) -> Result<Replacement, TextMatchError> {
    let hunks = parse_hunks(patch)?;
    let lines = source_lines(source);
    let default_ending = if lines.iter().any(|line| line.crlf) {
        "\r\n"
    } else {
        "\n"
    };
    let text_of = |index: usize| {
        lines
            .get(index)
            .and_then(|line| source.get(line.start..line.content_end))
            .unwrap_or_default()
    };
    let ending_of = |index: usize| -> &str {
        let Some(line) = lines.get(index) else {
            return default_ending;
        };
        if line.crlf {
            "\r\n"
        } else if source
            .get(line.content_end..)
            .is_some_and(|rest| rest.starts_with('\n'))
        {
            "\n"
        } else {
            ""
        }
    };
    // Around an insertion: the ending of the line it displaces, else of the line before it.
    let near = |index: usize| -> &str {
        match ending_of(index) {
            "" => match index.checked_sub(1).map(ending_of) {
                Some(ending) if !ending.is_empty() => ending,
                _ => default_ending,
            },
            ending => ending,
        }
    };
    let mut output: Vec<(&str, &str)> = Vec::with_capacity(lines.len());
    let mut cursor = 0_usize;
    let mut loosest = MatchLevel::Exact;
    let mut final_newline = source.is_empty() || source.ends_with('\n');
    for (number, hunk) in hunks.iter().enumerate() {
        let old = hunk.old();
        let start = if old.is_empty() {
            // A pure insertion goes after the claimed line.
            hunk.claimed.clamp(cursor, lines.len())
        } else {
            let (start, level) = locate(source, &lines, cursor, &old, hunk.claimed)
                .map_err(|error| error.in_hunk(number, hunks.len()))?;
            loosest = loosest.max(level);
            start
        };
        output.extend((cursor..start).map(|index| (text_of(index), ending_of(index))));
        let mut at = start;
        for (marker, text) in &hunk.lines {
            match marker {
                b'+' => output.push((text, near(at))),
                b'-' => at = at.saturating_add(1),
                _ => {
                    output.push((text_of(at), ending_of(at)));
                    at = at.saturating_add(1);
                }
            }
        }
        cursor = at;
        if let Some(newline) = hunk.final_newline {
            final_newline = newline;
        }
    }
    output.extend((cursor..lines.len()).map(|index| (text_of(index), ending_of(index))));
    let last = output.len().saturating_sub(1);
    let mut text = String::with_capacity(source.len());
    for (index, (line, ending)) in output.iter().enumerate() {
        text.push_str(line);
        let ending = match (index == last, ending.is_empty()) {
            (false, true) => default_ending,
            (true, _) if !final_newline => "",
            (true, true) => default_ending,
            _ => ending,
        };
        text.push_str(ending);
    }
    Ok(Replacement {
        text,
        level: loosest,
        count: hunks.len(),
    })
}

/// The one start line at or after `from` whose window equals `needle` at the strictest level
/// that finds any; the claimed one-based line breaks a tie.
fn locate(
    source: &str,
    lines: &[SourceLine],
    from: usize,
    needle: &[&str],
    claimed: usize,
) -> Result<(usize, MatchLevel), TextMatchError> {
    let tail = lines.get(from..).unwrap_or_default();
    for level in [
        MatchLevel::Exact,
        MatchLevel::TrailingWhitespace,
        MatchLevel::Indentation,
        MatchLevel::Punctuation,
    ] {
        let found: Vec<usize> = matching_windows(source, tail, needle, level)
            .into_iter()
            .map(|index| index.saturating_add(from))
            .collect();
        match found.as_slice() {
            [] => {}
            [one] => return Ok((*one, level)),
            many => {
                if let Some(start) = many.iter().find(|start| start.saturating_add(1) == claimed) {
                    return Ok((*start, level));
                }
                let starts: Vec<usize> = many
                    .iter()
                    .filter_map(|start| lines.get(*start).map(|line| line.start))
                    .collect();
                return Err(ambiguous(source, &starts));
            }
        }
    }
    Err(TextMatchError::NotFound {
        hint: nearest_hint(source, lines, needle),
    })
}

impl TextMatchError {
    /// Patch wording for a hunk failure, naming the hunk when a patch has several.
    fn in_hunk(self, number: usize, total: usize) -> Self {
        let place = if total < 2 {
            String::new()
        } else {
            format!("hunk {} of {total}: ", number.saturating_add(1))
        };
        match self {
            Self::Malformed(text) => Self::Malformed(format!("{place}{text}")),
            Self::NotFound { hint } => Self::Malformed(format!(
                "{place}the hunk's old lines (context and '-' lines) were not found in the file{hint}"
            )),
            Self::Ambiguous { count, lines } => Self::Malformed(format!(
                "{place}the hunk's old lines match {count} places (lines {lines}); add context lines"
            )),
            Self::Empty => Self::Empty,
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-tools/crates/builtin/src/text_match.rs"]
mod tests;
