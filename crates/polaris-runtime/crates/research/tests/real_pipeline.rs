use bytes::Bytes;
use polaris_runtime_frames::{ToolCall, ToolResultRef, ToolRunner};
use polaris_runtime_research::{
    CheckpointStore, Direction, ExperimentWorkspace, LedgerKind, ModelUsage, ProgramMd,
    ResearchEdit, ResearchLoop, ScopedToolCall, StopCondition, ToolContext, VcsWorkspace,
};
use polaris_sdk::{
    contracts::Timestamp,
    ids::{Cid, FrameId, SessionId},
    registry::{ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use polaris_security_capability::{
    Authority, Authorization, Clock, Facts, ResourcePath, ResourceScope, Right, Rights, Scope,
    Token, authorize,
};
use polaris_storage_cas::content_id;
use std::{
    error::Error,
    future::Future,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};

type TestResult<T> = Result<T, Box<dyn Error>>;
const PROGRAM: &str = "---\ngoal: improve validation loss\nwall_ms: 120000\ntokens: 20\nmicro_usd: 20\nmax_experiments: 20\neditable: src/**\nreadonly: program.md,metrics/**\n---\n```metric\nname: validation_loss\ndirection: min\nformat: named\nharness: metrics/run.ps1\ncommand: powershell.exe -NoProfile -ExecutionPolicy Bypass -File metrics/run.ps1\n```\n";

struct TempTree(PathBuf);
impl TempTree {
    fn new() -> TestResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "polaris-research-e2e-{}-{}",
            std::process::id(),
            SessionId::new_v7()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempTree {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

struct AuthorizedWriter {
    public_key: polaris_security_capability::PublicKeyBytes,
    root: PathBuf,
    frame: FrameId,
    calls: Arc<AtomicUsize>,
}
struct AuthorizedCall {
    path: String,
    candidate: String,
}

impl ToolRunner for AuthorizedWriter {
    type Authorized = AuthorizedCall;
    type Error = SandboxError;
    async fn authorize(
        &self,
        token: &Token,
        call: ToolCall,
    ) -> Result<Self::Authorized, Self::Error> {
        let Some((target, candidate)) = parse_candidate(&call.arguments) else {
            return Err(SandboxError::PolicyDenied(
                "invalid fixture tool call".to_owned(),
            ));
        };
        if call.name != "write_candidate" {
            return Err(SandboxError::PolicyDenied(
                "invalid fixture tool call".to_owned(),
            ));
        }
        let path = ResourcePath::new(&target)
            .map_err(|_| SandboxError::PolicyDenied("invalid fixture path".to_owned()))?;
        let facts = Facts {
            resource: &path,
            symbol: None,
            churn_lines: 1,
            touches_unsafe: false,
            diff_blake3: None,
            right: Right::ExecuteTool,
        };
        match authorize(token, &facts, &self.public_key, &FixedClock)
            .map_err(|_| SandboxError::PolicyDenied("capability evaluation failed".to_owned()))?
        {
            Authorization::Allowed => Ok(AuthorizedCall {
                path: target,
                candidate,
            }),
            Authorization::Denied { .. } => Err(SandboxError::PolicyDenied(
                "capability denied fixture write".to_owned(),
            )),
        }
    }
    async fn execute(
        &self,
        permit: Self::Authorized,
        frame: FrameId,
        max_result_bytes: usize,
    ) -> Result<ToolResultRef, Self::Error> {
        if frame != self.frame || max_result_bytes == 0 {
            return Err(SandboxError::PolicyDenied(
                "invalid authorized frame".to_owned(),
            ));
        }
        let content = format!("{}\n", permit.candidate);
        std::fs::write(self.root.join(&permit.path), content.as_bytes())
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        if content.len() > max_result_bytes {
            return Err(SandboxError::LimitExceeded("tool result".to_owned()));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let cid = content_id(content.as_bytes());
        Ok(ToolResultRef {
            cid,
            content_cid: cid,
            content,
        })
    }
}

struct LocalProcessBackend {
    root: PathBuf,
}
impl SandboxBackend for LocalProcessBackend {
    async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        Err(SandboxError::PolicyDenied(
            "fixture does not create an isolated sandbox".to_owned(),
        ))
    }
    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        let root = self.root.clone();
        let request = request.clone();
        let (program, arguments) = request
            .argv
            .split_first()
            .ok_or_else(|| SandboxError::Protocol("empty metric command".to_owned()))?;
        let started = Instant::now();
        let mut child = Command::new(program.as_str())
            .args(arguments.iter().map(String::as_str))
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| SandboxError::Unavailable(error.to_string()))?;
        let timeout = match request.timeout {
            Some(timeout) if timeout.is_zero() => {
                return Err(SandboxError::PolicyDenied("zero metric timeout".to_owned()));
            }
            Some(timeout) => timeout,
            None => Duration::from_secs(5),
        };
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let _killed = child.kill();
                    let _waited = child.wait();
                    return Err(SandboxError::Timeout);
                }
                Err(error) => return Err(SandboxError::Protocol(error.to_string())),
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let limit = match request.output_limit_bytes {
            Some(0) => {
                return Err(SandboxError::LimitExceeded(
                    "zero metric output limit".to_owned(),
                ));
            }
            Some(limit) => limit,
            None => 65_536,
        };
        if output
            .stdout
            .len()
            .checked_add(output.stderr.len())
            .is_none_or(|size| size > limit)
        {
            return Err(SandboxError::LimitExceeded("metric output".to_owned()));
        }
        let exit_code = if output.status.success() { 0 } else { 1 };
        Ok(ExecOutput {
            exit_code,
            stdout: Bytes::from(output.stdout),
            stderr: Bytes::from(output.stderr),
            duration: started.elapsed(),
        })
    }
    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
        _bytes: Bytes,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied(
            "fixture upload is unsupported".to_owned(),
        ))
    }
    async fn download(&self, _sandbox: &SandboxHandle, _path: &str) -> Result<Bytes, SandboxError> {
        Err(SandboxError::PolicyDenied(
            "fixture download is unsupported".to_owned(),
        ))
    }
    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Err(SandboxError::PolicyDenied(
            "fixture listing is unsupported".to_owned(),
        ))
    }
    async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied(
            "fixture destroy is unsupported".to_owned(),
        ))
    }
}

