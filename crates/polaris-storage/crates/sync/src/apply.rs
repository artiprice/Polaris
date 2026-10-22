//! Apply a plan through the sdk `SandboxBackend` transport.
//!
//! Backend contract: `upload` stages exact bytes in a prepared private namespace.
//! `conditional_commit` revalidates and installs file/link bytes and mode or deletes the
//! observed entry under the remote workspace lock. Unsupported backends fail closed.
//! Chunking below the gateway's 1 MiB request limit (spike S0-7) is the backend's job.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use futures::StreamExt;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{
    CommitOutcome, ExecRequest, SandboxBackend, SandboxCommitAction, SandboxCommitRequest,
    SandboxError, SandboxFileState, SandboxHandle,
};

use crate::path::{RelPath, SyncError};
use crate::plan::{PathConflict, SyncPlan};
use crate::scan::{EntryKind, Manifest};

const MAX_IN_FLIGHT_UPLOAD_BYTES: u64 = 128 * 1024 * 1024;

/// Transfer options.
#[derive(Debug, Clone)]
pub struct ApplyOpts {
    /// Absolute workspace directory inside the sandbox (e.g. `/sandbox/ws`).
    pub remote_root: String,
    /// Maximum concurrent uploads (0 behaves like 1).
    pub concurrency: usize,
    /// Optional cancellation flag. In-flight backend calls finish; no later mutation starts.
    pub cancellation: Option<Arc<AtomicBool>>,
}

/// Caller-provided secret-content check run on exact file bytes before remote staging.
pub type SecretPrecheck = dyn Fn(&RelPath, &[u8]) -> Result<bool, String> + Send + Sync;

/// What `apply` did. A path is in exactly one of the lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Exact file/link bytes and executable mode installed by the commit helper.
    pub uploaded: Vec<RelPath>,
    /// Deleted remotely.
    pub deleted: Vec<RelPath>,
    /// Changed on disk since the scan: skipped, rescan needed.
    pub changed: Vec<RelPath>,
    /// Failed with the backend's message; retried by the next plan.
    pub failed: Vec<(RelPath, String)>,
    /// Mutations refused because the remote path no longer matches the planning snapshot.
    pub conflicts: Vec<RelPath>,
    /// Paths blocked by the caller-provided secret-content precheck.
    pub secret_blocked: Vec<RelPath>,
    /// Windows path-fidelity conflicts that require caller resolution.
    pub path_conflicts: Vec<PathConflict>,
    /// Planned paths not started after cancellation was requested.
    pub cancelled: Vec<RelPath>,
}

impl SyncReport {
    /// The remote manifest after this apply (feed it to the next [`crate::plan`] to resume).
    #[must_use]
    pub fn applied_to(&self, remote: &Manifest, local: &Manifest) -> Manifest {
        let mut next = remote.clone();
        for path in &self.deleted {
            next.entries.remove(path);
        }
        for path in &self.uploaded {
            if let Some(entry) = local.entries.get(path) {
                next.entries.insert(path.clone(), entry.clone());
            }
        }
        next
    }
}

enum Outcome {
    Uploaded(RelPath),
    Conflict(RelPath),
    SecretBlocked(RelPath),
    Changed(RelPath),
    Cancelled(RelPath),
    Failed(RelPath, String),
}

fn remote_path(root: &str, rel: &RelPath) -> String {
    format!("{}/{}", root.trim_end_matches('/'), rel.as_str())
}

fn cancelled(opts: &ApplyOpts) -> bool {
    opts.cancellation
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Acquire))
}

