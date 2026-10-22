//! Apply a plan through the sdk `SandboxBackend` transport.
//!
//! Backend contract: `upload` stages one bounded deterministic tar in a prepared private
//! namespace, then fixed-argv `polaris-guest sync-batch` revalidates every expected state under
//! the workspace lock before any mutation. Unknown helper outcomes require a full remote rescan.
//! The backend chunks each archive below the gateway's 1 MiB request limit (spike S0-7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{
    ExecRequest, SandboxBackend, SandboxBatchAction, SandboxBatchMutation, SandboxBatchResult,
    SandboxFileState, SandboxHandle, SandboxSyncBatch,
};

use crate::path::{RelPath, SyncError};
use crate::plan::{PathConflict, SyncPlan};
use crate::scan::{EntryKind, Manifest};

const MAX_BATCH_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_BATCH_FILE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_BATCH_MUTATIONS: usize = 512;

/// Transfer options.
#[derive(Debug, Clone)]
pub struct ApplyOpts {
    /// Absolute workspace directory inside the sandbox (e.g. `/sandbox/ws`).
    pub remote_root: String,
    /// Compatibility hint; atomic archive batches are applied sequentially.
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
    /// Remote state is uncertain or diverged; caller must discard its snapshot and rescan.
    pub rescan_required: bool,
    /// Windows path-fidelity conflicts that require caller resolution.
    pub path_conflicts: Vec<PathConflict>,
    /// Planned paths not started after cancellation was requested.
    pub cancelled: Vec<RelPath>,
}

impl SyncReport {
    /// The remote manifest after this apply (feed it to the next [`crate::plan`] to resume).
    /// Do not use this result when [`SyncReport::rescan_required`] is true.
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

fn cancelled(opts: &ApplyOpts) -> bool {
    opts.cancellation
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Acquire))
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

fn expected_state(entry: &crate::scan::FileEntry) -> SandboxFileState {
    SandboxFileState {
        hash: entry.hash,
        executable: entry.executable,
        link_target: match &entry.kind {
            EntryKind::File => None,
            EntryKind::Link { target } => Some(target.clone()),
        },
    }
}

