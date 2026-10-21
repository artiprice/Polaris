//! # polaris-tools-rtk
//!
//! RTK diagnostic engine, a Rust port of the reference `rtk-engine` (`@prime/tmp/setup`): it reads
//! a compiler or linter stream, drops the noise, and reconciles the errors that remain against an
//! atomic issue ledger (`.polaris/issues.md`).
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L3 — domain |
//! | Kind | lib |
//! | Phase | P1 |
//! | Parent facade | `polaris-tools` |
//!
//! ## Pipeline
//!
//! 1. [`decode_stream`]: BOM / UTF-16 / CRLF handling and terminal-escape stripping.
//! 2. [`detect_format`]: [`StreamFormat::CargoJson`], [`StreamFormat::Text`] or
//!    [`StreamFormat::Mixed`] (JSON lines interleaved with text, as `2>&1` produces).
//! 3. [`parse_diagnostics`] / [`filter_text`] / [`render_json_report`]: the diagnostic view.
//!    Registry and git paths (also with Windows separators and under a custom `CARGO_HOME`),
//!    code frames, `Compiling`/`Downloaded` lines, build summaries and consecutive duplicates are
//!    dropped; warnings are dropped whenever an error exists; macro-expanded errors are reported
//!    at the outer call site.
//! 4. [`Ledger::reconcile`]: stable message-derived ids ([`IssueId`]), open/closed state with
//!    RFC 3339 UTC timestamps, notes and unknown lines preserved. [`LedgerFile`] replaces the file
//!    atomically (temp file in the same directory, fsync, rename, bounded retry for files held by
//!    antivirus on Windows) under an advisory lock.
//! 5. [`run`] ties it together: the raw stream goes into the CAS first, [`exit_code`] is 0 when
//!    no issue is open and 1 otherwise.
//!
//! ## Honesty
//!
//! The ledger reflects the last stream piped in. It is not a statement about the current tree;
//! [`render_agent_view`] says so in every summary.

mod detect;
mod diag;
mod error;
mod file;
mod input;
mod json;
mod ledger;
mod run;
mod text;

pub use detect::{StreamFormat, detect_format};
pub use diag::{Diagnostic, Level, Location, ThirdPartyMatcher, is_summary_message};
pub use error::RtkError;
pub use file::{ISSUES_FILE_ENV, LedgerFile, LedgerStamp, find_workspace_root};
pub use input::{MAX_STREAM_BYTES, decode_stream};
pub use json::{
    MAX_EXPANSION_DEPTH, parse_diagnostics, parse_diagnostics_with, render_json_report,
};
pub use ledger::{
    IssueId, IssueLine, Ledger, LedgerLine, LedgerSummary, MAX_ISSUE_TEXT_CHARS, exit_code,
    normalise_message,
};
pub use run::{RtkMode, RtkOutput, render_agent_view, run};
pub use text::{filter_text, filter_text_with};
