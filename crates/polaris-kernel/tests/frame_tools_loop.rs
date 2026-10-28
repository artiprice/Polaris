//! Real frame-loop integration over a temporary workspace and the production kernel tool runner.
//! The scripted chat model is a synthetic test provider; no live endpoint is contacted.
use polaris_kernel::{Kernel, KernelHandle, KernelToolRunner, LocalKernelConfig};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, AssembleRequest, Cancellation, FramePrompt, FrameScope,
    ModelSelection, ResourceQuota, ScratchpadRef, StepInput, StepOutcome, StepTool, StepVerifier,
    Verification, run_step,
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FastFingerprint, FrameId, ModelId, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, Effects, FinishReason,
        FunctionalOperator, ModelError, ToolSpec,
    },
};
use polaris_security::capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock,
};
use std::{
    collections::VecDeque,
    error::Error,
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: KernelHandle,
    authority: Authority,
    parent: AgentContextFrame<Active>,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(
            workspace.path().join("sample.txt"),
            "synthetic root bytes\n",
        )?;
        std::fs::create_dir(workspace.path().join("src"))?;
        std::fs::write(
            workspace.path().join("src/secret.txt"),
            "synthetic child bytes\n",
        )?;

        let frame_id = FrameId::new_v7();
        let session_id = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session_id,
            frame_id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read, Right::ExecuteTool]),
        };
        let token = authority.issue_root(frame_id, &scope, &SystemClock)?;
        let expires_at_micros = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("frame expiry overflow")?;
        let parent = AgentContextFrame::new_root(
            frame_id,
            AgentRole::Orchestrator,
            token,
            FrameScope {
                capability: scope,
                expires_at_micros,
            },
            quota(),
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new("test/synthetic-provider")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        Ok(Self {
            _root: root,
            workspace,
            kernel,
            authority,
            parent,
        })
    }

    fn runner(
        &self,
        frame: &AgentContextFrame<Active>,
    ) -> Result<KernelToolRunner, polaris_sdk::registry::ToolError> {
        KernelToolRunner::new(
            self.kernel.clone(),
            frame.tool_binding(),
            self.authority.public_key(),
            CancellationToken::new(),
        )
    }

    fn child(&mut self) -> TestResult<AgentContextFrame<Active>> {
        let scope = Scope {
            resource: ResourceScope::Exact(ResourcePath::new("src/secret.txt")?),
            max_churn_lines: 0,
            ttl: Duration::from_secs(120),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read]),
        };
        let child_quota = ResourceQuota {
            cpu_millis: 1_000,
            memory_mb: 64,
            network: Default::default(),
            max_steps: 3,
            max_depth: 2,
            token_budget: 300,
        };
        let spawned = self.parent.spawn_child(
            AgentRole::SubAgent,
            scope,
            child_quota,
            ScratchpadRef::new(Cid::from_bytes([2; 32])),
            &SystemClock,
        )?;
        Ok(spawned.activate(
            ModelSelection {
                model: ModelId::new("test/synthetic-provider")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?)
    }
}

fn quota() -> ResourceQuota {
    ResourceQuota {
        cpu_millis: 10_000,
        memory_mb: 128,
        network: Default::default(),
        max_steps: 8,
        max_depth: 3,
        token_budget: 1_000,
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn cas_blob_count(root: &std::path::Path) -> TestResult<usize> {
    let mut pending = vec![root.to_owned()];
    let mut files = 0_usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files = files.checked_add(1).ok_or("CAS file count overflow")?;
            }
        }
    }
    Ok(files)
}

struct ReadAssembler;
impl FunctionalOperator for ReadAssembler {
    type In = AssembleRequest;
    type Out = FramePrompt;
    type Err = polaris_runtime::frames::FrameError;

    async fn apply(&self, _input: AssembleRequest) -> Result<FramePrompt, Self::Err> {
        Ok(FramePrompt {
            messages: vec![ChatMessage::text(
                polaris_sdk::registry::ChatRole::User,
                "Inspect the requested file".to_owned(),
            )],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read a workspace file".into(),
                input_schema_json: "{}".into(),
                effects: Effects::READ_ONLY,
            }],
            // Protocol fixture bound only; this test does not provide a production tokenizer.
            input_token_upper_bound: TokenCount::new(20),
            context_token_budget: TokenCount::new(4096),
            max_output_tokens: Some(TokenCount::new(40)),
        })
    }
}

struct SyntheticModel {
    chunks: Vec<ChatChunk>,
    calls: std::sync::Arc<Mutex<Vec<ChatRequest>>>,
}

struct SyntheticStream(VecDeque<ChatChunk>);
impl ChatChunkSource for SyntheticStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.pop_front().map(Ok)
    }
}

impl ChatModel for SyntheticModel {
    type Stream = SyntheticStream;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        lock_recover(&self.calls).push(request);
        Ok(SyntheticStream(self.chunks.clone().into()))
    }
}

