//! Pure diff of two manifests, and path-conflict detection.

use std::collections::{BTreeMap, BTreeSet};

use crate::path::RelPath;
use crate::scan::{EntryKind, Manifest};

/// What to transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// Paths to upload (new or different), in path order.
    pub upload: Vec<RelPath>,
    /// Paths to delete remotely, in path order.
    pub delete: Vec<RelPath>,
    /// `(old, new)` pairs where a deleted file reappears with the same content.
    pub rename_hints: Vec<(RelPath, RelPath)>,
}

impl SyncPlan {
    /// `true` when nothing needs to change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.upload.is_empty() && self.delete.is_empty()
    }
}

/// Plans the transfer that turns `remote` into `local` (the workspace is authoritative).
#[must_use]
pub fn plan(local: &Manifest, remote: &Manifest) -> SyncPlan {
    let upload: Vec<RelPath> = local
        .entries
        .iter()
        .filter(|(path, entry)| remote.entries.get(*path) != Some(*entry))
        .map(|(path, _)| path.clone())
        .collect();
    let delete: Vec<RelPath> = remote
        .entries
        .keys()
        .filter(|path| !local.entries.contains_key(*path))
        .cloned()
        .collect();
    let mut fresh: BTreeMap<_, Vec<&RelPath>> = BTreeMap::new();
    for path in upload.iter().filter(|p| !remote.entries.contains_key(*p)) {
        if let Some(entry) = local
            .entries
            .get(path)
            .filter(|e| e.kind == EntryKind::File)
        {
            fresh.entry(entry.hash).or_default().push(path);
        }
    }
    let mut rename_hints = Vec::new();
    for old in &delete {
        let Some(entry) = remote
            .entries
            .get(old)
            .filter(|e| e.kind == EntryKind::File)
        else {
            continue;
        };
        if let Some(new) = fresh.get_mut(&entry.hash).and_then(|candidates| {
            if candidates.is_empty() {
                None
            } else {
                Some(candidates.remove(0))
            }
        }) {
            rename_hints.push((old.clone(), new.clone()));
        }
    }
    SyncPlan {
        upload,
        delete,
        rename_hints,
    }
}

/// A path that cannot exist faithfully on one side of a Windows <-> Linux sync.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathConflict {
    /// Two paths (or directories) differ only by case.
    CaseOnly {
        /// The first (in byte order).
        a: RelPath,
        /// The second.
        b: RelPath,
    },
    /// A Windows device name (`CON`, `NUL`, `COM1`, … with any extension).
    ReservedName(RelPath),
    /// A component ending in `.` or space.
    TrailingDotOrSpace(RelPath),
    /// A `:` in a name (NTFS alternate data stream syntax).
    AlternateStream(RelPath),
    /// `<>"|?*` or a control character.
    InvalidChar(RelPath),
    /// A component longer than 255 bytes.
    TooLong(RelPath),
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn component_conflict(path: &RelPath, name: &str) -> Option<PathConflict> {
    let path = || path.clone();
    if name.len() > 255 {
        return Some(PathConflict::TooLong(path()));
    }
    if name.contains(':') {
        return Some(PathConflict::AlternateStream(path()));
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return Some(PathConflict::InvalidChar(path()));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Some(PathConflict::ReservedName(path()));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some(PathConflict::TrailingDotOrSpace(path()));
    }
    None
}

/// Every conflict in `manifest`: per-path conflicts in path order (first applicable kind), then
/// case-only pairs (files or directories) in order.
#[must_use]
pub fn detect_conflicts(manifest: &Manifest) -> Vec<PathConflict> {
    let mut out = Vec::new();
    let mut prefixes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in manifest.entries.keys() {
        if let Some(conflict) = path
            .components()
            .find_map(|name| component_conflict(path, name))
        {
            out.push(conflict);
        }
        let mut prefix = String::new();
        for name in path.components() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(name);
            prefixes
                .entry(prefix.to_lowercase())
                .or_default()
                .insert(prefix.clone());
        }
    }
    for variants in prefixes.values().filter(|v| v.len() > 1) {
        let mut iter = variants.iter().filter_map(|v| RelPath::new(v).ok());
        if let Some(first) = iter.next() {
            for other in iter {
                out.push(PathConflict::CaseOnly {
                    a: first.clone(),
                    b: other,
                });
            }
        }
    }
    out
}
