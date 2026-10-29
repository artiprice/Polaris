//! Explicitly fake transport contract tests; live gateway acceptance has a separate gate.
// Test doubles and fixtures fail loudly: a panic is this test's failure signal.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
use polaris_kernel::{ExecutionRequest, Kernel, LocalKernelConfig, SandboxExecution};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
    registry::{
        CommitOutcome, ExecOutput, ExecRequest, SandboxBackend, SandboxBatchAction,
        SandboxBatchResult, SandboxCommitAction, SandboxCommitRequest, SandboxError,
        SandboxFileState, SandboxHandle, SandboxSnapshotEntry, SandboxSnapshotPage, SandboxSpec,
        SandboxSyncBatch,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct FakeBackend {
    prepared: AtomicBool,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    creates: AtomicUsize,
    destroys: AtomicUsize,
    fail: AtomicBool,
    collision: AtomicBool,
    no_tests: AtomicBool,
    tamper: AtomicBool,
    destroy_failure: AtomicBool,
    release_failure: Mutex<Option<std::path::PathBuf>>,
    wait: AtomicBool,
    started: tokio::sync::Notify,
}
impl SandboxBackend for FakeBackend {
    async fn snapshot_page(
        &self,
        _sandbox: &SandboxHandle,
        root: &str,
        generation: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<SandboxSnapshotPage, SandboxError> {
        use polaris_kernel::storage::sync::{EntryKind, FileEntry, Manifest, RelPath};
        let mut manifest = Manifest::default();
        let mut entries = Vec::new();
        let files = self.files.lock().expect("files");
        for (path, bytes) in files.iter() {
            let Some(relative) = path.strip_prefix(&format!("{root}/")) else {
                continue;
            };
            if relative.starts_with(".polaris/") {
                continue;
            }
            let hash = Cid::from_bytes(*blake3::hash(bytes).as_bytes());
            let size = u64::try_from(bytes.len()).expect("size");
            manifest.entries.insert(
                RelPath::new(relative).expect("path"),
                FileEntry {
                    size,
                    hash,
                    executable: false,
                    kind: EntryKind::File,
                },
            );
            entries.push(SandboxSnapshotEntry {
                relative_path: relative.to_owned(),
                size,
                hash,
                executable: false,
                link_target: None,
            });
        }
        let manifest_id = manifest.content_id();
        let token = format!("fake-{manifest_id}");
        if generation.is_some_and(|expected| expected != token) {
            return Err(SandboxError::Protocol("fake snapshot changed".into()));
        }
        let total_entries = u64::try_from(entries.len()).expect("count");
        let start = usize::try_from(offset).expect("offset");
        let selected: Vec<_> = entries
            .into_iter()
            .skip(start)
            .take(usize::try_from(limit).expect("limit"))
            .collect();
        let next = offset
            .checked_add(u64::try_from(selected.len()).expect("selected count"))
            .expect("next");
        Ok(SandboxSnapshotPage {
            generation: token,
            manifest_id,
            total_entries,
            offset,
            entries: selected,
            next_offset: if next < total_entries {
                Some(next)
            } else {
                None
            },
        })
    }
    async fn prepare_sync(&self, _sandbox: &SandboxHandle, root: &str) -> Result<(), SandboxError> {
        if root != "/sandbox/ws" {
            return Err(SandboxError::PolicyDenied("fake namespace".into()));
        }
        self.prepared.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn conditional_commit(
        &self,
        _sandbox: &SandboxHandle,
        request: &SandboxCommitRequest,
    ) -> Result<CommitOutcome, SandboxError> {
        let mut files = self.files.lock().expect("files");
        let path = format!("{}/{}", request.root, request.relative_path);
        let current = files.get(&path).map(|bytes| SandboxFileState {
            hash: Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
            executable: false,
            link_target: None,
        });
        if current != request.expected {
            return Ok(CommitOutcome::Conflict);
        }
        match &request.action {
            SandboxCommitAction::File {
                staged_path,
                executable: false,
                ..
            } => {
                let bytes = files
                    .remove(staged_path)
                    .ok_or_else(|| SandboxError::NotFound(staged_path.clone()))?;
                files.insert(path, bytes);
            }
            SandboxCommitAction::Delete => {
                files.remove(&path);
            }
            _ => return Err(SandboxError::PolicyDenied("file-only fake contract".into())),
        }
        Ok(CommitOutcome::Applied)
    }
    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        if self.collision.load(Ordering::SeqCst) {
            return Err(SandboxError::AlreadyExists(
                "fake preexisting resource".into(),
            ));
        }
        SandboxHandle::new(spec.name.as_deref().expect("journaled name"))
            .map_err(|error| SandboxError::Protocol(error.to_string()))
    }
    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        let program = request.argv.first().map(String::as_str).expect("argv");
        let mut output = ExecOutput::default();
        match program {
            "/usr/bin/polaris-guest"
                if request.argv.get(1).map(String::as_str) == Some("sync-batch") =>
            {
                use std::io::Read;
                let root = request.argv.get(2).expect("root");
                let stage = request.argv.get(3).expect("stage");
                let archive = self
                    .files
                    .lock()
                    .expect("files")
                    .remove(stage)
                    .expect("archive");
                let mut decoded = BTreeMap::new();
                for entry in tar::Archive::new(std::io::Cursor::new(archive))
                    .entries()
                    .expect("entries")
                {
                    let mut entry = entry.expect("entry");
                    let name = entry.path().expect("name").to_string_lossy().into_owned();
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes).expect("bytes");
                    assert!(
                        decoded.insert(name, bytes).is_none(),
                        "duplicate archive member"
                    );
                }
                let batch: SandboxSyncBatch =
                    serde_json::from_slice(decoded.get("manifest.json").expect("manifest"))
                        .expect("batch");
                assert_eq!(&batch.root, root, "fixed root");
                let mut files = self.files.lock().expect("files");
                let mut result = SandboxBatchResult {
                    applied: Vec::new(),
                    conflicts: Vec::new(),
                    failed: Vec::new(),
                };
                for mutation in &batch.mutations {
                    let path = format!("{root}/{}", mutation.relative_path);
                    let actual = files.get(&path).map(|bytes| SandboxFileState {
                        hash: Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
                        executable: false,
                        link_target: None,
                    });
                    if actual != mutation.expected {
                        result.conflicts.push(mutation.relative_path.clone());
                    }
                }
                if !result.conflicts.is_empty() {
                    result.failed.extend(
                        batch
                            .mutations
                            .iter()
                            .filter(|mutation| !result.conflicts.contains(&mutation.relative_path))
                            .map(|mutation| mutation.relative_path.clone()),
                    );
                } else {
                    for mutation in batch.mutations {
                        let path = format!("{root}/{}", mutation.relative_path);
                        match mutation.action {
                            SandboxBatchAction::File {
                                payload,
                                hash,
                                executable: false,
                            } => {
                                let bytes = decoded.remove(&payload).expect("payload");
                                assert_eq!(
                                    Cid::from_bytes(*blake3::hash(&bytes).as_bytes()),
                                    hash,
                                    "payload hash"
                                );
                                files.insert(path, bytes);
                            }
                            SandboxBatchAction::Delete => {
                                files.remove(&path);
                            }
                            _ => {
                                return Err(SandboxError::PolicyDenied(
                                    "file-only kernel fake".into(),
                                ));
                            }
                        }
                        result.applied.push(mutation.relative_path);
                    }
                }
                output.stdout = serde_json::to_vec(&result).expect("result").into();
            }
            "/usr/bin/polaris-guest" => output.stdout = b"1\n".to_vec().into(),
            "test" => {
                let path = request.argv.last().expect("path");
                let exists = self.files.lock().expect("files").contains_key(path);
                output.exit_code =
                    if request.argv.get(1).map(String::as_str) == Some("-e") && exists {
                        0
                    } else {
                        1
                    };
            }
            "cargo" => {
                self.started.notify_one();
                if self.wait.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                output.exit_code = if self.fail.load(Ordering::SeqCst) {
                    1
                } else {
                    0
                };
                output.stdout = if output.exit_code == 0 {
                    b"test result: ok. 3 passed; 0 failed; 0 ignored;\ntest result: ok. 0 passed; 0 failed; 0 ignored;\n".to_vec().into()
                } else {
                    b"test result: FAILED. 2 passed; 1 failed; 0 ignored;\n"
                        .to_vec()
                        .into()
                };
                if self.no_tests.load(Ordering::SeqCst) {
                    output.stdout = b"test result: ok. 0 passed; 0 failed; 0 ignored;\n"
                        .to_vec()
                        .into();
                }
                if self.tamper.load(Ordering::SeqCst) {
                    self.files.lock().expect("files").insert(
                        "/sandbox/ws/source.rs".into(),
                        b"fn tampered() {}\n".to_vec(),
                    );
                }
            }
            "mkdir" | "chmod" => {}
            _ => {
                return Err(SandboxError::Protocol(format!(
                    "unexpected fake command {program}"
                )));
            }
        }
        Ok(output)
    }
    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        path: &str,
        bytes: bytes::Bytes,
    ) -> Result<(), SandboxError> {
        if !self.prepared.load(Ordering::SeqCst) {
            return Err(SandboxError::PolicyDenied(
                "fake namespace unprepared".into(),
            ));
        }
        self.files
            .lock()
            .expect("files")
            .insert(path.into(), bytes.to_vec());
        Ok(())
    }
    async fn download(
        &self,
        _sandbox: &SandboxHandle,
        path: &str,
    ) -> Result<bytes::Bytes, SandboxError> {
        self.files
            .lock()
            .expect("files")
            .get(path)
            .cloned()
            .map(Into::into)
            .ok_or_else(|| SandboxError::NotFound(path.into()))
    }
    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Ok(self
            .files
            .lock()
            .expect("files")
            .keys()
            .filter(|path| path.starts_with(&format!("{dir}/")))
            .cloned()
            .collect())
    }
    async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        self.destroys.fetch_add(1, Ordering::SeqCst);
        if self.destroy_failure.load(Ordering::SeqCst) {
            return Err(SandboxError::Unavailable("injected destroy failure".into()));
        }
        self.files.lock().expect("files").clear();
        if let Some(journal) = self.release_failure.lock().expect("release fault").take() {
            std::fs::rename(&journal, journal.with_extension("release-backup"))
                .expect("retain original fake recovery journal");
            std::fs::create_dir(&journal).expect("inject journal removal failure");
        }
        Ok(())
    }
}

