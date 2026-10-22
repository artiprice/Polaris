//! Apply a plan through the sdk `SandboxBackend` transport.
//!
//! Backend contract: `upload` writes exactly the given bytes (no EOL translation) and creates
//! parent directories. Deletes, links and the executable bit go through `exec` argv (no shell).
//! Chunking below the gateway's 1 MiB request limit (spike S0-7) is the backend's job.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle};

use crate::path::{RelPath, SyncError};
use crate::plan::{PathConflict, SyncPlan};
use crate::scan::{EntryKind, Manifest};

/// Paths per `rm` / `chmod` exec call.
const ARGS_PER_EXEC: usize = 256;

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

/// What `apply` did. A path is in exactly one of the lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Uploaded (and, if executable, chmod-ed).
    pub uploaded: Vec<RelPath>,
    /// Deleted remotely.
    pub deleted: Vec<RelPath>,
    /// Changed on disk since the scan: skipped, rescan needed.
    pub changed: Vec<RelPath>,
    /// Failed with the backend's message; retried by the next plan.
    pub failed: Vec<(RelPath, String)>,
    /// Mutations refused because the remote path no longer matches the planning snapshot.
    pub conflicts: Vec<RelPath>,
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
    Uploaded(RelPath, bool),
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
                    .exec(
                        sandbox,
                        &exec_req(vec!["test".into(), "-x".into(), "--".into(), path]),
                    )
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
) -> Outcome {
    if cancelled(opts) {
        return Outcome::Cancelled(rel.clone());
    }
    let Some(entry) = local.entries.get(rel) else {
        return Outcome::Failed(rel.clone(), "not in the local manifest".into());
    };
    let target = remote_path(&opts.remote_root, rel);
    match &entry.kind {
        EntryKind::Link { target: link } => {
            if cancelled(opts) {
                return Outcome::Cancelled(rel.clone());
            }
            let req = exec_req(vec![
                "ln".into(),
                "-sfn".into(),
                "--".into(),
                link.clone(),
                target,
            ]);
            match backend.exec(sandbox, &req).await {
                Ok(out) if out.exit_code == 0 => Outcome::Uploaded(rel.clone(), false),
                Ok(out) => Outcome::Failed(rel.clone(), format!("ln exited {}", out.exit_code)),
                Err(e) => Outcome::Failed(rel.clone(), e.to_string()),
            }
        }
        EntryKind::File => {
            let local_path = rel.components().fold(root.to_path_buf(), |p, c| p.join(c));
            let bytes = match std::fs::read(&local_path) {
                Ok(bytes) => bytes,
                Err(_) => return Outcome::Changed(rel.clone()),
            };
            // Re-hash: the file may have changed between scan and upload (torn sync).
            if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != entry.hash {
                return Outcome::Changed(rel.clone());
            }
            if cancelled(opts) {
                return Outcome::Cancelled(rel.clone());
            }
            match backend.upload(sandbox, &target, Bytes::from(bytes)).await {
                Ok(()) => Outcome::Uploaded(rel.clone(), entry.executable),
                Err(e) => Outcome::Failed(rel.clone(), e.to_string()),
            }
        }
    }
}

/// Runs `cmd args -- paths…` in chunks; returns the paths whose call failed, with the reason.
async fn exec_paths<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    root: &str,
    prefix: &[&str],
    paths: &[RelPath],
) -> Vec<(RelPath, String)> {
    let mut failed = Vec::new();
    for chunk in paths.chunks(ARGS_PER_EXEC) {
        let mut argv: Vec<String> = prefix.iter().map(|s| (*s).to_owned()).collect();
        argv.push("--".into());
        argv.extend(chunk.iter().map(|p| remote_path(root, p)));
        let reason = match backend.exec(sandbox, &exec_req(argv)).await {
            Ok(out) if out.exit_code == 0 => continue,
            Ok(out) => {
                let command = match prefix.first() {
                    Some(command) => *command,
                    None => "exec",
                };
                format!("{command} exited {}", out.exit_code)
            }
            Err(e) => e.to_string(),
        };
        failed.extend(chunk.iter().map(|p| (p.clone(), reason.clone())));
    }
    failed
}

/// Applies `plan` (computed from `local`) to the sandbox: bounded-concurrency uploads, then
/// `chmod +x` for executables, then deletes. Per-path failures are reported, not fatal, so a
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
    if !valid_remote_root(&opts.remote_root) {
        return Err(SyncError::InvalidPath {
            path: opts.remote_root.clone(),
            reason: "remote root must be an absolute sandbox path without '..'",
        });
    }
    let mut report = SyncReport::default();
    report.conflicts.extend(plan.conflicts.iter().map(|conflict| conflict.path.clone()));
    report.path_conflicts = plan.path_conflicts.clone();
    let mut allowed_uploads = Vec::new();
    let mut allowed_deletes = Vec::new();
    for rel in plan.upload.iter().chain(plan.delete.iter()) {
        if report.path_conflicts.iter().any(|conflict| conflict_paths(conflict).contains(rel)) {
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
            report.failed.push((rel.clone(), "plan has no remote compare-and-swap baseline".into()));
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
    let outcomes: Vec<Outcome> = futures::stream::iter(&uncancelled_uploads)
        .map(|rel| upload_one(backend, sandbox, root, local, opts, rel))
        .buffer_unordered(opts.concurrency.max(1))
        .collect()
        .await;
    let mut executables = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Uploaded(rel, true) => executables.push(rel),
            Outcome::Uploaded(rel, false) => report.uploaded.push(rel),
            Outcome::Changed(rel) => report.changed.push(rel),
            Outcome::Cancelled(rel) => report.cancelled.push(rel),
            Outcome::Failed(rel, reason) => report.failed.push((rel, reason)),
        }
    }
    executables.sort();
    let chmod_cancelled = cancelled(opts);
    let chmod_failed = if chmod_cancelled {
        report.cancelled.extend(executables.iter().cloned());
        Vec::new()
    } else {
        exec_paths(
        backend,
        sandbox,
        &opts.remote_root,
        &["chmod", "+x"],
        &executables,
    )
    .await
    };
    for rel in executables {
        if !chmod_cancelled && !chmod_failed.iter().any(|(p, _)| *p == rel) {
            report.uploaded.push(rel);
        }
    }
    report.failed.extend(chmod_failed);
    let delete_failed = if cancelled(opts) {
        report.cancelled.extend(allowed_deletes.iter().cloned());
        Vec::new()
    } else {
        exec_paths(
        backend,
        sandbox,
        &opts.remote_root,
        &["rm", "-f"],
        &allowed_deletes,
    )
    .await
    };
    report.deleted = allowed_deletes
        .iter()
        .filter(|p| !delete_failed.iter().any(|(f, _)| f == *p))
        .cloned()
        .collect();
    report.failed.extend(delete_failed);
    report.uploaded.sort();
    report.changed.sort();
    report.failed.sort();
    report.cancelled.sort();
    report.cancelled.dedup();
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
    root.split('/').skip(1).all(|part| part != "." && part != "..")
}
