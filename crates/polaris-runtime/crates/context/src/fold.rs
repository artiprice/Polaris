//! Lossless tool-output folding backed by CAS.

use std::{collections::HashMap, sync::Mutex};

use bytes::Bytes;
use polaris_sdk::ids::{Cid, ShortRef};
use polaris_storage_cas::Cas;
use polaris_tools_rtk::{detect_format, parse_diagnostics};

use crate::ContextError;

/// Origin class for output-folding policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// Compiler or linter output, eligible for RTK diagnostic rendering.
    Compiler,
    /// File content, optionally tracked for repeated unchanged reads.
    File,
    /// Any other tool output.
    Other,
}

/// Optional identity of a file-read result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    /// Caller-provided workspace-relative path. No filesystem validation is performed here.
    pub path: String,
    /// Turn number associated with this read.
    pub turn: u32,
}

/// Per-call output folding parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldConfig {
    /// UTF-8 output at or below this byte count remains inline.
    pub inline_threshold: usize,
    /// Number of leading lines displayed in a generic head/tail fold.
    pub head_lines: usize,
    /// Number of trailing lines displayed in a generic head/tail fold.
    pub tail_lines: usize,
}

impl Default for FoldConfig {
    fn default() -> Self {
        Self {
            inline_threshold: 1_200,
            head_lines: 20,
            tail_lines: 20,
        }
    }
}

/// Request to fold one exact tool result.
#[derive(Debug, Clone, Copy)]
pub struct FoldRequest<'a> {
    /// Source class.
    pub tool: ToolKind,
    /// Exact original bytes. These bytes are stored unchanged whenever a fold occurs.
    pub output: &'a [u8],
    /// File path and turn for unchanged-read detection.
    pub file_read: Option<&'a FileRead>,
}

/// Display representation plus a validated CAS reference retaining exact original bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folded {
    /// Model-visible inline or folded view.
    pub view: String,
    /// Short CAS reference when content was stored; collisions fail rather than emitting an
    /// ambiguous marker.
    pub reference: Option<ShortRef>,
    /// Full content id for callers that need stable provenance outside the short marker.
    pub content_id: Option<Cid>,
    /// Original byte count.
    pub original_bytes: u64,
    /// Folding strategy used.
    pub kind: FoldKind,
}

/// Folding strategy, reported explicitly with each result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldKind {
    /// Small valid UTF-8 output kept inline. Tracked file reads may also carry a CAS id so a
    /// subsequent unchanged read can be folded without losing byte-exact expansion.
    Inline,
    /// RTK-rendered compiler diagnostics with the raw stream retained in CAS.
    Diagnostics,
    /// Deterministic first/last line view with the raw stream retained in CAS.
    HeadTail { head: u32, tail: u32 },
    /// A file body retained by its already-created CAS reference.
    FileRef,
    /// Same file bytes as the previous tracked read, with the original turn shown.
    UnchangedFile { since_turn: u32 },
}

/// Exact byte range requested from expanded CAS content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Inclusive starting byte offset.
    pub start: u64,
    /// Exclusive ending byte offset.
    pub end: u64,
}

/// Internal fold state. The CAS is synchronous; no async wrapper is implied.
#[derive(Debug, Default)]
pub(crate) struct FoldState {
    previous_files: Mutex<HashMap<String, (Cid, u32)>>,
}