fn spec() -> SandboxSpec {
    SandboxSpec {
        image: Some(format!("sha256:{}", "a".repeat(64))),
        policy: Some(Cid::from_bytes([1; 32])),
        ..SandboxSpec::default()
    }
}
fn request() -> ExecutionRequest {
    ExecutionRequest {
        argv: vec!["cargo".into(), "test".into(), "--lib".into()],
        independent_argv: vec![
            "cargo".into(),
            "test".into(),
            "--test".into(),
            "independent".into(),
        ],
        timeout_secs: 10,
        min_tests: 3,
    }
}
async fn shutdown_without_leaks(kernel: &polaris_kernel::KernelHandle) {
    let run = kernel.context().root().join("run");
    let report = kernel.shutdown().await.expect("shutdown");
    assert!(!report.deadline_exceeded);
    assert!(report.journal_error.is_none());
    assert!(
        polaris_kernel::reaper::Reaper::new(run)
            .expect("journal")
            .leak_report()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn stale_or_replayed_review_is_rejected_before_create() {
    let dir = tempfile::tempdir().expect("temp");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("source.rs"), "fn source() {}\n").expect("source");
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        dir.path().join("state"),
        &workspace,
        session,
        frame,
    ))
    .await
    .expect("kernel");
    let backend = Arc::new(FakeBackend::default());
    let engine =
        SandboxExecution::new(Arc::clone(&backend), spec(), session, frame).expect("engine");
    let review = engine
        .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
        .await
        .expect("review");
    let token = engine
        .credential(review.digest, review.digest)
        .expect("approval");
    std::fs::write(workspace.join("source.rs"), "fn changed() {}\n").expect("edit");
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        token,
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    assert!(
        engine
            .execute(
                Arc::clone(kernel.context()),
                workspace.clone(),
                command,
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(
        backend.creates.load(Ordering::SeqCst),
        0,
        "stale approval never creates sandbox"
    );
    let review = engine
        .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
        .await
        .expect("fresh review");
    let token = engine
        .credential(review.digest, review.digest)
        .expect("approval");
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        token.clone(),
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    engine
        .execute(
            Arc::clone(kernel.context()),
            workspace.clone(),
            command,
            CancellationToken::new(),
        )
        .await
        .expect("fake transport proof");
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        token,
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    assert!(
        engine
            .execute(
                Arc::clone(kernel.context()),
                workspace.clone(),
                command,
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
    assert_eq!(backend.destroys.load(Ordering::SeqCst), 1);
    shutdown_without_leaks(&kernel).await;
    drop(engine);
    drop(kernel);
    let resumed = Kernel::start(LocalKernelConfig::new(
        dir.path().join("state"),
        &workspace,
        session,
        frame,
    ))
    .await
    .expect("recover session");
    let restarted = SandboxExecution::new(Arc::clone(&backend), spec(), session, frame)
        .expect("fresh authority");
    let token = restarted
        .credential(review.digest, review.digest)
        .expect("new approval credential");
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        token,
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    assert!(
        restarted
            .execute(
                Arc::clone(resumed.context()),
                workspace,
                command,
                CancellationToken::new()
            )
            .await
            .is_err(),
        "consumed approval remains consumed after reopening with a fresh issuer"
    );
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
    shutdown_without_leaks(&resumed).await;
}

#[tokio::test]
async fn failed_run_does_not_publish_proof() {
    let dir = tempfile::tempdir().expect("temp");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("source.rs"), "fn source() {}\n").expect("source");
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        dir.path().join("state"),
        &workspace,
        session,
        frame,
    ))
    .await
    .expect("kernel");
    let backend = Arc::new(FakeBackend::default());
    backend.fail.store(true, Ordering::SeqCst);
    let engine =
        SandboxExecution::new(Arc::clone(&backend), spec(), session, frame).expect("engine");
    let review = engine
        .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
        .await
        .expect("review");
    let token = engine
        .credential(review.digest, review.digest)
        .expect("approval");
    assert!(
        engine
            .execute(
                Arc::clone(kernel.context()),
                workspace,
                Command::new(
                    CommandId::new_v7(),
                    frame,
                    token,
                    CommandBody::ExecuteTool {
                        call: review.digest
                    }
                ),
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    let records = kernel
        .context()
        .records(Seq::ZERO, Seq::new(u64::MAX))
        .expect("events");
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event(), Event::ProofOfDonePassed { .. }))
    );
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event(), Event::ProofOfDoneBlocked { .. }))
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.event(), Event::ProofOfDoneBlocked { .. }))
            .count(),
        1,
        "one execution must publish exactly one terminal blocked proof"
    );
    assert_eq!(backend.destroys.load(Ordering::SeqCst), 1);
    shutdown_without_leaks(&kernel).await;
}

