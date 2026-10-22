//! Remote manifest acquisition through the narrow `SandboxBackend` contract.

use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{SandboxBackend, SandboxHandle, SandboxSnapshotPage};

use crate::{EntryKind, FileEntry, Manifest, RelPath, SyncError, detect_conflicts};

const MAX_REMOTE_ENTRIES: u64 = 100_000;
const MAX_REMOTE_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REMOTE_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const REMOTE_PAGE_ENTRIES: u32 = 4096;

/// A bounded adapter that reads a sandbox tree into a content manifest.
#[derive(Debug, Clone, Copy)]
pub struct SandboxSyncBackend<'a, B> {
    backend: &'a B,
    sandbox: &'a SandboxHandle,
    remote_root: &'a str,
}

impl<'a, B: SandboxBackend> SandboxSyncBackend<'a, B> {
    /// Creates a snapshot adapter. `remote_root` must be an absolute sandbox path.
    ///
    /// # Errors
    /// [`SyncError::InvalidPath`] when `remote_root` contains invalid path components.
    pub fn new(
        backend: &'a B,
        sandbox: &'a SandboxHandle,
        remote_root: &'a str,
        _concurrency: usize,
    ) -> Result<Self, SyncError> {
        if !valid_root(remote_root) {
            return Err(SyncError::InvalidPath {
                path: remote_root.into(),
                reason: "remote root must be an absolute sandbox path without dot components",
            });
        }
        Ok(Self {
            backend,
            sandbox,
            remote_root,
        })
    }

