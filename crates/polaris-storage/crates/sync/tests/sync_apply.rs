//! Sync Apply (descriptor: specs/descriptors/sync_apply.json). The sandbox is an in-memory fake of
//! the sdk `SandboxBackend` (never shipped).

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_storage_sync::{
    ApplyOpts, HashMode, IgnoreRules, Manifest, RelPath, apply, plan, scan,
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Default)]
struct FakeSandbox {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    executable: Mutex<BTreeSet<String>>,
    fail_uploads: Mutex<BTreeSet<String>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, SandboxError> {
    m.lock()
        .map_err(|_| SandboxError::Protocol("poisoned".into()))
}

impl SandboxBackend for FakeSandbox {
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
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        if lock(&self.fail_uploads)?.contains(path) {
            return Err(SandboxError::Unavailable("injected failure".into()));
        }
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
async fn apply_sets_exec_bit_via_chmod() -> TestResult {
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
    lock(&fake.fail_uploads)?.insert("/sandbox/ws/src/lib.rs".into());
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
    assert_eq!(report.uploaded.len(), 2);
    let remote = report.applied_to(&remote, &local);
    let again = plan(&local, &remote);
    assert_eq!(
        again.upload,
        vec![RelPath::new("src/lib.rs")?],
        "only the failed path is retried"
    );
    lock(&fake.fail_uploads)?.clear();
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
    assert!((2..=3).contains(&max), "max in flight {max}");
    Ok(())
}