async fn remote_matches<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    opts: &ApplyOpts,
    rel: &RelPath,
    expected: Option<&crate::scan::FileEntry>,
) -> Result<bool, String> {
    let path = remote_path(&opts.remote_root, rel);
    match expected {
        None => match backend.download(sandbox, &path).await {
            Ok(_) => Ok(false),
            Err(SandboxError::NotFound(_)) => Ok(true),
            Err(error) => Err(error.to_string()),
        },
        Some(entry) => match &entry.kind {
            EntryKind::File => {
                let bytes = match backend.download(sandbox, &path).await {
                    Ok(bytes) => bytes,
                    Err(SandboxError::NotFound(_)) => return Ok(false),
                    Err(error) => return Err(error.to_string()),
                };
                if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != entry.hash {
                    return Ok(false);
                }
                let output = backend
                    .exec(sandbox, &exec_req(vec!["test".into(), "-x".into(), path]))
                    .await
                    .map_err(|error| error.to_string())?;
                Ok((output.exit_code == 0) == entry.executable)
            }
            EntryKind::Link { target } => {
                let output = backend
                    .exec(
                        sandbox,
                        &exec_req(vec!["readlink".into(), "--".into(), path]),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let observed = std::str::from_utf8(&output.stdout)
                    .map_err(|error| error.to_string())?
                    .trim_end_matches(['\r', '\n']);
                Ok(output.exit_code == 0 && observed == target)
            }
        },
    }
}

fn exec_req(argv: Vec<String>) -> ExecRequest {
    ExecRequest {
        argv,
        workdir: None,
        stdin: None,
        env: Vec::new(),
        timeout: None,
    }
}

async fn upload_one<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    root: &Path,
    local: &Manifest,
    opts: &ApplyOpts,
    rel: &RelPath,
    expected: Option<&crate::scan::FileEntry>,
    secret_precheck: Option<&SecretPrecheck>,
) -> Outcome {
    if cancelled(opts) {
        return Outcome::Cancelled(rel.clone());
    }
    let Some(entry) = local.entries.get(rel) else {
        return Outcome::Failed(rel.clone(), "not in the local manifest".into());
    };
    match &entry.kind {
        EntryKind::Link { target: link } => {
            if cancelled(opts) {
                return Outcome::Cancelled(rel.clone());
            }
            let request = commit_request(
                opts,
                rel,
                expected,
                SandboxCommitAction::Link {
                    target: link.clone(),
                    hash: entry.hash,
                },
            );
            commit_outcome(backend.conditional_commit(sandbox, &request).await, rel)
        }
        EntryKind::File => {
            if entry.size > MAX_IN_FLIGHT_UPLOAD_BYTES {
                return Outcome::Failed(
                    rel.clone(),
                    "source file exceeds upload memory budget".into(),
                );
            }
            let local_path = rel.components().fold(root.to_path_buf(), |p, c| p.join(c));
            let bytes = match read_local_bounded(&local_path, entry.size) {
                Ok(bytes) => bytes,
                Err(_) => return Outcome::Changed(rel.clone()),
            };
            // Re-hash: the file may have changed between scan and upload (torn sync).
            if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != entry.hash {
                return Outcome::Changed(rel.clone());
            }
            if let Some(check) = secret_precheck {
                match check(rel, &bytes) {
                    Ok(true) => return Outcome::SecretBlocked(rel.clone()),
                    Ok(false) => {}
                    Err(reason) => return Outcome::Failed(rel.clone(), reason),
                }
            }
            if cancelled(opts) {
                return Outcome::Cancelled(rel.clone());
            }
            let stage = format!(
                "{}/.polaris/sync/staging/{}",
                opts.remote_root.trim_end_matches('/'),
                uuid::Uuid::now_v7()
            );
            let upload = backend.upload(sandbox, &stage, Bytes::from(bytes)).await;
            let result = match upload {
                Ok(()) if !cancelled(opts) => {
                    let request = commit_request(
                        opts,
                        rel,
                        expected,
                        SandboxCommitAction::File {
                            staged_path: stage.clone(),
                            hash: entry.hash,
                            executable: entry.executable,
                        },
                    );
                    commit_outcome(backend.conditional_commit(sandbox, &request).await, rel)
                }
                Ok(()) => Outcome::Cancelled(rel.clone()),
                Err(error) => Outcome::Failed(rel.clone(), error.to_string()),
            };
            // The helper consumes a committed stage. Failed/conflicting stages are still ours;
            // remove only the exact generated pathname. A failed cleanup is a visible failure.
            if !matches!(result, Outcome::Uploaded(..)) {
                let cleanup = backend
                    .exec(
                        sandbox,
                        &exec_req(vec!["rm".into(), "-f".into(), "--".into(), stage]),
                    )
                    .await;
                if !matches!(cleanup,Ok(ref output) if output.exit_code==0) {
                    return Outcome::Failed(rel.clone(), "staged upload cleanup failed".into());
                }
            }
            result
        }
    }
}