    /// Lists and hashes the current sandbox tree. Paths outside `remote_root`, unsafe relative
    /// paths, and entries that cannot be read fail the snapshot rather than being omitted.
    ///
    /// # Errors
    /// [`SyncError::Backend`] on backend failure, [`SyncError::InvalidPath`] on an unsafe listed
    /// path, or [`SyncError::Rule`] if case or Windows-name conflicts are present.
    pub async fn snapshot(&self) -> Result<Manifest, SyncError> {
        let rules = crate::IgnoreRules::new(std::path::Path::new("/"), None)?;
        let mut manifest = Manifest::default();
        let mut generation: Option<String> = None;
        let mut expected_id: Option<Cid> = None;
        let mut expected_total: Option<u64> = None;
        let mut offset = 0_u64;
        let mut total_bytes = 0_u64;
        loop {
            let page = self
                .backend
                .snapshot_page(
                    self.sandbox,
                    self.remote_root,
                    generation.as_deref(),
                    offset,
                    REMOTE_PAGE_ENTRIES,
                )
                .await
                .map_err(|error| SyncError::Backend(error.to_string()))?;
            validate_page_header(
                &page,
                generation.as_deref(),
                expected_id,
                expected_total,
                offset,
            )?;
            if usize::try_from(REMOTE_PAGE_ENTRIES).map_or(true, |limit| page.entries.len() > limit)
            {
                return Err(SyncError::Limit {
                    what: "remote page entries",
                    limit: u64::from(REMOTE_PAGE_ENTRIES),
                });
            }
            generation.get_or_insert_with(|| page.generation.clone());
            expected_id.get_or_insert(page.manifest_id);
            expected_total.get_or_insert(page.total_entries);
            let page_count = u64::try_from(page.entries.len()).map_err(|_| SyncError::Limit {
                what: "remote page entries",
                limit: u64::from(REMOTE_PAGE_ENTRIES),
            })?;
            if page.total_entries > MAX_REMOTE_ENTRIES {
                return Err(SyncError::Limit {
                    what: "remote entries",
                    limit: MAX_REMOTE_ENTRIES,
                });
            }
            for remote in page.entries {
                if remote.relative_path.len() > 4096 {
                    return Err(SyncError::Limit {
                        what: "remote path bytes",
                        limit: 4096,
                    });
                }
                let rel = RelPath::new(&remote.relative_path)?;
                if rules.classify(&rel, false) == crate::Exclusion::Denied {
                    return Err(SyncError::Rule(
                        "remote snapshot disclosed a hard-denied path".into(),
                    ));
                }
                let kind = match remote.link_target {
                    Some(target) => {
                        if !safe_link_target(&rel, &target)
                            || remote.executable
                            || u64::try_from(target.len()).ok() != Some(remote.size)
                            || Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes())
                                != remote.hash
                        {
                            return Err(SyncError::RootEscape(rel.to_string()));
                        }
                        total_bytes =
                            total_bytes
                                .checked_add(remote.size)
                                .ok_or(SyncError::Limit {
                                    what: "remote total bytes",
                                    limit: MAX_REMOTE_TOTAL_BYTES,
                                })?;
                        EntryKind::Link { target }
                    }
                    None => {
                        if remote.size > MAX_REMOTE_FILE_BYTES {
                            return Err(SyncError::Limit {
                                what: "remote file bytes",
                                limit: MAX_REMOTE_FILE_BYTES,
                            });
                        }
                        total_bytes =
                            total_bytes
                                .checked_add(remote.size)
                                .ok_or(SyncError::Limit {
                                    what: "remote total bytes",
                                    limit: MAX_REMOTE_TOTAL_BYTES,
                                })?;
                        EntryKind::File
                    }
                };
                if manifest
                    .entries
                    .insert(
                        rel,
                        FileEntry {
                            size: remote.size,
                            hash: remote.hash,
                            executable: remote.executable,
                            kind,
                        },
                    )
                    .is_some()
                {
                    return Err(SyncError::Rule("remote snapshot repeated a path".into()));
                }
            }
            if total_bytes > MAX_REMOTE_TOTAL_BYTES {
                return Err(SyncError::Limit {
                    what: "remote total bytes",
                    limit: MAX_REMOTE_TOTAL_BYTES,
                });
            }
            let next = match page.next_offset {
                Some(next) => next,
                None => break,
            };
            let expected_next = offset.checked_add(page_count).ok_or(SyncError::Limit {
                what: "remote entries",
                limit: MAX_REMOTE_ENTRIES,
            })?;
            if next != expected_next || next <= offset {
                return Err(SyncError::Backend(
                    "remote snapshot pagination skipped or repeated entries".into(),
                ));
            }
            offset = next;
        }
        let Some(expected_id) = expected_id else {
            return Err(SyncError::Backend(
                "remote snapshot returned no pages".into(),
            ));
        };
        if u64::try_from(manifest.entries.len()).ok() != expected_total
            || manifest.content_id() != expected_id
        {
            return Err(SyncError::Backend(
                "remote snapshot count or content digest mismatch".into(),
            ));
        }
        if !detect_conflicts(&manifest).is_empty() {
            return Err(SyncError::Rule(
                "remote tree contains Windows path conflicts".into(),
            ));
        }
        Ok(manifest)
    }
}

fn validate_page_header(
    page: &SandboxSnapshotPage,
    generation: Option<&str>,
    manifest_id: Option<Cid>,
    total_entries: Option<u64>,
    offset: u64,
) -> Result<(), SyncError> {
    if page.generation.is_empty()
        || page.generation.len() > 128
        || page.offset != offset
        || generation.is_some_and(|value| value != page.generation)
        || manifest_id.is_some_and(|value| value != page.manifest_id)
        || total_entries.is_some_and(|value| value != page.total_entries)
    {
        return Err(SyncError::Backend(
            "remote snapshot generation or page header changed".into(),
        ));
    }
    Ok(())
}

fn valid_root(root: &str) -> bool {
    root.starts_with('/')
        && !root.starts_with("//")
        && !root.contains(['\\', '\0'])
        && root
            .split('/')
            .skip(1)
            .all(|part| part != "." && part != "..")
}

fn safe_link_target(link: &RelPath, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains(['\\', '\0']) {
        return false;
    }
    let mut depth = link
        .parent()
        .map_or(0usize, |parent| parent.split('/').count());
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(value) => depth = value,
                None => return false,
            },
            _ => {
                depth = match depth.checked_add(1) {
                    Some(value) => value,
                    None => return false,
                }
            }
        }
    }
    depth > 0
}
