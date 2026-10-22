//! Sync Apply (descriptor: specs/descriptors/sync_apply.json). The sandbox is an in-memory fake of
//! the sdk `SandboxBackend` (never shipped).

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use polaris_sdk::registry::{
    CommitOutcome, ExecOutput, ExecRequest, SandboxBackend, SandboxBatchAction, SandboxBatchResult,
    SandboxCommitAction, SandboxCommitRequest, SandboxError, SandboxFileState, SandboxHandle,
    SandboxSpec, SandboxSyncBatch,
};
use polaris_storage_sync::{
    ApplyOpts, HashMode, IgnoreRules, Manifest, RelPath, apply, apply_with_precheck, plan, scan,
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Default)]
struct FakeSandbox {
    prepared: Mutex<BTreeSet<String>>,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    executable: Mutex<BTreeSet<String>>,
    fail_commits: Mutex<BTreeSet<String>>,
    intervene: Mutex<Option<(String, Vec<u8>)>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    batch_count: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, SandboxError> {
    m.lock()
        .map_err(|_| SandboxError::Protocol("poisoned".into()))
}

impl SandboxBackend for FakeSandbox {
    async fn prepare_sync(&self, _sandbox: &SandboxHandle, root: &str) -> Result<(), SandboxError> {
        if root != "/sandbox/ws" {
            return Err(SandboxError::PolicyDenied(
                "unexpected fake namespace".into(),
            ));
        }
        lock(&self.prepared)?.insert(root.to_owned());
        Ok(())
    }
    async fn conditional_commit(
        &self,
        _sandbox: &SandboxHandle,
        request: &SandboxCommitRequest,
    ) -> Result<CommitOutcome, SandboxError> {
        let path = format!("{}/{}", request.root, request.relative_path);
        if lock(&self.fail_commits)?.contains(&path) {
            return Err(SandboxError::Unavailable(
                "injected commit transport failure".into(),
            ));
        }
        let mut files = lock(&self.files)?;
        let mut executable = lock(&self.executable)?;
        if let Some((path, bytes)) = lock(&self.intervene)?.take() {
            files.insert(path, bytes);
        }
        let current = files.get(&path).map(|bytes| SandboxFileState {
            hash: polaris_sdk::ids::Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
            executable: executable.contains(&path),
            link_target: None,
        });
        if current != request.expected {
            return Ok(CommitOutcome::Conflict);
        }
        match &request.action {
            SandboxCommitAction::Delete => {
                files.remove(&path);
                executable.remove(&path);
            }
            SandboxCommitAction::File {
                staged_path,
                hash,
                executable: mode,
            } => {
                let bytes = files
                    .get(staged_path)
                    .ok_or_else(|| SandboxError::NotFound(staged_path.clone()))?;
                if polaris_sdk::ids::Cid::from_bytes(*blake3::hash(bytes).as_bytes()) != *hash {
                    return Err(SandboxError::Protocol("stage digest mismatch".into()));
                }
                let bytes = bytes.clone();
                files.remove(staged_path);
                files.insert(path.clone(), bytes);
                if *mode {
                    executable.insert(path);
                } else {
                    executable.remove(&path);
                }
            }
            SandboxCommitAction::Link { .. } => {
                return Err(SandboxError::PolicyDenied(
                    "this file-only fake does not model links".into(),
                ));
            }
        }
        Ok(CommitOutcome::Applied)
    }
    async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        Err(SandboxError::Protocol("not used".into()))
    }

    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        req: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        let (cmd, args) = req
            .argv
            .split_first()
            .ok_or_else(|| SandboxError::Protocol("empty argv".into()))?;
        let paths: Vec<&String> = args
            .iter()
            .filter(|a| !a.starts_with('-') && *a != "+x")
            .collect();
        match cmd.as_str() {
            "/usr/bin/polaris-guest" if args.first().is_some_and(|arg| arg == "sync-batch") => {
                self.batch_count.fetch_add(1, Ordering::SeqCst);
                let root = args
                    .get(1)
                    .ok_or_else(|| SandboxError::Protocol("missing root".into()))?;
                let archive = args
                    .get(2)
                    .ok_or_else(|| SandboxError::Protocol("missing archive".into()))?;
                let archive_bytes = lock(&self.files)?
                    .remove(archive)
                    .ok_or_else(|| SandboxError::NotFound(archive.clone()))?;
                let mut tar = tar::Archive::new(std::io::Cursor::new(archive_bytes));
                let mut entries = tar
                    .entries()
                    .map_err(|error| SandboxError::Protocol(error.to_string()))?;
                let mut manifest_bytes = Vec::new();
                entries
                    .next()
                    .ok_or_else(|| SandboxError::Protocol("missing manifest".into()))?
                    .map_err(|error| SandboxError::Protocol(error.to_string()))?
                    .read_to_end(&mut manifest_bytes)
                    .map_err(|error| SandboxError::Protocol(error.to_string()))?;
                let batch: SandboxSyncBatch = serde_json::from_slice(&manifest_bytes)
                    .map_err(|error| SandboxError::Protocol(error.to_string()))?;
                if batch.root != *root {
                    return Err(SandboxError::Protocol("batch root mismatch".into()));
                }
                let mut payloads = BTreeMap::new();
                for entry in entries {
                    let mut entry =
                        entry.map_err(|error| SandboxError::Protocol(error.to_string()))?;
                    let name = entry
                        .path()
                        .map_err(|error| SandboxError::Protocol(error.to_string()))?
                        .to_string_lossy()
                        .into_owned();
                    let mut bytes = Vec::new();
                    entry
                        .read_to_end(&mut bytes)
                        .map_err(|error| SandboxError::Protocol(error.to_string()))?;
                    payloads.insert(name, bytes);
                }
                let mut files = lock(&self.files)?;
                let mut executable = lock(&self.executable)?;
                if let Some((path, bytes)) = lock(&self.intervene)?.take() {
                    files.insert(path, bytes);
                }
                let state = |path: &str,
                             files: &BTreeMap<String, Vec<u8>>,
                             executable: &BTreeSet<String>| {
                    files.get(path).map(|bytes| SandboxFileState {
                        hash: polaris_sdk::ids::Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
                        executable: executable.contains(path),
                        link_target: None,
                    })
                };
                let mut result = SandboxBatchResult {
                    applied: Vec::new(),
                    conflicts: Vec::new(),
                    failed: Vec::new(),
                };
                for mutation in &batch.mutations {
                    let path = format!("{root}/{}", mutation.relative_path);
                    if state(&path, &files, &executable) != mutation.expected {
                        result.conflicts.push(mutation.relative_path.clone());
                    }
                }
                if !result.conflicts.is_empty() {
                    let mismatches = result.conflicts.iter().cloned().collect::<BTreeSet<_>>();
                    result.failed.extend(
                        batch
                            .mutations
                            .iter()
                            .filter(|mutation| !mismatches.contains(&mutation.relative_path))
                            .map(|mutation| mutation.relative_path.clone()),
                    );
                } else {
                    let mut failed = false;
                    for mutation in &batch.mutations {
                        let path = format!("{root}/{}", mutation.relative_path);
                        if failed || lock(&self.fail_commits)?.contains(&path) {
                            failed = true;
                            result.failed.push(mutation.relative_path.clone());
                            continue;
                        }
                        match &mutation.action {
                            SandboxBatchAction::Delete => {
                                files.remove(&path);
                                executable.remove(&path);
                            }
                            SandboxBatchAction::File {
                                payload,
                                hash,
                                executable: mode,
                            } => {
                                let bytes = payloads.get(payload).ok_or_else(|| {
                                    SandboxError::Protocol("missing payload".into())
                                })?;
                                if polaris_sdk::ids::Cid::from_bytes(
                                    *blake3::hash(bytes).as_bytes(),
                                ) != *hash
                                {
                                    return Err(SandboxError::Protocol(
                                        "payload hash mismatch".into(),
                                    ));
                                }
                                files.insert(path.clone(), bytes.clone());
                                if *mode {
                                    executable.insert(path);
                                } else {
                                    executable.remove(&path);
                                }
                            }
                            SandboxBatchAction::Link { .. } => {
                                return Err(SandboxError::PolicyDenied(
                                    "fake does not model links".into(),
                                ));
                            }
                        }
                        result.applied.push(mutation.relative_path.clone());
                    }
                }
                let stdout = serde_json::to_vec(&result)
                    .map_err(|error| SandboxError::Protocol(error.to_string()))?;
                return Ok(ExecOutput {
                    stdout: stdout.into(),
                    ..ExecOutput::default()
                });
            }
            "test" => {
                let executable = lock(&self.executable)?;
                let path = match paths.first() {
                    Some(path) => path.as_str(),
                    None => "",
                };
                let expected = req.argv.iter().any(|arg| arg == "-x");
                let observed = executable.contains(path);
                if expected == observed {
                    return Ok(ExecOutput::default());
                }
                return Ok(ExecOutput {
                    exit_code: 1,
                    ..ExecOutput::default()
                });
            }
            "rm" => {
                let mut files = lock(&self.files)?;
                paths.iter().for_each(|p| {
                    files.remove(p.as_str());
                });
            }
            "chmod" => lock(&self.executable)?.extend(paths.iter().map(|p| (*p).clone())),
            other => {
                return Err(SandboxError::Protocol(format!(
                    "unexpected command {other}"
                )));
            }
        }
        Ok(ExecOutput::default())
    }

    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        path: &str,
        bytes: Bytes,
    ) -> Result<(), SandboxError> {
        if !lock(&self.prepared)?
            .iter()
            .any(|root| path.starts_with(&format!("{root}/.polaris/sync/staging/")))
        {
            return Err(SandboxError::PolicyDenied(
                "fake stage namespace was not prepared".into(),
            ));
        }
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        lock(&self.files)?.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    async fn download(&self, _sandbox: &SandboxHandle, path: &str) -> Result<Bytes, SandboxError> {
        lock(&self.files)?
            .get(path)
            .cloned()
            .map(Bytes::from)
            .ok_or_else(|| SandboxError::NotFound(path.into()))
    }

    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Ok(lock(&self.files)?.keys().cloned().collect())
    }

    async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        Ok(())
    }
}