fn state(entry: &crate::scan::FileEntry) -> SandboxFileState {
    SandboxFileState {
        hash: entry.hash,
        executable: entry.executable,
        link_target: match &entry.kind {
            EntryKind::File => None,
            EntryKind::Link { target } => Some(target.clone()),
        },
    }
}
fn commit_request(
    opts: &ApplyOpts,
    rel: &RelPath,
    expected: Option<&crate::scan::FileEntry>,
    action: SandboxCommitAction,
) -> SandboxCommitRequest {
    SandboxCommitRequest {
        root: opts.remote_root.clone(),
        relative_path: rel.as_str().to_owned(),
        expected: expected.map(state),
        action,
    }
}
fn commit_outcome(result: Result<CommitOutcome, SandboxError>, rel: &RelPath) -> Outcome {
    match result {
        Ok(CommitOutcome::Applied) => Outcome::Uploaded(rel.clone()),
        Ok(CommitOutcome::Conflict) => Outcome::Conflict(rel.clone()),
        Err(error) => Outcome::Failed(rel.clone(), error.to_string()),
    }
}
fn read_local_bounded(path: &Path, size: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    if size > MAX_IN_FLIGHT_UPLOAD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "source file exceeds transfer bound",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(size.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).ok() != Some(size) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "source size changed",
        ));
    }
    Ok(bytes)
}

/// Applies `plan` using private staging, bounded upload memory and locked conditional commits
/// that install bytes and mode together. Per-path failures are reported, not fatal, so a
/// second `plan` against [`SyncReport::applied_to`] resumes.
///
/// # Errors
/// [`SyncError::InvalidPath`] when `opts.remote_root` is not an absolute sandbox path.
pub async fn apply<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    root: &Path,
    local: &Manifest,
    plan: &SyncPlan,
    opts: &ApplyOpts,
) -> Result<SyncReport, SyncError> {
    apply_with_precheck(backend, sandbox, root, local, plan, opts, None).await
}