fn parse_candidate(value: &str) -> Option<(String, String)> {
    let (path, candidate) = value.split_once('=')?;
    if !path.starts_with("src/") {
        return None;
    }
    let suffix = candidate.strip_prefix("candidate-")?;
    let number = suffix.parse::<u8>().ok()?;
    (1..=20)
        .contains(&number)
        .then(|| (path.to_owned(), candidate.to_owned()))
}

fn git(root: &Path, args: &[&str]) -> TestResult<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git failed for {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn prepare_repository(root: &Path) -> TestResult<()> {
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::create_dir_all(root.join("metrics"))?;
    std::fs::write(root.join("src/value.txt"), "baseline\n")?;
    std::fs::write(root.join("program.md"), PROGRAM)?;
    let harness = r#"$content = Get-Content -Raw 'src/value.txt'
if ($content -notmatch '^candidate-(\d+)\s*$') { [Console]::Error.WriteLine('candidate missing'); exit 3 }
$index = [int]$Matches[1]
$scores = @(10, 9, 9, 11, 8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9)
Write-Output ("validation_loss=" + $scores[$index - 1])
"#;
    std::fs::write(root.join("metrics/run.ps1"), harness)?;
    git(root, &["init", "--quiet"])?;
    git(root, &["config", "user.name", "Research Acceptance"])?;
    git(
        root,
        &[
            "config",
            "user.email",
            "research-acceptance@example.invalid",
        ],
    )?;
    git(
        root,
        &["add", "src/value.txt", "program.md", "metrics/run.ps1"],
    )?;
    git(root, &["commit", "--quiet", "-m", "acceptance baseline"])?;
    Ok(())
}

fn signed_context(root: &Path) -> TestResult<(ToolContext, AuthorizedWriter, Arc<AtomicUsize>)> {
    let frame = FrameId::new_v7();
    let scope = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("src")?),
        max_churn_lines: 2,
        ttl: Duration::from_secs(3_600),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::ExecuteTool]),
    };
    let authority = Authority::new_session()?;
    let token = authority.issue_root(frame, &scope, &FixedClock)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let tool = AuthorizedWriter {
        public_key: authority.public_key(),
        root: root.to_path_buf(),
        frame,
        calls: Arc::clone(&calls),
    };
    Ok((ToolContext { token, frame }, tool, calls))
}

