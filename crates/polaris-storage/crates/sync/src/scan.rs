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

/// Streams a file through blake3; also reports whether it starts with `#!`.
pub(crate) fn hash_file(path: &Path, rel: &RelPath) -> Result<(Cid, u64, bool), SyncError> {
    let mut file = File::open(path).map_err(|e| io_err(rel, &e))?;
    let mut head = [0u8; 2];
    let mut got = 0;
    while got < head.len() {
        let slot = head.get_mut(got..).unwrap_or_default();
        match file.read(slot) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(rel, &e)),
        }
    }
    let head = head.get(..got).unwrap_or_default();
    let mut hasher = blake3::Hasher::new();
    hasher.update(head);
    hasher
        .update_reader(&mut file)
        .map_err(|e| io_err(rel, &e))?;
    let size = file.metadata().map_err(|e| io_err(rel, &e))?.len();
    Ok((
        Cid::from_bytes(*hasher.finalize().as_bytes()),
        size,
        head == b"#!",
    ))
}

/// The link target relative to the workspace, or `None` if it is absolute or leaves the root.
fn link_target(rel: &RelPath, target: &Path) -> Option<String> {
    let mut depth: Vec<&str> = rel
        .parent()
        .map(|p| p.split('/').collect())
        .unwrap_or_default();
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
    let mut out = ScanOutcome::default();
    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1)
        .into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|e| SyncError::Walk(e.to_string()))?;
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
        if file_type.is_symlink() {
            let target = std::fs::read_link(entry.path()).map_err(|e| io_err(&rel, &e))?;
            match link_target(&rel, &target) {
                Some(target) => {
                    let entry = FileEntry {
                        size: u64::try_from(target.len()).unwrap_or(u64::MAX),
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
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos());
        let cached = cache.and_then(|c| c.files.get(&rel)).filter(|c| {
            mode == HashMode::Fast && c.size == meta.len() && Some(c.mtime_ns) == mtime_ns
        });
        let (hash, size, executable) = match cached {
            Some(c) => {
                out.stats.reused += 1;
                (c.hash, c.size, c.executable)
            }
            None => {
                out.stats.hashed += 1;
                hash_file(entry.path(), &rel)?
            }
        };
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
    Ok(out)
}
