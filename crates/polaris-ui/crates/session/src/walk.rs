//! Bounded workspace walk shared by the local analysis commands (`/nv-verify`, `/hack`): sorted
//! source paths whose names a filter accepts, skipping vendored, build and tool directories.

use std::path::{Path, PathBuf};

use crate::SessionHostError;

/// Directory entries looked at before the walk stops.
const MAX_ENTRIES: usize = 20_000;
/// Directory depth descended to.
const MAX_DEPTH: usize = 12;
/// Directories that never hold the project's own sources.
const SKIPPED: &[&str] = &[
    ".git",
    ".polaris",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "build",
    "dist",
    "third_party",
    "external",
];

/// Sorted paths of files whose name `accept` takes, at most `limit`; `true` when a bound stopped
/// the walk early.
/// # Errors
/// The workspace cannot be listed.
pub(crate) fn source_files(
    workspace: &Path,
    accept: impl Fn(&str) -> bool,
    limit: usize,
) -> Result<(Vec<PathBuf>, bool), SessionHostError> {
    let mut sources = Vec::new();
    let mut truncated = false;
    let mut stack = vec![(workspace.to_owned(), 0_usize)];
    let mut seen = 0_usize;
    'walk: while let Some((directory, depth)) = stack.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|_| SessionHostError::Unavailable("workspace is unavailable"))?;
        for entry in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > MAX_ENTRIES || sources.len() >= limit {
                truncated = true;
                break 'walk;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIPPED.contains(&name.as_str()) {
                    stack.push((entry.path(), depth.saturating_add(1)));
                }
            } else if kind.is_file() && accept(&name) {
                sources.push(entry.path());
            }
        }
    }
    sources.sort();
    Ok((sources, truncated))
}

/// `path` relative to `workspace`, with `/` separators.
pub(crate) fn relative(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One-based line of byte `offset` in `source`.
pub(crate) fn line_of(source: &[u8], offset: usize) -> usize {
    source
        .iter()
        .take(offset)
        .filter(|byte| **byte == b'\n')
        .count()
        .saturating_add(1)
}
