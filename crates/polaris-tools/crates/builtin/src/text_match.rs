//! Tiered location of model-supplied `old_text`: exact bytes first, then whole lines with
//! trailing whitespace ignored, then indentation ignored, then common Unicode punctuation folded
//! to ASCII. A relaxed level is used only when it finds exactly one match (or `replace_all`), and
//! the level is reported so callers can surface how much fuzz was applied.
//!
//! Failures are written for the model: an ambiguous match names the lines, a missing one shows
//! the closest source lines so the next attempt can copy them exactly.

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

#[cfg(test)]
mod tests {
    use super::{MatchLevel, TextMatchError, replace_text};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn exact_match_is_preferred_and_must_be_unique() -> TestResult {
        let edited = replace_text("let a = 1;\nlet b = 2;\n", "b = 2", "b = 3", false)?;
        assert_eq!(edited.text, "let a = 1;\nlet b = 3;\n");
        assert_eq!(edited.level, MatchLevel::Exact);
        let error = replace_text("x\nx\n", "x", "y", false);
        assert!(
            matches!(&error, Err(TextMatchError::Ambiguous { count: 2, lines }) if lines == "1, 2"),
            "{error:?}"
        );
        let all = replace_text("x\nx\n", "x", "y", true)?;
        assert_eq!((all.text.as_str(), all.count), ("y\ny\n", 2));
        Ok(())
    }

    #[test]
    fn trailing_whitespace_and_crlf_are_absorbed() -> TestResult {
        let source = "fn main() {  \r\n    run();\r\n}\r\n";
        let edited = replace_text(
            source,
            "fn main() {\n    run();\n",
            "fn main() {\n    go();\n",
            false,
        )?;
        assert_eq!(edited.level, MatchLevel::TrailingWhitespace);
        assert_eq!(edited.text, "fn main() {\r\n    go();\r\n}\r\n");
        Ok(())
    }

    #[test]
    fn indentation_and_punctuation_levels_apply_in_order() -> TestResult {
        let source = "impl A {\n        call(x);\n}\n";
        let edited = replace_text(source, "call(x);", "call(y);", false)?;
        assert_eq!(
            edited.level,
            MatchLevel::Exact,
            "a substring still matches exactly"
        );
        let edited = replace_text(source, "\tcall(x);\n", "        call(y);\n", false)?;
        assert_eq!(edited.level, MatchLevel::Indentation);
        assert_eq!(edited.text, "impl A {\n        call(y);\n}\n");
        let quoted = "say(\"it’s — fine…\");\n";
        let edited = replace_text(quoted, "say(\"it's - fine...\");", "say(\"ok\");", false)?;
        assert_eq!(edited.level, MatchLevel::Punctuation);
        assert_eq!(edited.text, "say(\"ok\");\n");
        Ok(())
    }

    #[test]
    fn relaxed_levels_still_require_one_location() {
        let source = "  a();\n    a();\n";
        let error = replace_text(source, "a();\n", "b();\n", false);
        assert!(
            matches!(error, Err(TextMatchError::Ambiguous { count: 2, .. })),
            "{error:?}"
        );
    }

    #[test]
    fn missing_text_points_at_the_closest_lines() -> TestResult {
        let source = "fn alpha() {}\nfn parse_input(text: &str) -> Value {\n    todo!()\n}\n";
        let error = replace_text(
            source,
            "fn parse_input(text: String) -> Value {",
            "x",
            false,
        );
        let Err(TextMatchError::NotFound { hint }) = error else {
            return Err(format!("expected NotFound, got {error:?}").into());
        };
        assert!(hint.contains("line 2"), "{hint}");
        assert!(
            hint.contains("    2| fn parse_input(text: &str) -> Value {"),
            "{hint}"
        );
        assert!(matches!(
            replace_text("a", "", "b", false),
            Err(TextMatchError::Empty)
        ));
        Ok(())
    }
}
