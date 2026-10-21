//! Apply a plan through the sdk `SandboxBackend` transport.
//!
//! Backend contract: `upload` writes exactly the given bytes (no EOL translation) and creates
//! parent directories. Deletes, links and the executable bit go through `exec` argv (no shell).
//! Chunking below the gateway's 1 MiB request limit (spike S0-7) is the backend's job.

use std::path::Path;

use bytes::Bytes;
use futures::StreamExt;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxHandle};

use crate::path::{RelPath, SyncError};
use crate::plan::SyncPlan;
use crate::scan::{EntryKind, Manifest};

/// Paths per `rm` / `chmod` exec call.
const ARGS_PER_EXEC: usize = 256;

/// Transfer options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOpts {
    /// Absolute workspace directory inside the sandbox (e.g. `/sandbox/ws`).
    pub remote_root: String,
    /// Maximum concurrent uploads (0 behaves like 1).
    pub concurrency: usize,
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
    Failed(RelPath, String),
}

fn remote_path(root: &str, rel: &RelPath) -> String {
    format!("{}/{}", root.trim_end_matches('/'), rel.as_str())
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
    let Some(entry) = local.entries.get(rel) else {
        return Outcome::Failed(rel.clone(), "not in the local manifest".into());
    };
    let target = remote_path(&opts.remote_root, rel);
    match &entry.kind {
        EntryKind::Link { target: link } => {
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
            Ok(out) => format!(
                "{} exited {}",
                prefix.first().unwrap_or(&"exec"),
                out.exit_code
            ),
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
    if !opts.remote_root.starts_with('/') || opts.remote_root.contains("..") {
        return Err(SyncError::InvalidPath {
            path: opts.remote_root.clone(),
            reason: "remote root must be an absolute sandbox path without '..'",
        });
    }
    let outcomes: Vec<Outcome> = futures::stream::iter(&plan.upload)
        .map(|rel| upload_one(backend, sandbox, root, local, opts, rel))
        .buffer_unordered(opts.concurrency.max(1))
        .collect()
        .await;
    let mut report = SyncReport::default();
    let mut executables = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Uploaded(rel, true) => executables.push(rel),
            Outcome::Uploaded(rel, false) => report.uploaded.push(rel),
            Outcome::Changed(rel) => report.changed.push(rel),
            Outcome::Failed(rel, reason) => report.failed.push((rel, reason)),
        }
    }
    executables.sort();
    let chmod_failed = exec_paths(
        backend,
        sandbox,
        &opts.remote_root,
        &["chmod", "+x"],
        &executables,
    )
    .await;
    for rel in executables {
        if !chmod_failed.iter().any(|(p, _)| *p == rel) {
            report.uploaded.push(rel);
        }
    }
    report.failed.extend(chmod_failed);
    let delete_failed = exec_paths(
        backend,
        sandbox,
        &opts.remote_root,
        &["rm", "-f"],
        &plan.delete,
    )
    .await;
    report.deleted = plan
        .delete
        .iter()
        .filter(|p| !delete_failed.iter().any(|(f, _)| f == *p))
        .cloned()
        .collect();
    report.failed.extend(delete_failed);
    report.uploaded.sort();
    report.changed.sort();
    report.failed.sort();
    Ok(report)
}