struct Accept;
impl StepVerifier for Accept {
    type Error = polaris_runtime::frames::FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime::frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, Self::Error> {
        Ok(Verification::Accepted {
            // Protocol fixture value only; this test does not perform independent verification.
            fingerprint: FastFingerprint::new(7),
            verified_diffs: Vec::new(),
        })
    }
}

fn model(
    calls: std::sync::Arc<Mutex<Vec<ChatRequest>>>,
    paths: &[&str],
) -> TestResult<SyntheticModel> {
    let mut chunks = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let index = u32::try_from(index)?;
        let call_id = format!("synthetic-call-{index}");
        let arguments = serde_json::json!({"path": path}).to_string();
        chunks.push(ChatChunk::ToolCallDelta {
            index,
            id: Some(call_id),
            name: Some("read_file".to_owned()),
            arguments_fragment: arguments,
        });
    }
    chunks.push(ChatChunk::Finished(FinishReason::ToolCalls));
    Ok(SyntheticModel { chunks, calls })
}

async fn read_step(
    frame: &mut AgentContextFrame<Active>,
    tools: &KernelToolRunner,
    paths: &[&str],
) -> TestResult<(
    Result<polaris_runtime::frames::StepExecution, polaris_runtime::frames::FrameError>,
    Vec<polaris_runtime::frames::ExecutedToolResult>,
    usize,
)> {
    let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
    let chat = model(std::sync::Arc::clone(&calls), paths)?;
    let mut request_capture = None;
    let mut executed = Vec::new();
    let mut assistant_capture = None;
    let cancellation = Cancellation::new();
    let result = run_step(
        StepInput {
            frame,
            goal: "inspect files",
            prior_messages: Vec::new(),
            request_capture: &mut request_capture,
            executed_tool_results_capture: &mut executed,
            assistant_response_capture: &mut assistant_capture,
            cancellation: &cancellation,
            max_goal_bytes: 128,
            max_prompt_bytes: 2_048,
            max_output_bytes: 2_048,
            max_tool_calls: 4,
            max_tool_argument_bytes: 256,
            max_tool_result_bytes: 4_096,
            max_stream_chunks: 16,
        },
        &ReadAssembler,
        &chat,
        tools,
        &Accept,
    )
    .await;
    Ok((result, executed, lock_recover(&calls).len()))
}

#[tokio::test]
async fn child_loop_reads_only_its_exact_path_and_parent_scope_remains_available() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let mut child = fixture.child()?;
    let child_runner = fixture.runner(&child)?;
    let (child_result, child_executed, child_model_calls) =
        read_step(&mut child, &child_runner, &["src/secret.txt"]).await?;
    let execution = match child_result {
        Ok(execution) => execution,
        Err(error) => return Err(format!("child exact-path read failed: {error}").into()),
    };
    if child_model_calls != 1 || child_executed.len() != 1 {
        return Err("child run_step did not dispatch once and execute one authorized read".into());
    }
    match execution.outcome {
        StepOutcome::ToolResults { results, .. }
            if results.first().is_some_and(|result| {
                result.tool_call_id == "synthetic-call-0"
                    && result.result.content.contains("synthetic child bytes")
            }) => {}
        _ => return Err("child run_step did not return its actual file result".into()),
    }

    let parent_runner = fixture.runner(&fixture.parent)?;
    let (parent_result, parent_executed, parent_model_calls) =
        read_step(&mut fixture.parent, &parent_runner, &["sample.txt"]).await?;
    let parent_execution = match parent_result {
        Ok(execution) => execution,
        Err(error) => return Err(format!("parent workspace read failed: {error}").into()),
    };
    if parent_model_calls != 1 || parent_executed.len() != 1 {
        return Err("parent scope did not retain its wider workspace read".into());
    }
    match parent_execution.outcome {
        StepOutcome::ToolResults { results, .. }
            if results
                .first()
                .is_some_and(|result| result.result.content.contains("synthetic root bytes")) => {}
        _ => return Err("parent runner lost access to its authorized root file".into()),
    }

    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn run_step_authorizes_all_calls_before_executing_any_child_call() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let mut child = fixture.child()?;
    let child_runner = fixture.runner(&child)?;
    let cas_before = cas_blob_count(&fixture._root.path().join("cas"))?;
    let (result, executed, model_calls) =
        read_step(&mut child, &child_runner, &["src/secret.txt", "sample.txt"]).await?;
    match result {
        Err(polaris_runtime::frames::FrameError::Backend(_, "tool authorization")) => {}
        Err(error) => return Err(format!("unexpected run_step error: {error}").into()),
        Ok(_) => return Err("child capability unexpectedly authorized a sibling path".into()),
    }
    if model_calls != 1 {
        return Err("synthetic model response was not dispatched exactly once".into());
    }
    if !executed.is_empty() {
        return Err("run_step executed the valid first call before denying the second".into());
    }
    let cas_after = cas_blob_count(&fixture._root.path().join("cas"))?;
    if cas_after != cas_before {
        return Err("denied tool batch published a CAS result before full authorization".into());
    }
    if std::fs::read_to_string(fixture.workspace.path().join("src/secret.txt"))?
        != "synthetic child bytes\n"
    {
        return Err("denied response changed the child file".into());
    }
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