#[tokio::test]
async fn zero_tests_tampered_source_and_name_collision_never_complete() {
    for scenario in [
        "zero_tests",
        "tamper",
        "collision",
        "secret",
        "destroy_failure",
        "release_failure",
    ] {
        let dir = tempfile::tempdir().expect("temp");
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace");
        let source = if scenario == "secret" {
            "AKIA0123456789ABCDEF\n"
        } else {
            "fn source() {}\n"
        };
        std::fs::write(workspace.join("source.rs"), source).expect("source");
        let session = SessionId::new_v7();
        let frame = FrameId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            dir.path().join("state"),
            &workspace,
            session,
            frame,
        ))
        .await
        .expect("kernel");
        let backend = Arc::new(FakeBackend::default());
        let journal = kernel.context().root().join("run/reaper.journal");
        if scenario == "release_failure" {
            *backend.release_failure.lock().expect("release fault") = Some(journal.clone());
        }
        backend
            .no_tests
            .store(scenario == "zero_tests", Ordering::SeqCst);
        backend.tamper.store(scenario == "tamper", Ordering::SeqCst);
        backend
            .collision
            .store(scenario == "collision", Ordering::SeqCst);
        backend
            .destroy_failure
            .store(scenario == "destroy_failure", Ordering::SeqCst);
        let engine =
            SandboxExecution::new(Arc::clone(&backend), spec(), session, frame).expect("engine");
        let review = engine
            .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
            .await
            .expect("review");
        let token = engine
            .credential(review.digest, review.digest)
            .expect("approval");
        let result = engine
            .execute(
                Arc::clone(kernel.context()),
                workspace,
                Command::new(
                    CommandId::new_v7(),
                    frame,
                    token,
                    CommandBody::ExecuteTool {
                        call: review.digest,
                    },
                ),
                CancellationToken::new(),
            )
            .await;
        assert!(result.is_err(), "{scenario} must be blocked");
        if scenario == "release_failure" {
            assert!(
                result
                    .as_ref()
                    .err()
                    .expect("release must fail")
                    .to_string()
                    .contains("reaper journal")
            );
        }
        let records = kernel
            .context()
            .records(Seq::ZERO, Seq::new(u64::MAX))
            .expect("records");
        assert!(
            !records
                .iter()
                .any(|record| matches!(record.event(), Event::ProofOfDonePassed { .. })),
            "{scenario} must never publish a pass"
        );
        assert!(
            records
                .iter()
                .any(|record| matches!(record.event(), Event::ProofOfDoneBlocked { .. })),
            "{scenario} must publish a durable blocker"
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.event(), Event::ProofOfDoneBlocked { .. }))
                .count(),
            1,
            "{scenario} must publish exactly one terminal blocked proof"
        );
        assert_eq!(
            backend.destroys.load(Ordering::SeqCst),
            if scenario == "collision" { 0 } else { 1 },
            "{scenario} cleanup ownership"
        );
        backend.destroy_failure.store(false, Ordering::SeqCst);
        if scenario == "release_failure" {
            std::fs::remove_dir(&journal).expect("remove empty injected fault directory");
            std::fs::rename(journal.with_extension("release-backup"), &journal)
                .expect("restore retained recovery journal");
        }
        shutdown_without_leaks(&kernel).await;
    }
}

