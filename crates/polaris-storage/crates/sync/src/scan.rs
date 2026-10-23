//! Manifest scan: walk the workspace (never following links), apply the rules, hash with blake3.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use polaris_sdk::ids::Cid;
use walkdir::WalkDir;

use crate::ignore::{Exclusion, IgnoreRules};
use crate::path::{RelPath, SyncError};

/// What an entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A link inside the workspace, recorded (never followed); target uses `/` separators.
    Link {
        /// Relative link target.
        target: String,
    },
}

/// One manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Size in bytes (link: target length).
    pub size: u64,
    /// blake3 of the bytes (link: of the target text).
    pub hash: Cid,
    /// Executable in the sandbox. Windows has no exec bit, so a file starting with `#!` counts.
    pub executable: bool,
    /// File or link.
    pub kind: EntryKind,
}

/// Every synced path, in path order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The entries.
    pub entries: BTreeMap<RelPath, FileEntry>,
}

impl Manifest {
    /// A deterministic BLAKE3 digest binding every path, size, content hash, mode, and link target.
    #[must_use]
    pub fn content_id(&self) -> Cid {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"polaris-sync-manifest-v1\0");
        for (path, entry) in &self.entries {
            let path_bytes = path.as_str().as_bytes();
            let path_len = manifest_length(path_bytes.len());
            hasher.update(&path_len.to_be_bytes());
            hasher.update(path_bytes);
            hasher.update(&entry.size.to_be_bytes());
            hasher.update(entry.hash.as_bytes());
            hasher.update(&[u8::from(entry.executable)]);
            match &entry.kind {
                EntryKind::File => hasher.update(&[0]),
                EntryKind::Link { target } => {
                    hasher.update(&[1]);
                    let target_bytes = target.as_bytes();
                    let target_len = manifest_length(target_bytes.len());
                    hasher.update(&target_len.to_be_bytes());
                    hasher.update(target_bytes)
                }
            };
        }
        Cid::from_bytes(*hasher.finalize().as_bytes())
    }
}

// Keep the manifest's fixed-width length encoding deterministic even on a wider host.
fn manifest_length(length: usize) -> u64 {
    let Ok(length) = u64::try_from(length) else {
        return u64::MAX;
    };
    length
}

/// How unchanged files are detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashMode {
    /// Reuse the cached hash when size and mtime are unchanged (misses same-size edits that
    /// restore the mtime, and coarse-mtime filesystems).
    Fast,
    /// Rehash every file.
    Paranoid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedFile {
    size: u64,
    mtime_ns: u128,
    hash: Cid,
    executable: bool,
}

/// `(size, mtime) -> hash` from the previous scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanCache {
    files: BTreeMap<RelPath, CachedFile>,
}

/// Hashing work done by one scan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Files hashed.
    pub hashed: usize,
    /// Files whose cached hash was reused.
    pub reused: usize,
}

/// Hard bounds for one workspace scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// Maximum filesystem entries visited, including directories and denied paths.
    pub max_walk_entries: u64,
    /// Maximum files and links in the manifest.
    pub max_manifest_entries: u64,
    /// Maximum UTF-8 bytes in one workspace-relative path.
    pub max_path_bytes: u64,
    /// Maximum bytes in one regular file.
    pub max_file_bytes: u64,
    /// Maximum aggregate bytes hashed or reused from the cache.
    pub max_total_bytes: u64,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_walk_entries: 1_000_000,
            max_manifest_entries: 100_000,
            max_path_bytes: 4096,
            max_file_bytes: 256 * 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}

/// Result of [`scan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    /// What to sync.
    pub manifest: Manifest,
    /// Cache for the next scan.
    pub cache: ScanCache,
    /// Paths withheld by the deny-list (directories are reported once).
    pub denied: Vec<RelPath>,
    /// Links whose target leaves the workspace (never followed, never synced).
    pub escaped: Vec<RelPath>,
    /// Entries whose names are not valid UTF-8 / not representable (skipped).
    pub unrepresentable: Vec<PathBuf>,
    /// Hashing statistics.
    pub stats: ScanStats,
}

fn rel_of(root: &Path, path: &Path) -> Option<RelPath> {
    let mut parts = Vec::new();
    for component in path.strip_prefix(root).ok()?.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str()?),
            _ => return None,
        }
    }
    RelPath::new(&parts.join("/")).ok()
}

fn io_err(rel: &RelPath, e: &std::io::Error) -> SyncError {
    SyncError::Io {
        path: rel.as_str().to_owned(),
        reason: e.to_string(),
    }
}

fn modified_ns(metadata: &std::fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
}

