//! Explicitly fake transport contract tests; live gateway acceptance has a separate gate.
use polaris_kernel::{ExecutionRequest, Kernel, LocalKernelConfig, SandboxExecution};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
    registry::{
        CommitOutcome, ExecOutput, ExecRequest, SandboxBackend, SandboxCommitAction,
        SandboxCommitRequest, SandboxError, SandboxFileState, SandboxHandle, SandboxSnapshotEntry,
        SandboxSnapshotPage, SandboxSpec,
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
        self.files.lock().expect("files").clear();
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
    assert_eq!(backend.destroys.load(Ordering::SeqCst), 1);
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