fn workspace(root: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), "pub fn f() {}\n")?;
    fs::write(root.join("notes.txt"), b"crlf\r\nkept\r\n")?;
    fs::write(root.join("run.sh"), "#!/bin/sh\nexit 0\n")?;
    Ok(())
}

fn opts(concurrency: usize) -> ApplyOpts {
    ApplyOpts {
        remote_root: "/sandbox/ws".into(),
        concurrency,
        cancellation: None,
    }
}

fn handle() -> Result<SandboxHandle, Box<dyn Error>> {
    Ok(SandboxHandle::new("polaris-test")?)
}

#[tokio::test]
async fn apply_uploads_and_deletes_byte_exact() -> TestResult {
    let tmp = tempfile::tempdir()?;
    workspace(tmp.path())?;
    let fake = FakeSandbox::default();
    lock(&fake.files)?.insert("/sandbox/ws/stale.rs".into(), b"old".to_vec());
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    let mut remote = Manifest::default();
    remote.entries.insert(
        RelPath::new("stale.rs")?,
        polaris_storage_sync::FileEntry {
            size: 3,
            hash: polaris_sdk::ids::Cid::from_bytes(*blake3::hash(b"old").as_bytes()),
            executable: false,
            kind: polaris_storage_sync::EntryKind::File,
        },
    );
    let p = plan(&local, &remote);
    let report = apply(&fake, &handle()?, tmp.path(), &local, &p, &opts(4)).await?;
    assert!(
        report.failed.is_empty() && report.changed.is_empty(),
        "{report:?}"
    );
    let files = lock(&fake.files)?.clone();
    assert_eq!(
        files.keys().cloned().collect::<Vec<_>>(),
        [
            "/sandbox/ws/notes.txt",
            "/sandbox/ws/run.sh",
            "/sandbox/ws/src/lib.rs"
        ]
    );
    assert_eq!(
        files.get("/sandbox/ws/notes.txt").map(Vec::as_slice),
        Some(&b"crlf\r\nkept\r\n"[..])
    );
    assert_eq!(report.deleted, vec![RelPath::new("stale.rs")?]);
    Ok(())
}

