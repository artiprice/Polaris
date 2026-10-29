//! The issue ledger (`.polaris/issues.md`): stable ids, open/closed state, tolerant parsing.
//!
//! ```text
//! - [ ] error[E0425]: cannot find value `config` in this scope | ID: #ERR-a1b2c3d4e5f60718 | Opened: 2026-09-22T11:04:00Z
//! - [x] ~~error[E0308]: mismatched types~~ | ID: #ERR-0123456789abcdef | Opened: 2026-09-21T09:12:00Z | Closed: 2026-09-22T11:04:00Z
//! ```
//!
//! Every other line is kept verbatim as [`LedgerLine::Raw`], so users can keep notes in the file.

use std::collections::HashSet;
use std::fmt;

use jiff::Timestamp;
use polaris_storage_cas::content_id;

use crate::diag::Diagnostic;

/// Longest issue text (in characters) the ledger stores; longer messages are cut.
pub const MAX_ISSUE_TEXT_CHARS: usize = 400;

/// Normalises a message for the ledger: control characters and whitespace runs become one space,
/// `|` becomes `¦` (so a message cannot forge the ` | ID: ` / ` | Closed: ` fields), the result is
/// trimmed and cut to [`MAX_ISSUE_TEXT_CHARS`]. Idempotent.
#[must_use]
pub fn normalise_message(message: &str) -> String {
    let mut out = String::with_capacity(message.len().min(MAX_ISSUE_TEXT_CHARS * 4));
    let mut pending_space = false;
    for c in message.chars() {
        if c.is_whitespace() || c.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(if c == '|' { '\u{a6}' } else { c });
    }
    let cut: String = out.chars().take(MAX_ISSUE_TEXT_CHARS).collect();
    cut.trim_end().to_owned()
}

/// Stable issue id: `#ERR-` plus the first 16 hex digits of the BLAKE3 hash of the normalised
/// message. The same failure keeps its id across runs and when its line number shifts; identical
/// messages in different files share one id (and one ledger entry).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IssueId(String);

impl IssueId {
    const PREFIX: &'static str = "#ERR-";

    /// Derives the id from a message (normalised first, see [`normalise_message`]).
    #[must_use]
    pub fn from_message(message: &str) -> Self {
        Self::of_normalised(&normalise_message(message))
    }

    fn of_normalised(normalised: &str) -> Self {
        let hex = content_id(normalised.as_bytes()).to_hex();
        Self(format!(
            "{}{}",
            Self::PREFIX,
            hex.chars().take(16).collect::<String>()
        ))
    }

    /// Accepts `#ERR-` followed by 1 to 64 hex digits (any case).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix(Self::PREFIX)?;
        let ok = (1..=64).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| Self(text.to_owned()))
    }

    /// The id as written in the ledger.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IssueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A timestamp field must be non-empty and must not contain the `|` field separator.
fn is_timestamp_field(field: &str) -> bool {
    !field.is_empty() && !field.contains('|')
}

/// One issue entry of the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueLine {
    /// Stable id.
    pub id: IssueId,
    /// Normalised headline (`error[E0425]: ...`).
    pub text: String,
    /// When the issue was first seen (RFC 3339 UTC, as written in the file).
    pub opened: String,
    /// When the issue disappeared; `None` while open.
    pub closed: Option<String>,
}

impl IssueLine {
    /// `true` while the issue has no close time.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.closed.is_none()
    }

    fn render(&self) -> String {
        match &self.closed {
            None => format!(
                "- [ ] {} | ID: {} | Opened: {}",
                self.text, self.id, self.opened
            ),
            Some(closed) => format!(
                "- [x] ~~{}~~ | ID: {} | Opened: {} | Closed: {closed}",
                self.text, self.id, self.opened
            ),
        }
    }

    /// Parses an issue line; splits from the right, so text containing the field markers cannot
    /// shift the metadata. Anything that does not fit exactly is not an issue line.
    fn parse(line: &str) -> Option<Self> {
        if let Some(rest) = line.strip_prefix("- [ ] ") {
            let (head, opened) = rest.rsplit_once(" | Opened: ")?;
            let (text, id) = head.rsplit_once(" | ID: ")?;
            if !is_timestamp_field(opened) {
                return None;
            }
            return Some(Self {
                id: IssueId::parse(id)?,
                text: text.to_owned(),
                opened: opened.to_owned(),
                closed: None,
            });
        }
        let rest = line.strip_prefix("- [x] ~~")?;
        let (head, closed) = rest.rsplit_once(" | Closed: ")?;
        let (head, opened) = head.rsplit_once(" | Opened: ")?;
        let (struck, id) = head.rsplit_once(" | ID: ")?;
        let text = struck.strip_suffix("~~")?;
        if !(is_timestamp_field(opened) && is_timestamp_field(closed)) {
            return None;
        }
        Some(Self {
            id: IssueId::parse(id)?,
            text: text.to_owned(),
            opened: opened.to_owned(),
            closed: Some(closed.to_owned()),
        })
    }
}