fn read_local_bounded(path: &Path, size: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    if size > MAX_BATCH_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "source exceeds batch file bound",
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

fn parse_batch_result(
    bytes: &[u8],
    requested: &BTreeSet<String>,
) -> Result<SandboxBatchResult, String> {
    let result: SandboxBatchResult =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    for path in result
        .applied
        .iter()
        .chain(&result.conflicts)
        .chain(&result.failed)
    {
        if !requested.contains(path) || !seen.insert(path.as_str()) {
            return Err("batch helper returned duplicate or unrequested paths".into());
        }
    }
    if seen.len() != requested.len() {
        return Err("batch helper omitted requested paths".into());
    }
    Ok(result)
}

async fn flush_batch<B: SandboxBackend>(
    backend: &B,
    sandbox: &SandboxHandle,
    opts: &ApplyOpts,
    slice: &[(RelPath, SandboxBatchMutation, Option<Bytes>)],
    report: &mut SyncReport,
) {
    if slice.is_empty() {
        return;
    }
    let batch = SandboxSyncBatch {
        version: 1,
        root: opts.remote_root.clone(),
        mutations: slice
            .iter()
            .map(|(_, mutation, _)| mutation.clone())
            .collect(),
    };
    let payloads = slice
        .iter()
        .filter_map(|(_, mutation, bytes)| match (&mutation.action, bytes) {
            (SandboxBatchAction::File { payload, .. }, Some(bytes)) => {
                Some((payload.clone(), bytes.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let archive = match crate::encode_batch_archive(&batch, &payloads) {
        Ok(archive) => archive,
        Err(error) => {
            report.failed.extend(
                slice
                    .iter()
                    .map(|(rel, _, _)| (rel.clone(), error.to_string())),
            );
            return;
        }
    };
    let paths = slice
        .iter()
        .map(|(rel, _, _)| rel.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let stage = format!(
        "{}/.polaris/sync/staging/{}.tar",
        opts.remote_root.trim_end_matches('/'),
        uuid::Uuid::now_v7()
    );
    if let Err(error) = backend.upload(sandbox, &stage, archive).await {
        report.failed.extend(
            slice
                .iter()
                .map(|(rel, _, _)| (rel.clone(), error.to_string())),
        );
        return;
    }
    if cancelled(opts) {
        let cleanup = backend
            .exec(
                sandbox,
                &exec_req(vec!["rm".into(), "-f".into(), "--".into(), stage]),
            )
            .await;
        if !matches!(cleanup, Ok(ref output) if output.exit_code == 0) {
            report.failed.extend(
                slice.iter().map(|(rel, _, _)| {
                    (rel.clone(), "cancelled batch staging cleanup failed".into())
                }),
            );
        } else {
            report
                .cancelled
                .extend(slice.iter().map(|(rel, _, _)| rel.clone()));
        }
        return;
    }
    let request = exec_req(vec![
        "/usr/bin/polaris-guest".into(),
        "sync-batch".into(),
        opts.remote_root.clone(),
        stage,
    ]);
    match backend.exec(sandbox, &request).await {
        Ok(output) if output.exit_code == 0 => {
            match parse_batch_result(&output.stdout, &paths) {
                Ok(result) => {
                    if !result.conflicts.is_empty() || !result.failed.is_empty() {
                        report.rescan_required = true;
                    }
                    for value in result.applied {
                        if let Some((rel, mutation, _)) =
                            slice.iter().find(|(rel, _, _)| rel.as_str() == value)
                        {
                            if matches!(mutation.action, SandboxBatchAction::Delete) {
                                report.deleted.push(rel.clone());
                            } else {
                                report.uploaded.push(rel.clone());
                            }
                        }
                    }
                    report
                        .conflicts
                        .extend(result.conflicts.into_iter().filter_map(|path| {
                            slice
                                .iter()
                                .find(|(rel, _, _)| rel.as_str() == path)
                                .map(|(rel, _, _)| rel.clone())
                        }));
                    report
                        .failed
                        .extend(result.failed.into_iter().filter_map(|path| {
                            slice.iter().find(|(rel, _, _)| rel.as_str() == path).map(
                                |(rel, _, _)| {
                                    (
                                        rel.clone(),
                                        "guest batch apply failed after preflight".into(),
                                    )
                                },
                            )
                        }));
                }
                Err(reason) => {
                    report.rescan_required = true;
                    report.failed.extend(slice.iter().map(|(rel, _, _)| {
                        (rel.clone(), format!("untrusted batch result: {reason}"))
                    }));
                }
            }
        }
        Ok(output) => {
            report.rescan_required = true;
            let reason = format!("sync-batch exited with status {}", output.exit_code);
            report.failed.extend(
                slice
                    .iter()
                    .map(|(rel, _, _)| (rel.clone(), reason.clone())),
            );
        }
        Err(error) => {
            report.rescan_required = true;
            let reason = format!("sync-batch outcome unknown: {error}");
            report.failed.extend(
                slice
                    .iter()
                    .map(|(rel, _, _)| (rel.clone(), reason.clone())),
            );
        }
    }
}

/// Applies `plan` using private staging, bounded archive memory and locked batch compare-and-swap
/// that installs bytes and mode together. Per-path failures are reported, not fatal, so a
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
    let mut report = SyncReport::default();
    report
        .conflicts
        .extend(plan.conflicts.iter().map(|conflict| conflict.path.clone()));
    report.path_conflicts = plan.path_conflicts.clone();
    if !plan.upload.is_empty() || !plan.delete.is_empty() {
        backend
            .prepare_sync(sandbox, &opts.remote_root)
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
    }

    let mut pending: Vec<(RelPath, SandboxBatchMutation, Option<Bytes>)> = Vec::new();
    let mut payload_bytes = 0_u64;
    for rel in plan.upload.iter().chain(&plan.delete) {
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
        if !plan.expected_remote.contains_key(rel) {
            report.failed.push((
                rel.clone(),
                "plan has no remote compare-and-swap baseline".into(),
            ));
            continue;
        }
        let expected = plan
            .expected_remote
            .get(rel)
            .and_then(Option::as_ref)
            .map(expected_state);
        let action = if plan.delete.contains(rel) {
            SandboxBatchAction::Delete
        } else {
            let Some(entry) = local.entries.get(rel) else {
                report
                    .failed
                    .push((rel.clone(), "not in the local manifest".into()));
                continue;
            };
            match &entry.kind {
                EntryKind::Link { target } => SandboxBatchAction::Link {
                    target: target.clone(),
                    hash: entry.hash,
                },
                EntryKind::File => {
                    if entry.size > MAX_BATCH_FILE_BYTES {
                        report
                            .failed
                            .push((rel.clone(), "source file exceeds batch file bound".into()));
                        continue;
                    }
                    if !pending.is_empty()
                        && (pending.len() >= MAX_BATCH_MUTATIONS
                            || payload_bytes.saturating_add(entry.size)
                                > MAX_BATCH_ARCHIVE_BYTES - 8 * 1024 * 1024)
                    {
                        flush_batch(backend, sandbox, opts, &pending, &mut report).await;
                        pending.clear();
                        payload_bytes = 0;
                    }
                    let path = rel
                        .components()
                        .fold(root.to_path_buf(), |base, component| base.join(component));
                    let bytes = match read_local_bounded(&path, entry.size) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            report.changed.push(rel.clone());
                            continue;
                        }
                    };
                    if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != entry.hash {
                        report.changed.push(rel.clone());
                        continue;
                    }
                    if let Some(check) = secret_precheck {
                        match check(rel, &bytes) {
                            Ok(true) => {
                                report.secret_blocked.push(rel.clone());
                                continue;
                            }
                            Ok(false) => {}
                            Err(reason) => {
                                report.failed.push((rel.clone(), reason));
                                continue;
                            }
                        }
                    }
                    let payload = format!("payload/{:06}", pending.len());
                    let mutation = SandboxBatchMutation {
                        relative_path: rel.as_str().to_owned(),
                        expected,
                        action: SandboxBatchAction::File {
                            payload,
                            hash: entry.hash,
                            executable: entry.executable,
                        },
                    };
                    payload_bytes = payload_bytes.saturating_add(entry.size);
                    pending.push((rel.clone(), mutation, Some(Bytes::from(bytes))));
                    continue;
                }
            }
        };
        if pending.len() >= MAX_BATCH_MUTATIONS {
            flush_batch(backend, sandbox, opts, &pending, &mut report).await;
            pending.clear();
            payload_bytes = 0;
        }
        pending.push((
            rel.clone(),
            SandboxBatchMutation {
                relative_path: rel.as_str().to_owned(),
                expected,
                action,
            },
            None,
        ));
    }
    if !pending.is_empty() {
        flush_batch(backend, sandbox, opts, &pending, &mut report).await;
    }

    report.uploaded.sort();
    report.deleted.sort();
    report.changed.sort();
    report.failed.sort();
    report.cancelled.sort();
    report.cancelled.dedup();
    report.secret_blocked.sort();
    report.conflicts.sort();
    report.conflicts.dedup();
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
    let parts = root.split('/').skip(1);
    let mut any = false;
    for part in parts {
        if part.is_empty() || part == "." || part == ".." {
            return false;
        }
        any = true;
    }
    any
}