#[tokio::test]
async fn apply_installs_executable_mode_in_commit() -> TestResult {
    let tmp = tempfile::tempdir()?;
    workspace(tmp.path())?;
    let fake = FakeSandbox::default();
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    apply(
        &fake,
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &Manifest::default()),
        &opts(2),
    )
    .await?;
    assert_eq!(
        lock(&fake.executable)?.iter().cloned().collect::<Vec<_>>(),
        ["/sandbox/ws/run.sh"]
    );
    Ok(())
}

#[tokio::test]
async fn partial_failure_is_reported_and_resumable() -> TestResult {
    let tmp = tempfile::tempdir()?;
    workspace(tmp.path())?;
    let fake = FakeSandbox::default();
    lock(&fake.fail_commits)?.insert("/sandbox/ws/src/lib.rs".into());
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    let remote = Manifest::default();
    let report = apply(
        &fake,
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &remote),
        &opts(4),
    )
    .await?;
    assert_eq!(
        report
            .failed
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs"]
    );
    assert!(report.rescan_required);
    assert_eq!(report.uploaded.len(), 2);
    let remote = report.applied_to(&remote, &local);
    let again = plan(&local, &remote);
    assert_eq!(
        again.upload,
        vec![RelPath::new("src/lib.rs")?],
        "only the failed path is retried"
    );
    lock(&fake.fail_commits)?.clear();
    let report = apply(&fake, &handle()?, tmp.path(), &local, &again, &opts(4)).await?;
    assert!(report.failed.is_empty());
    assert!(plan(&local, &report.applied_to(&remote, &local)).is_empty());
    Ok(())
}

