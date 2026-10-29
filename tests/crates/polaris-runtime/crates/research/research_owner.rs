use polaris_runtime_frames::{ToolCall, ToolResultRef, ToolRunner};
use polaris_runtime_research::{
    CheckpointStore, ExperimentWorkspace, LoopError, ProgramMd, ResearchLoop,
};
use polaris_sdk::{
    ids::FrameId,
    registry::{ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use polaris_security_capability::Token;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

type TestResult<T> = Result<T, Box<dyn Error>>;

const PROGRAM: &str = "---\ngoal: verify exclusive ownership\nwall_ms: 1000\ntokens: 20\nmicro_usd: 20\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n```metric\nname: score\ndirection: min\nformat: named\nharness: metrics/run.ps1\ncommand: powershell.exe -NoProfile -File metrics/run.ps1\n```\n";

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> TestResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "polaris-research-owner-{}-{}",
            std::process::id(),
            polaris_sdk::ids::SessionId::new_v7()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
struct EmptyWorkspace;

impl ExperimentWorkspace for EmptyWorkspace {
    fn begin(
        &self,
        _id: polaris_runtime_research::ExperimentId,
        _paths: &[String],
    ) -> Result<(), polaris_runtime_research::WorkspaceError> {
        Ok(())
    }

    fn settle(
        &self,
        _id: polaris_runtime_research::ExperimentId,
        _decision: polaris_runtime_research::Decision,
    ) -> Result<polaris_runtime_research::WorkspaceReceipt, polaris_runtime_research::WorkspaceError>
    {
        Err(polaris_runtime_research::WorkspaceError::Receipt)
    }

    fn recover(
        &self,
        _id: polaris_runtime_research::ExperimentId,
    ) -> Result<polaris_runtime_research::WorkspaceReceipt, polaris_runtime_research::WorkspaceError>
    {
        Err(polaris_runtime_research::WorkspaceError::Receipt)
    }
}

#[derive(Clone, Copy)]
struct EmptySandbox;

impl SandboxBackend for EmptySandbox {
    async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        _request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
        _bytes: bytes::Bytes,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn download(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
    ) -> Result<bytes::Bytes, SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }
}

#[derive(Clone, Copy)]
struct EmptyTools;

impl ToolRunner for EmptyTools {
    type Authorized = ();
    type Error = SandboxError;

    async fn authorize(
        &self,
        _token: &Token,
        _call: ToolCall,
    ) -> Result<Self::Authorized, Self::Error> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }

    async fn execute(
        &self,
        _permit: Self::Authorized,
        _frame: FrameId,
        _max_result_bytes: usize,
    ) -> Result<ToolResultRef, Self::Error> {
        Err(SandboxError::PolicyDenied("unused fixture".to_owned()))
    }
}

fn construct(
    store_root: &Path,
    resume: bool,
) -> TestResult<ResearchLoop<EmptyWorkspace, EmptySandbox, EmptyTools>> {
    let program = ProgramMd::parse(PROGRAM)?;
    let store = CheckpointStore::open(store_root, program.cid())?;
    let sandbox_handle = SandboxHandle::new("owner-fixture").map_err(|_| LoopError::Edit)?;
    if resume {
        Ok(ResearchLoop::resume(
            program,
            EmptyWorkspace,
            EmptySandbox,
            sandbox_handle,
            EmptyTools,
            store,
        )?)
    } else {
        Ok(ResearchLoop::new(
            program,
            EmptyWorkspace,
            EmptySandbox,
            sandbox_handle,
            EmptyTools,
            store,
        )?)
    }
}

#[test]
fn second_loop_owner_is_rejected_while_first_is_alive() -> TestResult<()> {
    let temp = TempTree::new()?;
    let root = temp.path().join("store");
    let first = construct(&root, false)?;
    let second = match construct(&root, true) {
        Ok(_) => return Err("second run owner was unexpectedly accepted".into()),
        Err(error) => error,
    };
    assert!(second.to_string().contains("already active"));
    let reader = CheckpointStore::open(&root, ProgramMd::parse(PROGRAM)?.cid())?;
    assert!(reader.load()?.is_some());
    drop(first);
    let resumed = construct(&root, true)?;
    drop(resumed);
    Ok(())
}

#[test]
fn fresh_constructor_refuses_existing_checkpoint() -> TestResult<()> {
    let temp = TempTree::new()?;
    let root = temp.path().join("store");
    let first = construct(&root, false)?;
    drop(first);
    let fresh = match construct(&root, false) {
        Ok(_) => return Err("fresh constructor overwrote an existing checkpoint".into()),
        Err(error) => error,
    };
    assert!(fresh.to_string().contains("checkpoint already exists"));
    Ok(())
}

#[test]
fn resume_can_acquire_owner_after_previous_loop_drops() -> TestResult<()> {
    let temp = TempTree::new()?;
    let root = temp.path().join("store");
    let first = construct(&root, false)?;
    drop(first);
    let resumed = construct(&root, true)?;
    drop(resumed);
    Ok(())
}

#[test]
fn abrupt_process_termination_releases_run_owner() -> TestResult<()> {
    let temp = TempTree::new()?;
    let root = temp.path().join("store");
    let ready = temp.path().join("owner-ready");
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("abrupt_owner_child_helper")
        .arg("--nocapture")
        .env("POLARIS_RESEARCH_OWNER_STORE", &root)
        .env("POLARIS_RESEARCH_OWNER_READY", &ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    while !ready.exists() && started.elapsed() < Duration::from_secs(10) {
        if let Some(status) = child.try_wait()? {
            return Err(format!("owner helper exited early: {status}").into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.exists(),
        "child did not acquire and announce ownership"
    );
    let concurrent = construct(&root, true);
    let denied_while_child_active = match concurrent {
        Ok(owner) => {
            drop(owner);
            false
        }
        Err(error) => error.to_string().contains("already active"),
    };
    child.kill()?;
    let status = child.wait()?;
    assert!(!status.success());
    assert!(denied_while_child_active);
    let resumed = construct(&root, true)?;
    drop(resumed);
    Ok(())
}

#[test]
fn abrupt_owner_child_helper() -> TestResult<()> {
    let (Ok(root), Ok(ready)) = (
        std::env::var("POLARIS_RESEARCH_OWNER_STORE"),
        std::env::var("POLARIS_RESEARCH_OWNER_READY"),
    ) else {
        return Ok(());
    };
    let owner = construct(Path::new(&root), false)?;
    fs::write(ready, b"owned")?;
    thread::sleep(Duration::from_secs(60));
    drop(owner);
    Ok(())
}