#[tokio::test]
async fn changed_review_environment_and_remote_root_are_rejected_before_create() {
    let directory = tempfile::tempdir().expect("temp");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("source.rs"), b"fn source() {}\n").expect("source");
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        directory.path().join("state"),
        &workspace,
        session,
        frame,
    ))
    .await
    .expect("kernel");
    let backend = Arc::new(FakeBackend::default());
    let engine =
        SandboxExecution::new(Arc::clone(&backend), spec(), session, frame).expect("engine");
    let review = engine
        .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
        .await
        .expect("review");
    for field in ["environment", "remote_root", "version"] {
        let mut changed = review.details.clone();
        match field {
            "environment" => changed["environment"]["PATH"] = serde_json::json!("/sandbox/ws"),
            "remote_root" => changed["remote_root"] = serde_json::json!("/sandbox/other"),
            _ => changed["version"] = serde_json::json!(1),
        }
        use polaris_kernel::verifier::EvidenceStore;
        let cid = polaris_kernel::KernelEvidence::new(Arc::clone(kernel.context()))
            .put(&serde_json::to_vec(&changed).expect("json"))
            .expect("CAS");
        let token = engine.credential(cid, cid).expect("credential");
        let result = engine
            .execute(
                Arc::clone(kernel.context()),
                workspace.clone(),
                Command::new(
                    CommandId::new_v7(),
                    frame,
                    token,
                    CommandBody::ExecuteTool { call: cid },
                ),
                CancellationToken::new(),
            )
            .await;
        assert!(result.is_err(), "{field} changed without runtime agreement");
    }
    assert_eq!(
        backend.creates.load(Ordering::SeqCst),
        0,
        "context mismatch never creates sandbox"
    );
    shutdown_without_leaks(&kernel).await;
}