/// A line of the ledger file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerLine {
    /// An open or closed issue entry.
    Issue(IssueLine),
    /// Any other line, preserved verbatim (notes, headings, blank lines, unparsable entries).
    Raw(String),
}

/// Result of a [`Ledger::reconcile`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LedgerSummary {
    /// All issues that are open after reconciliation, in ledger order.
    pub open: Vec<IssueLine>,
    /// Issues closed by this reconciliation.
    pub newly_closed: u32,
    /// Issues opened by this reconciliation.
    pub newly_opened: u32,
}

/// Exit status of the RTK command: 0 when no issue is open after reconciliation, 1 otherwise.
/// It reflects the ledger, not the build's own exit code.
#[must_use]
pub fn exit_code(summary: &LedgerSummary) -> i32 {
    i32::from(!summary.open.is_empty())
}

/// RFC 3339 UTC with second precision (`2026-09-22T11:04:00Z`).
fn format_timestamp(now: Timestamp) -> String {
    Timestamp::from_second(now.as_second())
        .unwrap_or(now)
        .to_string()
}

/// The ordered lines of an issues file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    lines: Vec<LedgerLine>,
}

impl Ledger {
    /// Parses ledger text. Tolerant: `LF`, `CRLF` and a missing final newline are accepted, and
    /// every line that is not a well-formed issue entry is kept as [`LedgerLine::Raw`].
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut pieces: Vec<&str> = text.split('\n').collect();
        if pieces.last() == Some(&"") {
            pieces.pop();
        }
        let lines = pieces
            .into_iter()
            .map(|piece| {
                let line = piece.trim_end_matches('\r');
                IssueLine::parse(line)
                    .map_or_else(|| LedgerLine::Raw(line.to_owned()), LedgerLine::Issue)
            })
            .collect();
        Self { lines }
    }

    /// The lines in file order.
    #[must_use]
    pub fn lines(&self) -> &[LedgerLine] {
        &self.lines
    }

    /// Renders the ledger with `LF` line endings and a final newline (empty for no lines).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                LedgerLine::Issue(issue) => out.push_str(&issue.render()),
                LedgerLine::Raw(raw) => out.push_str(raw),
            }
            out.push('\n');
        }
        out
    }

    /// Reconciles the ledger with the issues of the latest stream.
    ///
    /// - An open issue that is still present stays untouched (its `Opened` time is kept).
    /// - An open issue that vanished is closed with `now`.
    /// - A present error without an open entry is appended as open with `now`; this includes an
    ///   error that was closed earlier and returned (the closed entry stays as history).
    /// - Warnings, build summaries and `Raw` lines never change; reconciling the same diagnostics
    ///   twice is a no-op.
    pub fn reconcile(&mut self, current: &[Diagnostic], now: Timestamp) -> LedgerSummary {
        let stamp = format_timestamp(now);
        let mut seen: HashSet<IssueId> = HashSet::new();
        let mut wanted: Vec<(IssueId, String)> = Vec::new();
        for diagnostic in current.iter().filter(|d| d.is_issue()) {
            let text = normalise_message(&diagnostic.headline());
            let id = IssueId::of_normalised(&text);
            if seen.insert(id.clone()) {
                wanted.push((id, text));
            }
        }

        let mut still_open: HashSet<IssueId> = HashSet::new();
        let mut newly_closed = 0u32;
        for line in &mut self.lines {
            if let LedgerLine::Issue(issue) = line
                && issue.is_open()
            {
                if seen.contains(&issue.id) {
                    still_open.insert(issue.id.clone());
                } else {
                    issue.closed = Some(stamp.clone());
                    newly_closed = newly_closed.saturating_add(1);
                }
            }
        }

        let mut newly_opened = 0u32;
        for (id, text) in wanted {
            if !still_open.contains(&id) {
                self.lines.push(LedgerLine::Issue(IssueLine {
                    id,
                    text,
                    opened: stamp.clone(),
                    closed: None,
                }));
                newly_opened = newly_opened.saturating_add(1);
            }
        }

        let open = self
            .lines
            .iter()
            .filter_map(|line| match line {
                LedgerLine::Issue(issue) if issue.is_open() => Some(issue.clone()),
                _ => None,
            })
            .collect();
        LedgerSummary {
            open,
            newly_closed,
            newly_opened,
        }
    }
}