#[cfg(unix)]
fn metadata_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn metadata_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Streams a file through blake3; also reports whether it starts with `#!`.
pub(crate) fn hash_file(
    path: &Path,
    rel: &RelPath,
    max_bytes: u64,
) -> Result<(Cid, u64, bool), SyncError> {
    let mut file = File::open(path).map_err(|e| io_err(rel, &e))?;
    let mut buffer = [0u8; 64 * 1024];
    let mut head = Vec::with_capacity(2);
    let mut size = 0_u64;
    let mut hasher = blake3::Hasher::new();
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let count = u64::try_from(count).map_err(|_| SyncError::Limit {
                    what: "file bytes",
                    limit: max_bytes,
                })?;
                size = size.checked_add(count).ok_or(SyncError::Limit {
                    what: "file bytes",
                    limit: max_bytes,
                })?;
                if size > max_bytes {
                    return Err(SyncError::Limit {
                        what: "file bytes",
                        limit: max_bytes,
                    });
                }
                if head.len() < 2 {
                    let read_count = match usize::try_from(count) {
                        Ok(count) => count,
                        Err(_) => {
                            return Err(SyncError::Limit {
                                what: "file bytes",
                                limit: max_bytes,
                            });
                        }
                    };
                    let needed = 2_usize.saturating_sub(head.len()).min(read_count);
                    if let Some(prefix) = buffer.get(..needed) {
                        head.extend_from_slice(prefix);
                    }
                }
                let read = match usize::try_from(count) {
                    Ok(count) => count,
                    Err(_) => {
                        return Err(SyncError::Limit {
                            what: "file bytes",
                            limit: max_bytes,
                        });
                    }
                };
                let bytes = buffer.get(..read).ok_or(SyncError::Limit {
                    what: "file bytes",
                    limit: max_bytes,
                })?;
                hasher.update(bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(rel, &e)),
        }
    }
    Ok((
        Cid::from_bytes(*hasher.finalize().as_bytes()),
        size,
        executable_file(&file, &head)?,
    ))
}