fn edit(index: u8, context: &ToolContext) -> ResearchEdit {
    ResearchEdit {
        hypothesis: format!("try candidate {index}").into_boxed_str(),
        calls: vec![ScopedToolCall {
            path: "src/value.txt".into(),
            call: ToolCall {
                id: format!("candidate-{index}"),
                name: "write_candidate".to_owned(),
                arguments: format!("src/value.txt=candidate-{index}"),
            },
        }],
        tool_context: Some(context.clone()),
        model_usage: ModelUsage {
            tokens: 1,
            micro_usd: 1,
        },
    }
}

#[test]
fn tool_cannot_write_outside_the_exact_vcs_snapshot_path() -> TestResult<()> {
    let temp = TempTree::new()?;
    let repo = temp.path().join("repo");
    prepare_repository(&repo)?;
    let program = ProgramMd::parse(PROGRAM)?;
    let (context, tools, calls) = signed_context(&repo)?;
    let mut research = ResearchLoop::new(
        program,
        VcsWorkspace::open(&repo)?,
        LocalProcessBackend { root: repo.clone() },
        SandboxHandle::new("local-process-fixture")?,
        tools,
        CheckpointStore::open(&temp.path().join("store"), ProgramMd::parse(PROGRAM)?.cid())?,
    )?;
    let mut proposal = edit(1, &context);
    if let Some(call) = proposal.calls.first_mut() {
        call.call.arguments = "src/other.txt=candidate-1".to_owned();
    } else {
        return Err("fixture did not construct a tool call".into());
    }
    let result = block_on(research.step(proposal))?;
    assert_eq!(
        result.status,
        polaris_runtime_research::IterationStatus::Recovered(
            polaris_runtime_research::FailureReason::Tool,
        )
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_to_string(repo.join("src/value.txt"))?,
        "baseline\n"
    );
    assert!(!repo.join("src/other.txt").exists());
    Ok(())
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

#[test]
fn twenty_iterations_keep_only_strict_improvements_and_reopen_evidence() -> TestResult<()> {
    let temp = TempTree::new()?;
    let repo = temp.path().join("repo");
    let store_root = temp.path().join("store");
    prepare_repository(&repo)?;
    let program = ProgramMd::parse(PROGRAM)?;
    let (context, tools, calls) = signed_context(&repo)?;
    let research = ResearchLoop::new(
        program,
        VcsWorkspace::open(&repo)?,
        LocalProcessBackend { root: repo.clone() },
        SandboxHandle::new("local-process-fixture")?,
        tools,
        CheckpointStore::open(&store_root, ProgramMd::parse(PROGRAM)?.cid())?,
    )?;
    let mut research = research;
    let plans = (1..=20)
        .map(|index| edit(index, &context))
        .collect::<Vec<_>>();
    let summary = block_on(research.run_until(
        plans,
        StopCondition {
            max_iterations: 20,
            plateau_patience: None,
        },
    ))?;
    assert_eq!(summary.iterations, 20);
    assert_eq!(research.status().completed, 20);
    assert_eq!(research.status().best.map(|value| value.get()), Some(8.0));
    assert_eq!(calls.load(Ordering::SeqCst), 20);
    assert_eq!(
        std::fs::read_to_string(repo.join("src/value.txt"))?,
        "candidate-5\n"
    );
    let report = research.report();
    assert_eq!(report.matches("decision=Keep").count(), 3);
    assert_eq!(report.matches("decision=Revert").count(), 17);
    assert_eq!(report.matches("kind=Tool").count(), 20);
    assert_eq!(report.matches("kind=MetricRun").count(), 20);
    assert_eq!(report.matches("kind=Mutation").count(), 20);
    drop(research);
    let reopened = CheckpointStore::open(&store_root, ProgramMd::parse(PROGRAM)?.cid())?;
    let entries = reopened.ledger()?;
    assert_eq!(entries.len(), 60);
    for entry in entries {
        let payload = reopened.get_evidence(entry.evidence)?;
        assert!(!payload.is_empty());
        match entry.kind {
            LedgerKind::Tool => assert!(payload.starts_with(b"POLARIS-RESEARCH-TOOL\x01")),
            LedgerKind::MetricRun => assert!(payload.starts_with(b"POLARIS-RESEARCH-RUN\x01")),
            LedgerKind::Mutation | LedgerKind::Recovery => {
                assert!(payload.starts_with(b"POLARIS-RESEARCH-MUTATION\x01"));
            }
        }
    }
    Ok(())
}

#[test]
fn abrupt_child_exit_reopens_as_recovery_and_restores_baseline() -> TestResult<()> {
    let temp = TempTree::new()?;
    let repo = temp.path().join("repo");
    let store_root = temp.path().join("store");
    prepare_repository(&repo)?;
    let status = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("abrupt_child_exit_helper")
        .arg("--nocapture")
        .env("POLARIS_RESEARCH_CRASH_REPO", &repo)
        .env("POLARIS_RESEARCH_CRASH_STORE", &store_root)
        .status()?;
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(repo.join("src/value.txt"))?,
        "orphan\n"
    );
    let program = ProgramMd::parse(PROGRAM)?;
    let (context, tools, _) = signed_context(&repo)?;
    let mut resumed = ResearchLoop::resume(
        program,
        VcsWorkspace::open(&repo)?,
        LocalProcessBackend { root: repo.clone() },
        SandboxHandle::new("local-process-fixture")?,
        tools,
        CheckpointStore::open(&store_root, ProgramMd::parse(PROGRAM)?.cid())?,
    )?;
    assert_eq!(
        resumed.status().recovery_required.map(|id| id.get()),
        Some(1)
    );
    let receipt = resumed.recover_pending()?;
    assert_ne!(receipt, Cid::from_bytes([0; 32]));
    assert_eq!(resumed.status().recovery_required, None);
    assert_eq!(
        std::fs::read_to_string(repo.join("src/value.txt"))?,
        "baseline\n"
    );
    assert!(resumed.report().contains("kind=Recovery"));
    let _ = context;
    Ok(())
}

#[test]
fn abrupt_child_exit_helper() -> TestResult<()> {
    let Ok(repo) = std::env::var("POLARIS_RESEARCH_CRASH_REPO") else {
        return Ok(());
    };
    let store_root = PathBuf::from(std::env::var("POLARIS_RESEARCH_CRASH_STORE")?);
    let repo = PathBuf::from(repo);
    let program = ProgramMd::parse(PROGRAM)?;
    let mut state = polaris_runtime_research::ResearchState::new(
        program.cid(),
        Direction::Min,
        program.budget(),
    )?;
    let id = state.begin(polaris_runtime_research::Usage::new(6_000, 1, 1))?;
    CheckpointStore::open(&store_root, program.cid())?.save(&state)?;
    VcsWorkspace::open(&repo)?.begin(id, &["src/value.txt".to_owned()])?;
    std::fs::write(repo.join("src/value.txt"), "orphan\n")?;
    std::process::exit(0);
}