#[tokio::test]
async fn cancellation_retains_cleanup_ownership() {
    let dir = tempfile::tempdir().expect("temp");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    std::fs::write(workspace.join("source.rs"), "fn source() {}\n").expect("source");
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        dir.path().join("state"),
        &workspace,
        session,
        frame,
    ))
    .await
    .expect("kernel");
    let backend = Arc::new(FakeBackend::default());
    backend.wait.store(true, Ordering::SeqCst);
    let engine =
        SandboxExecution::new(Arc::clone(&backend), spec(), session, frame).expect("engine");
    let review = engine
        .prepare(Arc::clone(kernel.context()), workspace.clone(), request())
        .await
        .expect("review");
    let token = engine
        .credential(review.digest, review.digest)
        .expect("approval");
    let stop = CancellationToken::new();
    let child = stop.clone();
    let ctx = Arc::clone(kernel.context());
    let task = tokio::spawn(async move {
        engine
            .execute(
                ctx,
                workspace,
                Command::new(
                    CommandId::new_v7(),
                    frame,
                    token,
                    CommandBody::ExecuteTool {
                        call: review.digest,
                    },
                ),
                child,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), backend.started.notified())
        .await
        .expect("runner started");
    stop.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("owned cleanup finished")
            .expect("joined")
            .is_err()
    );
    assert_eq!(backend.destroys.load(Ordering::SeqCst), 1);
    shutdown_without_leaks(&kernel).await;
}