/// Applies with a caller-provided secret-content precheck. A flagged file is withheld before
/// staging; an error fails that path closed. The check runs on exact bytes revalidated against
/// the manifest immediately before transfer.
pub async fn apply_with_precheck<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    root: &Path,
    local: &Manifest,
    plan: &SyncPlan,
    opts: &ApplyOpts,
    secret_precheck: Option<&SecretPrecheck>,
) -> Result<SyncReport, SyncError> {
    if !valid_remote_root(&opts.remote_root) {
        return Err(SyncError::InvalidPath {
            path: opts.remote_root.clone(),
            reason: "remote root must be an absolute sandbox path without '..'",
        });
    }
    if !plan.upload.is_empty() {
        backend
            .prepare_sync(sandbox, &opts.remote_root)
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
    }
    let mut report = SyncReport::default();
    report
        .conflicts
        .extend(plan.conflicts.iter().map(|conflict| conflict.path.clone()));
    report.path_conflicts = plan.path_conflicts.clone();
    let mut allowed_uploads = Vec::new();
    let mut allowed_deletes = Vec::new();
    for rel in plan.upload.iter().chain(plan.delete.iter()) {
        if report
            .path_conflicts
            .iter()
            .any(|conflict| conflict_paths(conflict).contains(rel))
        {
            report.conflicts.push(rel.clone());
            continue;
        }
        if cancelled(opts) {
            report.cancelled.push(rel.clone());
            continue;
        }
        let expected = plan.expected_remote.get(rel).and_then(Option::as_ref);
        let absent_was_expected = plan.expected_remote.contains_key(rel)
            && plan.expected_remote.get(rel).is_some_and(Option::is_none);
        if !plan.expected_remote.contains_key(rel) {
            report.failed.push((
                rel.clone(),
                "plan has no remote compare-and-swap baseline".into(),
            ));
            continue;
        }
        match remote_matches(backend, sandbox, opts, rel, expected).await {
            Ok(true) if absent_was_expected || expected.is_some() => {
                if plan.upload.contains(rel) {
                    allowed_uploads.push(rel.clone());
                } else {
                    allowed_deletes.push(rel.clone());
                }
            }
            Ok(false) => report.conflicts.push(rel.clone()),
            Ok(true) => report.conflicts.push(rel.clone()),
            Err(reason) => report.failed.push((rel.clone(), reason)),
        }
    }
    report.conflicts.sort();
    report.conflicts.dedup();
    let mut uncancelled_uploads = Vec::new();
    for rel in allowed_uploads {
        if cancelled(opts) {
            report.cancelled.push(rel);
        } else {
            uncancelled_uploads.push(rel);
        }
    }
    let largest = match uncancelled_uploads
        .iter()
        .filter_map(|path| local.entries.get(path))
        .map(|entry| entry.size)
        .max()
    {
        Some(size) => size.max(1),
        None => 1,
    };
    let slots = match MAX_IN_FLIGHT_UPLOAD_BYTES.checked_div(largest) {
        Some(slots) => slots.max(1),
        None => 1,
    };
    let memory_concurrency = usize::try_from(slots).map_err(|_| SyncError::Limit {
        what: "upload concurrency",
        limit: MAX_IN_FLIGHT_UPLOAD_BYTES,
    })?;
    let outcomes: Vec<Outcome> = futures::stream::iter(uncancelled_uploads)
        .map(move |rel| async move {
            upload_one(
                backend,
                sandbox,
                root,
                local,
                opts,
                &rel,
                plan.expected_remote.get(&rel).and_then(Option::as_ref),
                secret_precheck,
            )
            .await
        })
        .buffer_unordered(opts.concurrency.clamp(1, 64).min(memory_concurrency))
        .collect()
        .await;
    for outcome in outcomes {
        match outcome {
            Outcome::Uploaded(rel) => report.uploaded.push(rel),
            Outcome::Changed(rel) => report.changed.push(rel),
            Outcome::Conflict(rel) => report.conflicts.push(rel),
            Outcome::SecretBlocked(rel) => report.secret_blocked.push(rel),
            Outcome::Cancelled(rel) => report.cancelled.push(rel),
            Outcome::Failed(rel, reason) => report.failed.push((rel, reason)),
        }
    }
    for rel in allowed_deletes {
        if cancelled(opts) {
            report.cancelled.push(rel);
            continue;
        }
        let request = commit_request(
            opts,
            &rel,
            plan.expected_remote.get(&rel).and_then(Option::as_ref),
            SandboxCommitAction::Delete,
        );
        match backend.conditional_commit(sandbox, &request).await {
            Ok(CommitOutcome::Applied) => report.deleted.push(rel),
            Ok(CommitOutcome::Conflict) => report.conflicts.push(rel),
            Err(error) => report.failed.push((rel, error.to_string())),
        }
    }
    report.uploaded.sort();
    report.changed.sort();
    report.failed.sort();
    report.cancelled.sort();
    report.cancelled.dedup();
    report.secret_blocked.sort();
    Ok(report)
}

fn conflict_paths(conflict: &PathConflict) -> Vec<RelPath> {
    match conflict {
        PathConflict::CaseOnly { a, b } => vec![a.clone(), b.clone()],
        PathConflict::ReservedName(path)
        | PathConflict::TrailingDotOrSpace(path)
        | PathConflict::AlternateStream(path)
        | PathConflict::InvalidChar(path)
        | PathConflict::TooLong(path) => vec![path.clone()],
    }
}

fn valid_remote_root(root: &str) -> bool {
    if !root.starts_with('/') || root.starts_with("//") || root.contains(['\\', '\0']) {
        return false;
    }
    root.split('/')
        .skip(1)
        .all(|part| part != "." && part != "..")
}