#[tokio::test]
async fn file_changed_after_scan_is_skipped() -> TestResult {
    let tmp = tempfile::tempdir()?;
    workspace(tmp.path())?;
    let fake = FakeSandbox::default();
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    fs::write(
        tmp.path().join("src/lib.rs"),
        "pub fn f() { /* edited */ }\n",
    )?;
    let report = apply(
        &fake,
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &Manifest::default()),
        &opts(4),
    )
    .await?;
    assert_eq!(report.changed, vec![RelPath::new("src/lib.rs")?]);
    assert!(!lock(&fake.files)?.contains_key("/sandbox/ws/src/lib.rs"));
    Ok(())
}

#[tokio::test]
async fn secret_precheck_blocks_exact_bytes_before_remote_staging() -> TestResult {
    let tmp = tempfile::tempdir()?;
    workspace(tmp.path())?;
    fs::write(
        tmp.path().join("src/lib.rs"),
        b"const TOKEN: &str = \"secret\";\n",
    )?;
    let fake = FakeSandbox::default();
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Paranoid,
    )?
    .manifest;
    let checker =
        |_path: &RelPath, bytes: &[u8]| Ok(bytes.windows(6).any(|window| window == b"secret"));
    let report = apply_with_precheck(
        &fake,
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &Manifest::default()),
        &opts(2),
        Some(&checker),
    )
    .await?;
    assert_eq!(report.secret_blocked, vec![RelPath::new("src/lib.rs")?]);
    assert!(!lock(&fake.files)?.contains_key("/sandbox/ws/src/lib.rs"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_concurrency_is_bounded() -> TestResult {
    let tmp = tempfile::tempdir()?;
    for i in 0..24 {
        fs::write(tmp.path().join(format!("f{i:02}.txt")), format!("file {i}"))?;
    }
    let fake = Arc::new(FakeSandbox::default());
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    let report = apply(
        fake.as_ref(),
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &Manifest::default()),
        &opts(3),
    )
    .await?;
    assert_eq!(report.uploaded.len(), 24);
    let max = fake.max_in_flight.load(Ordering::SeqCst);
    assert_eq!(max, 1, "one bounded tar upload per batch");
    Ok(())
}

#[tokio::test]
async fn more_than_one_bounded_batch_is_applied_and_reported_exactly() -> TestResult {
    let tmp = tempfile::tempdir()?;
    for index in 0..520 {
        fs::write(
            tmp.path().join(format!("item-{index:04}.txt")),
            format!("payload {index}"),
        )?;
    }
    let fake = FakeSandbox::default();
    let local = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?
    .manifest;
    let report = apply(
        &fake,
        &handle()?,
        tmp.path(),
        &local,
        &plan(&local, &Manifest::default()),
        &opts(8),
    )
    .await?;
    assert_eq!(report.uploaded.len(), 520);
    assert!(report.failed.is_empty());
    assert!(!report.rescan_required);
    assert_eq!(fake.batch_count.load(Ordering::SeqCst), 2);
    assert_eq!(lock(&fake.files)?.len(), 520);
    Ok(())
}

#[tokio::test]
async fn remote_change_during_transfer_is_preserved_for_replace_and_delete() -> TestResult {
    for deleting in [false, true] {
        let tmp = tempfile::tempdir()?;
        fs::write(tmp.path().join("file.txt"), b"base")?;
        let rules = IgnoreRules::new(tmp.path(), None)?;
        let base = scan(tmp.path(), &rules, None, HashMode::Paranoid)?.manifest;
        if deleting {
            fs::remove_file(tmp.path().join("file.txt"))?;
        } else {
            fs::write(tmp.path().join("file.txt"), b"local edit")?;
        }
        let local = scan(tmp.path(), &rules, None, HashMode::Paranoid)?.manifest;
        let fake = FakeSandbox::default();
        lock(&fake.files)?.insert("/sandbox/ws/file.txt".into(), b"base".to_vec());
        *lock(&fake.intervene)? = Some((
            "/sandbox/ws/file.txt".into(),
            b"independent remote edit".to_vec(),
        ));
        let report = apply(
            &fake,
            &handle()?,
            tmp.path(),
            &local,
            &plan(&local, &base),
            &opts(2),
        )
        .await?;
        assert_eq!(report.conflicts, vec![RelPath::new("file.txt")?]);
        assert!(report.rescan_required);
        assert!(report.uploaded.is_empty() && report.deleted.is_empty());
        assert_eq!(
            lock(&fake.files)?
                .get("/sandbox/ws/file.txt")
                .map(Vec::as_slice),
            Some(b"independent remote edit".as_slice())
        );
        assert_eq!(lock(&fake.files)?.len(), 1, "conflicting stages cleaned up");
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_never_reports_unperformed_deletion() -> TestResult {
    let tmp = tempfile::tempdir()?;
    fs::write(tmp.path().join("file.txt"), b"base")?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    let base = scan(tmp.path(), &rules, None, HashMode::Paranoid)?.manifest;
    fs::remove_file(tmp.path().join("file.txt"))?;
    let fake = FakeSandbox::default();
    lock(&fake.files)?.insert("/sandbox/ws/file.txt".into(), b"base".to_vec());
    let mut options = opts(1);
    options.cancellation = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
    let report = apply(
        &fake,
        &handle()?,
        tmp.path(),
        &Manifest::default(),
        &plan(&Manifest::default(), &base),
        &options,
    )
    .await?;
    assert!(report.deleted.is_empty());
    assert_eq!(report.cancelled, vec![RelPath::new("file.txt")?]);
    assert!(lock(&fake.files)?.contains_key("/sandbox/ws/file.txt"));
    Ok(())
}