// Synthetic sandbox transport; production verifier reads actual CAS, event order and source scans.
#[tokio::test]
async fn frame_completion_verifier_requires_current_actual_proof_and_latest_verdict()
-> Result<(), Box<dyn std::error::Error>> {
    use polaris_runtime::frames::{
        AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    };
    use polaris_security::capability::{
        Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
    };
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("source.rs"), b"fn source() {}\n")?;
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        state.path(),
        workspace.path(),
        session,
        frame,
    ))
    .await?;
    let authority = Authority::new_session()?;
    let scope = Scope {
        resource: ResourceScope::Workspace,
        max_churn_lines: 0,
        ttl: Duration::from_secs(120),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    let token = authority.issue_root(frame, &scope, &SystemClock)?;
    let active = AgentContextFrame::new_root(
        frame,
        AgentRole::Orchestrator,
        token,
        FrameScope {
            capability: scope,
            expires_at_micros: SystemClock
                .now()
                .as_microsecond()
                .checked_add(120_000_000)
                .ok_or("expiry overflow")?,
        },
        ResourceQuota {
            cpu_millis: 10000,
            memory_mb: 128,
            network: Default::default(),
            max_steps: 8,
            max_depth: 2,
            token_budget: 1000,
        },
        ScratchpadRef::new(Cid::from_bytes([8; 32])),
    )
    .activate(
        ModelSelection {
            model: polaris_sdk::ids::ModelId::new("fixture/proof-model")?,
            maximum_effort: polaris_sdk::effort::Effort::Low,
        },
        polaris_sdk::effort::Effort::Low,
    )?;
    let verifier = polaris_kernel::KernelFrameVerifier::new(kernel.clone(), active.tool_binding())?;
    assert!(
        verifier.validate_completion(&active.view()).await.is_err(),
        "no execution means no completion proof"
    );
    let backend = Arc::new(FakeBackend::default());
    let execution = SandboxExecution::new(Arc::clone(&backend), spec(), session, frame)?;
    let review = execution
        .prepare(
            Arc::clone(kernel.context()),
            workspace.path().to_owned(),
            request(),
        )
        .await?;
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        execution.credential(review.digest, review.digest)?,
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    execution
        .execute(
            Arc::clone(kernel.context()),
            workspace.path().to_owned(),
            command,
            CancellationToken::new(),
        )
        .await?;
    let proof = verifier.validate_completion(&active.view()).await?;
    assert_eq!(
        proof.source_cid(),
        review.tree,
        "proof must bind the exact paranoid manifest"
    );
    {
        use polaris_kernel::verifier::EvidenceStore;
        let bytes = polaris_kernel::KernelEvidence::new(Arc::clone(kernel.context()))
            .get(&proof.report_cid())?
            .ok_or("missing proof report")?;
        let report: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(
            report.get("schema"),
            Some(&serde_json::json!(2)),
            "current proof report must use the explicit bound schema"
        );
        assert_eq!(
            report.get("review"),
            Some(&serde_json::json!(review.digest)),
            "proof must name the exact approved execution review"
        );
    }
    assert_eq!(proof.frame(), frame);
    assert_eq!(proof.session(), session);
    std::fs::write(workspace.path().join("source.rs"), b"fn changed() {}\n")?;
    assert!(
        verifier.validate_completion(&active.view()).await.is_err(),
        "actual source mutation must invalidate prior pass"
    );
    std::fs::write(workspace.path().join("source.rs"), b"fn source() {}\n")?;
    verifier.validate_completion(&active.view()).await?;
    backend.fail.store(true, Ordering::SeqCst);
    let review = execution
        .prepare(
            Arc::clone(kernel.context()),
            workspace.path().to_owned(),
            request(),
        )
        .await?;
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        execution.credential(review.digest, review.digest)?,
        CommandBody::ExecuteTool {
            call: review.digest,
        },
    );
    assert!(
        execution
            .execute(
                Arc::clone(kernel.context()),
                workspace.path().to_owned(),
                command,
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert!(
        verifier.validate_completion(&active.view()).await.is_err(),
        "later failure must prevent revival of older passed proof"
    );
    let shutdown = kernel.shutdown().await?;
    assert!(!shutdown.deadline_exceeded);
    assert!(shutdown.journal_error.is_none());
    Ok(())
}