impl FoldState {
    pub(crate) fn fold(
        &self,
        cas: &Cas,
        config: FoldConfig,
        request: FoldRequest<'_>,
    ) -> Result<Folded, ContextError> {
        let original_bytes =
            u64::try_from(request.output.len()).map_err(|_| ContextError::EstimateOverflow)?;
        let tracked_file = if request.tool == ToolKind::File {
            if let Some(read) = request.file_read {
                let cid = cas.put(request.output)?;
                let reference = cas.short_ref_checked(&cid)?;
                let path = normalize_path(&read.path);
                let prior = {
                    let mut entries = self
                        .previous_files
                        .lock()
                        .map_err(|_| ContextError::StatePoisoned)?;
                    let prior = entries.get(&path).copied();
                    entries.insert(path.clone(), (cid, read.turn));
                    prior
                };
                if let Some((previous, since_turn)) = prior
                    && previous == cid
                {
                    return Ok(Folded {
                        view: format!(
                            "[FILE: {path} | {} | unchanged since turn {since_turn}]",
                            reference.to_marker()
                        ),
                        reference: Some(reference),
                        content_id: Some(cid),
                        original_bytes,
                        kind: FoldKind::UnchangedFile { since_turn },
                    });
                }
                Some((path, cid, reference))
            } else {
                None
            }
        } else {
            None
        };
        if request.output.len() <= config.inline_threshold
            && let Ok(text) = std::str::from_utf8(request.output)
        {
            let (content_id, reference) = match tracked_file.as_ref() {
                Some((_, cid, short)) => (Some(*cid), Some(*short)),
                None => (None, None),
            };
            return Ok(Folded {
                view: text.to_owned(),
                reference,
                content_id,
                original_bytes,
                kind: FoldKind::Inline,
            });
        }

        let (cid, reference) = match tracked_file.as_ref() {
            Some((_, cid, reference)) => (*cid, *reference),
            None => {
                let cid = cas.put(request.output)?;
                (cid, cas.short_ref_checked(&cid)?)
            }
        };

        if request.tool == ToolKind::Compiler {
            let format = detect_format(request.output);
            let diagnostics = parse_diagnostics(request.output, format)?;
            let mut view = String::new();
            for diagnostic in diagnostics {
                view.push_str(&diagnostic.render());
            }
            if !view.is_empty() {
                view.push_str("\n[full compiler output: ");
                view.push_str(&reference.to_marker());
                view.push_str("]\n");
                return Ok(Folded {
                    view,
                    reference: Some(reference),
                    content_id: Some(cid),
                    original_bytes,
                    kind: FoldKind::Diagnostics,
                });
            }
        }

        let text = String::from_utf8_lossy(request.output);
        let (view, head, tail) = head_tail(&text, config.head_lines, config.tail_lines);
        let mut with_marker = view;
        with_marker.push_str("\n[full output: ");
        with_marker.push_str(&reference.to_marker());
        with_marker.push_str("]\n");
        if let Some((path, _, _)) = tracked_file.as_ref() {
            return Ok(Folded {
                view: format!("[FILE: {path}]\n{with_marker}"),
                reference: Some(reference),
                content_id: Some(cid),
                original_bytes,
                kind: FoldKind::FileRef,
            });
        }
        Ok(Folded {
            view: with_marker,
            reference: Some(reference),
            content_id: Some(cid),
            original_bytes,
            kind: FoldKind::HeadTail { head, tail },
        })
    }
}

/// Resolve a CAS short reference and optionally return an exact byte range.
pub(crate) fn expand(
    cas: &Cas,
    reference: ShortRef,
    range: Option<ByteRange>,
) -> Result<Bytes, ContextError> {
    let cid = cas.resolve(&reference)?;
    let bytes = cas.get(&cid)?;
    let Some(range) = range else {
        return Ok(bytes);
    };
    if range.start > range.end
        || range.end > u64::try_from(bytes.len()).map_err(|_| ContextError::InvalidRange)?
    {
        return Err(ContextError::InvalidRange);
    }
    let start = usize::try_from(range.start).map_err(|_| ContextError::InvalidRange)?;
    let length = usize::try_from(
        range
            .end
            .checked_sub(range.start)
            .ok_or(ContextError::InvalidRange)?,
    )
    .map_err(|_| ContextError::InvalidRange)?;
    let selected: Vec<u8> = bytes.iter().skip(start).take(length).copied().collect();
    Ok(Bytes::from(selected))
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn head_tail(text: &str, head_count: usize, tail_count: usize) -> (String, u32, u32) {
    let lines: Vec<&str> = text.lines().collect();
    let head_len = lines.len().min(head_count);
    let tail_len = lines.len().saturating_sub(head_len).min(tail_count);
    let mut out = String::new();
    for line in lines.iter().take(head_len) {
        out.push_str(line);
        out.push('\n');
    }
    let omitted = lines
        .len()
        .saturating_sub(head_len)
        .saturating_sub(tail_len);
    if omitted > 0 {
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "[… {omitted} lines omitted; full output is retained in CAS …]"
        );
    }
    for line in lines.iter().skip(lines.len().saturating_sub(tail_len)) {
        out.push_str(line);
        out.push('\n');
    }
    let head = match u32::try_from(head_len) {
        Ok(value) => value,
        Err(_) => maximum_line_count(),
    };
    let tail = match u32::try_from(tail_len) {
        Ok(value) => value,
        Err(_) => maximum_line_count(),
    };
    (out, head, tail)
}

const fn maximum_line_count() -> u32 {
    u32::MAX
}
