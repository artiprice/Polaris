//! The RTK pipeline: store the raw stream, filter it, reconcile the ledger, report.

use std::collections::HashMap;

use jiff::Timestamp;
use polaris_sdk::ids::Cid;
use polaris_storage_cas::Cas;

use crate::detect::detect_decoded;
use crate::diag::{Diagnostic, ThirdPartyMatcher};
use crate::error::RtkError;
use crate::file::LedgerFile;
use crate::input::{check_size, decode_stream};
use crate::json::render_json_report;
use crate::ledger::{IssueId, LedgerSummary, exit_code};
use crate::text::{collect_diagnostics, filter_decoded};

/// What the engine does with a stream (the reference's `--auto`, `--json`, `--filter`, `--issues`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtkMode {
    /// Print the filtered diagnostics and reconcile the ledger.
    Auto,
    /// Print the structured JSON report; the ledger is untouched.
    Json,
    /// Print the cleaned text; the ledger is untouched.
    Filter,
    /// Reconcile the ledger and report the summary only.
    Issues,
}

/// Result of [`run`].
#[derive(Debug, Clone)]
pub struct RtkOutput {
    /// The filtered view (`Auto`, `Filter`), the JSON report (`Json`), or empty (`Issues`).
    pub filtered: String,
    /// The ledger summary; `None` in the modes that leave the ledger untouched.
    pub summary: Option<LedgerSummary>,
    /// Stable text for the agent (open count, locations, ledger counts, honesty note); empty when
    /// the ledger was not reconciled.
    pub view: String,
    /// Content id of the raw stream in the CAS: the filtered view is never the only copy.
    pub raw: Cid,
    /// 0 when no issue is open, 1 otherwise. For `Auto` and `Issues` this is the ledger's state;
    /// `Json` and `Filter` do not touch the ledger, so it reflects the issues of this stream.
    pub exit_code: i32,
}

/// The note every summary ends with.
const HONESTY_NOTE: &str =
    "Note: the ledger reflects the last stream piped in, not the current state of the tree.";

/// Renders the stable agent view: open count, each open issue with the `file:line` of its current
/// location (when the latest stream still shows it), the ledger counts and the honesty note.
#[must_use]
pub fn render_agent_view(summary: &LedgerSummary, diagnostics: &[Diagnostic]) -> String {
    let locations: HashMap<IssueId, String> = diagnostics
        .iter()
        .filter(|d| d.is_issue())
        .filter_map(|d| {
            let loc = d.primary.as_ref()?;
            Some((
                IssueId::from_message(&d.headline()),
                format!("{}:{}", loc.file, loc.line),
            ))
        })
        .collect();
    let mut out = format!(
        "RTK ledger: {} open issue(s), {} newly opened, {} newly closed.\n",
        summary.open.len(),
        summary.newly_opened,
        summary.newly_closed
    );
    for issue in &summary.open {
        out.push_str(&format!("- {} {}", issue.id, issue.text));
        if let Some(loc) = locations.get(&issue.id) {
            out.push_str(&format!(" ({loc})"));
        }
        out.push('\n');
    }
    out.push_str(HONESTY_NOTE);
    out.push('\n');
    out
}

/// Runs the RTK pipeline on a piped stream.
///
/// The raw bytes go into the CAS first, so nothing is lost whatever the filter drops. Then the
/// stream is decoded, its format detected, and the mode decides what happens: see [`RtkMode`].
/// Exit status semantics are in [`RtkOutput::exit_code`].
///
/// # Errors
/// [`RtkError::StreamTooLarge`] (checked before anything is stored), [`RtkError::Cas`] if the raw
/// stream cannot be stored, and [`RtkError::Io`], [`RtkError::LockTimeout`] or
/// [`RtkError::ConcurrentWrite`] from the ledger.
pub fn run(
    mode: RtkMode,
    stream: &[u8],
    file: &LedgerFile,
    now: Timestamp,
    store: &Cas,
) -> Result<RtkOutput, RtkError> {
    check_size(stream)?;
    let raw = store.put(stream)?;
    let decoded = decode_stream(stream);
    let format = detect_decoded(&decoded);
    let matcher = ThirdPartyMatcher::from_env();
    let diagnostics = collect_diagnostics(&decoded, format, &matcher);

    let (filtered, summary) = match mode {
        RtkMode::Auto => (
            filter_decoded(&decoded, format, &matcher),
            Some(file.update(&diagnostics, now)?),
        ),
        RtkMode::Issues => (String::new(), Some(file.update(&diagnostics, now)?)),
        RtkMode::Filter => (filter_decoded(&decoded, format, &matcher), None),
        RtkMode::Json => (render_json_report(&diagnostics)?, None),
    };
    let (view, exit_code) = match &summary {
        Some(s) => (render_agent_view(s, &diagnostics), exit_code(s)),
        None => (
            String::new(),
            i32::from(diagnostics.iter().any(Diagnostic::is_issue)),
        ),
    };
    Ok(RtkOutput {
        filtered,
        summary,
        view,
        raw,
        exit_code,
    })
}
