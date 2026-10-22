//! Remote manifest acquisition through the narrow `SandboxBackend` contract.

use std::str;

use futures::StreamExt;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxHandle};

use crate::{EntryKind, FileEntry, Manifest, RelPath, SyncError, detect_conflicts};

/// A bounded adapter that reads a sandbox tree into a content manifest.
#[derive(Debug, Clone, Copy)]
pub struct SandboxSyncBackend<'a, B> {
    backend: &'a B,
    sandbox: &'a SandboxHandle,
    remote_root: &'a str,
    concurrency: usize,
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
        concurrency: usize,
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
            concurrency: concurrency.max(1),
        })
    }

    /// Lists and hashes the current sandbox tree. Paths outside `remote_root`, unsafe relative
    /// paths, and entries that cannot be read fail the snapshot rather than being omitted.
    ///
    /// # Errors
    /// [`SyncError::Backend`] on backend failure, [`SyncError::InvalidPath`] on an unsafe listed
    /// path, or [`SyncError::Rule`] if case or Windows-name conflicts are present.
    pub async fn snapshot(&self) -> Result<Manifest, SyncError> {
        let paths = self
            .backend
            .list_files(self.sandbox, self.remote_root)
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        let mut normalized = Vec::new();
        for path in paths {
            let relative = strip_root(self.remote_root, &path).ok_or_else(|| {
                SyncError::InvalidPath {
                    path: path.clone(),
                    reason: "backend listed a path outside the requested remote root",
                }
            })?;
            let rel = RelPath::new(relative)?;
            normalized.push((rel, path));
        }
        normalized.sort_by(|left, right| left.0.cmp(&right.0));
        let entries: Vec<Result<(RelPath, FileEntry), SyncError>> = futures::stream::iter(normalized)
            .map(|(rel, path)| async move {
                let entry = self.read_entry(&rel, &path).await?;
                Ok((rel, entry))
            })
            .buffer_unordered(self.concurrency)
            .collect()
            .await;
        let mut manifest = Manifest::default();
        for entry in entries {
            let (path, entry) = entry?;
            manifest.entries.insert(path, entry);
        }
        if !detect_conflicts(&manifest).is_empty() {
            return Err(SyncError::Rule("remote tree contains Windows path conflicts".into()));
        }
        Ok(manifest)
    }

    async fn read_entry(&self, rel: &RelPath, path: &str) -> Result<FileEntry, SyncError> {
        let link = self
            .backend
            .exec(self.sandbox, &request(vec!["test".into(), "-L".into(), path.into()]))
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        if link.exit_code == 0 {
            let output = self
                .backend
                .exec(
                    self.sandbox,
                    &request(vec!["readlink".into(), "--".into(), path.into()]),
                )
                .await
                .map_err(|error| SyncError::Backend(error.to_string()))?;
            if output.exit_code != 0 {
                return Err(SyncError::Backend(format!("readlink failed for {rel}")));
            }
            let target = str::from_utf8(&output.stdout)
                .map_err(|error| SyncError::Backend(error.to_string()))?
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            if !safe_link_target(rel, &target) {
                return Err(SyncError::RootEscape(rel.to_string()));
            }
            let size = u64::try_from(target.len()).map_err(|_| SyncError::Rule("link target too large".into()))?;
            return Ok(FileEntry {
                size,
                hash: Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes()),
                executable: false,
                kind: EntryKind::Link { target },
            });
        }
        let bytes = self
            .backend
            .download(self.sandbox, path)
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        let size = u64::try_from(bytes.len()).map_err(|_| SyncError::Rule("file too large".into()))?;
        let executable = self
            .backend
            .exec(self.sandbox, &request(vec!["test".into(), "-x".into(), "--".into(), path.into()]))
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?
            .exit_code
            == 0;
        Ok(FileEntry {
            size,
            hash: Cid::from_bytes(*blake3::hash(&bytes).as_bytes()),
            executable,
            kind: EntryKind::File,
        })
    }
}

fn request(argv: Vec<String>) -> ExecRequest {
    ExecRequest {
        argv,
        ..ExecRequest::default()
    }
}

fn valid_root(root: &str) -> bool {
    root.starts_with('/')
        && !root.starts_with("//")
        && !root.contains(['\\', '\0'])
        && root.split('/').skip(1).all(|part| part != "." && part != "..")
}

fn strip_root<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    let prefix = root.trim_end_matches('/');
    let relative = path.strip_prefix(prefix)?.strip_prefix('/')?;
    (!relative.is_empty()).then_some(relative)
}

fn safe_link_target(link: &RelPath, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains(['\\', '\0']) {
        return false;
    }
    let mut depth = link.parent().map_or(0usize, |parent| parent.split('/').count());
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(value) => depth = value,
                None => return false,
            },
            _ => depth = match depth.checked_add(1) {
                Some(value) => value,
                None => return false,
            },
        }
    }
    depth > 0
}