#[cfg(unix)]
fn executable_file(file: &File, _head: &[u8]) -> Result<bool, SyncError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = file
        .metadata()
        .map_err(|error| SyncError::Walk(error.to_string()))?
        .permissions()
        .mode();
    Ok(mode & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable_file(_file: &File, head: &[u8]) -> Result<bool, SyncError> {
    Ok(head == b"#!")
}

/// The link target relative to the workspace, or `None` if it is absolute or leaves the root.
fn link_target(rel: &RelPath, target: &Path) -> Option<String> {
    let mut depth: Vec<&str> = match rel.parent() {
        Some(parent) => parent.split('/').collect(),
        None => Vec::new(),
    };
    let mut parts = Vec::new();
    for component in target.components() {
        match component {
            Component::Normal(name) => {
                let name = name.to_str()?;
                parts.push(name);
                depth.push(name);
            }
            Component::ParentDir => {
                depth.pop()?;
                parts.push("..");
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if parts.iter().any(|part| part.contains('\\')) {
        return None;
    }
    if let Some(first) = parts.first() {
        let mut chars = first.chars();
        if chars
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
            && chars.next() == Some(':')
        {
            return None;
        }
    }
    Some(parts.join("/"))
}

/// Scans `root` into a manifest. Links are recorded, never followed; links leaving the workspace
/// are reported in [`ScanOutcome::escaped`].
///
/// # Errors
/// [`SyncError::Walk`] when the root cannot be walked, [`SyncError::Io`] when a file cannot be
/// read.
pub fn scan(
    root: &Path,
    rules: &IgnoreRules,
    cache: Option<&ScanCache>,
    mode: HashMode,
) -> Result<ScanOutcome, SyncError> {
    scan_with_limits(root, rules, cache, mode, ScanLimits::default())
}

/// Scans a workspace under explicit entry and byte limits.
pub fn scan_with_limits(
    root: &Path,
    rules: &IgnoreRules,
    cache: Option<&ScanCache>,
    mode: HashMode,
    limits: ScanLimits,
) -> Result<ScanOutcome, SyncError> {
    let mut out = ScanOutcome::default();
    let mut walked = 0_u64;
    let mut total_bytes = 0_u64;
    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|e| SyncError::Walk(e.to_string()))?;
        walked = walked.checked_add(1).ok_or(SyncError::Limit {
            what: "walk entries",
            limit: limits.max_walk_entries,
        })?;
        if walked > limits.max_walk_entries {
            return Err(SyncError::Limit {
                what: "walk entries",
                limit: limits.max_walk_entries,
            });
        }
        let Some(rel) = rel_of(root, entry.path()) else {
            out.unrepresentable.push(entry.path().to_path_buf());
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        };
        let file_type = entry.file_type();
        let is_dir = file_type.is_dir();
        match rules.classify(&rel, is_dir) {
            Exclusion::Denied => {
                out.denied.push(rel);
                if is_dir {
                    walker.skip_current_dir();
                }
                continue;
            }
            Exclusion::Ignored => {
                if is_dir {
                    walker.skip_current_dir();
                }
                continue;
            }
            Exclusion::Included if is_dir => continue,
            Exclusion::Included => {}
        }
        if u64::try_from(rel.as_str().len()).map_or(true, |length| length > limits.max_path_bytes) {
            return Err(SyncError::Limit {
                what: "path bytes",
                limit: limits.max_path_bytes,
            });
        }
        if file_type.is_symlink() {
            let target = std::fs::read_link(entry.path()).map_err(|e| io_err(&rel, &e))?;
            match link_target(&rel, &target) {
                Some(target) => {
                    let size = u64::try_from(target.len()).map_err(|_| SyncError::Limit {
                        what: "link bytes",
                        limit: limits.max_file_bytes,
                    })?;
                    total_bytes = total_bytes.checked_add(size).ok_or(SyncError::Limit {
                        what: "total bytes",
                        limit: limits.max_total_bytes,
                    })?;
                    if size > limits.max_file_bytes || total_bytes > limits.max_total_bytes {
                        return Err(SyncError::Limit {
                            what: "link or total bytes",
                            limit: limits.max_total_bytes.min(limits.max_file_bytes),
                        });
                    }
                    check_entry_limit(&out, limits)?;
                    let entry = FileEntry {
                        size,
                        hash: Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes()),
                        executable: false,
                        kind: EntryKind::Link { target },
                    };
                    out.manifest.entries.insert(rel, entry);
                }
                None => out.escaped.push(rel),
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| SyncError::Walk(e.to_string()))?;
        if meta.len() > limits.max_file_bytes {
            return Err(SyncError::Limit {
                what: "file bytes",
                limit: limits.max_file_bytes,
            });
        }
        total_bytes = total_bytes
            .checked_add(meta.len())
            .ok_or(SyncError::Limit {
                what: "total bytes",
                limit: limits.max_total_bytes,
            })?;
        if total_bytes > limits.max_total_bytes {
            return Err(SyncError::Limit {
                what: "total bytes",
                limit: limits.max_total_bytes,
            });
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        let cached = cache.and_then(|c| c.files.get(&rel)).filter(|c| {
            mode == HashMode::Fast
                && c.size == meta.len()
                && Some(c.mtime_ns) == mtime_ns
                && (cfg!(not(unix)) || c.executable == metadata_executable(&meta))
        });
        let (hash, size, executable) = match cached {
            Some(c) => {
                out.stats.reused += 1;
                (c.hash, c.size, c.executable)
            }
            None => {
                out.stats.hashed += 1;
                hash_file(entry.path(), &rel, limits.max_file_bytes)?
            }
        };
        let after = std::fs::metadata(entry.path()).map_err(|error| io_err(&rel, &error))?;
        if after.len() != size
            || modified_ns(&after) != modified_ns(&meta)
            || (cfg!(unix) && metadata_executable(&after) != executable)
        {
            return Err(SyncError::Walk(format!(
                "file changed while scanning: {rel}"
            )));
        }
        total_bytes = total_bytes
            .checked_sub(meta.len())
            .and_then(|bytes| bytes.checked_add(size))
            .ok_or(SyncError::Limit {
                what: "total bytes",
                limit: limits.max_total_bytes,
            })?;
        if total_bytes > limits.max_total_bytes {
            return Err(SyncError::Limit {
                what: "total bytes",
                limit: limits.max_total_bytes,
            });
        }
        if let Some(mtime_ns) = mtime_ns {
            out.cache.files.insert(
                rel.clone(),
                CachedFile {
                    size,
                    mtime_ns,
                    hash,
                    executable,
                },
            );
        }
        check_entry_limit(&out, limits)?;
        out.manifest.entries.insert(
            rel,
            FileEntry {
                size,
                hash,
                executable,
                kind: EntryKind::File,
            },
        );
    }
    out.denied.sort();
    out.escaped.sort();
    out.unrepresentable.sort();
    Ok(out)
}

fn check_entry_limit(out: &ScanOutcome, limits: ScanLimits) -> Result<(), SyncError> {
    let count = u64::try_from(out.manifest.entries.len()).map_err(|_| SyncError::Limit {
        what: "manifest entries",
        limit: limits.max_manifest_entries,
    })?;
    if count >= limits.max_manifest_entries {
        return Err(SyncError::Limit {
            what: "manifest entries",
            limit: limits.max_manifest_entries,
        });
    }
    Ok(())
}
