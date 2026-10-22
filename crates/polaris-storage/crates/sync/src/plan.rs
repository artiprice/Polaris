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
    /// Remote state observed while planning, required for compare-and-swap writes/deletes.
    /// `None` means the path was absent. Apply re-reads every target and refuses divergence.
    pub expected_remote: BTreeMap<RelPath, Option<crate::scan::FileEntry>>,
    /// Paths changed on both sides relative to the common base; callers must resolve these.
    pub conflicts: Vec<SyncConflict>,
    /// Windows path-fidelity conflicts in the union of both manifests.
    pub path_conflicts: Vec<PathConflict>,
}

/// A two-way synchronization conflict relative to a previously shared manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncConflict {
    /// The conflicting workspace-relative path.
    pub path: RelPath,
    /// Base entry before either side changed.
    pub base: Option<crate::scan::FileEntry>,
    /// Current local entry.
    pub local: Option<crate::scan::FileEntry>,
    /// Current remote entry.
    pub remote: Option<crate::scan::FileEntry>,
}

impl SyncPlan {
    /// `true` when nothing needs to change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.upload.is_empty()
            && self.delete.is_empty()
            && self.conflicts.is_empty()
            && self.path_conflicts.is_empty()
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
    let mut expected_remote = BTreeMap::new();
    for path in &upload {
        expected_remote.insert(path.clone(), remote.entries.get(path).cloned());
    }
    for path in &delete {
        expected_remote.insert(path.clone(), remote.entries.get(path).cloned());
    }
    let mut union = local.clone();
    union.entries.extend(remote.entries.clone());
    let path_conflicts = detect_conflicts(&union);
    SyncPlan {
        upload,
        delete,
        rename_hints,
        expected_remote,
        conflicts: Vec::new(),
        path_conflicts,
    }
}

/// Plans a two-way sync against a common base. Concurrent edits and edit/delete races are
/// returned as conflicts and excluded from mutation lists.
#[must_use]
pub fn plan_two_way(base: &Manifest, local: &Manifest, remote: &Manifest) -> SyncPlan {
    let paths: BTreeSet<RelPath> = base
        .entries
        .keys()
        .chain(local.entries.keys())
        .chain(remote.entries.keys())
        .cloned()
        .collect();
    let mut result = SyncPlan::default();
    let mut union = local.clone();
    union.entries.extend(remote.entries.clone());
    result.path_conflicts = detect_conflicts(&union);
    for path in paths {
        let before = base.entries.get(&path);
        let left = local.entries.get(&path);
        let right = remote.entries.get(&path);
        let local_changed = left != before;
        let remote_changed = right != before;
        if local_changed && remote_changed && left != right {
            result.conflicts.push(SyncConflict {
                path,
                base: before.cloned(),
                local: left.cloned(),
                remote: right.cloned(),
            });
            continue;
        }
        if local_changed && left != right {
            match left {
                Some(_) => result.upload.push(path.clone()),
                None if right.is_some() => result.delete.push(path.clone()),
                None => {}
            }
            result.expected_remote.insert(path, right.cloned());
        }
    }
    result
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
    let stem = match name.split('.').next() {
        Some(stem) => stem.to_ascii_uppercase(),
        None => name.to_ascii_uppercase(),
    };
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
        if path.as_str().len() > 260 || path.components().any(|name| name.len() > 255) {
            out.push(PathConflict::TooLong(path.clone()));
        } else if let Some(conflict) = path
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
