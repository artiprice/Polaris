//! Multi-turn frame execution through the actual kernel tool runner and durable journal.
//! Chat model, assembler, and verifier are synthetic fixtures. The filesystem, CAS, Biscuit
//! capability checks, session, and durable event bus are the production local kernel.
use polaris_kernel::{
    Kernel, KernelFrameJournal, KernelHandle, KernelToolRunner, LocalKernelConfig,
};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, AssembleRequest, Cancellation, FramePrompt,
    FrameRunInput, FrameRunLimits, FrameRunStop, FrameScope, ModelSelection, ResourceQuota,
    ScratchpadRef, StepTool, StepVerifier, Verification, run_frame,
};
use polaris_sdk::{
    contracts::{Event, FrameStepOutcome},
    effort::Effort,
    ids::{Cid, FastFingerprint, FrameId, ModelId, Seq, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, Effects,
        FinishReason, FunctionalOperator, ModelError, ToolSpec,
    },
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
};
use std::{
    collections::VecDeque,
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const CALL_ID: &str = "provider-call-durable-17";
const FILE_BYTES: &str = "synthetic durable run source\n";

struct Fixture {
    _root: tempfile::TempDir,
    workspace: tempfile::TempDir,
    session: SessionId,
    kernel: KernelHandle,
    authority: Authority,
    frame: AgentContextFrame<Active>,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(workspace.path().join("sample.txt"), FILE_BYTES)?;
        let frame_id = FrameId::new_v7();
        let session = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
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
        let frame = AgentContextFrame::new_root(
            frame_id,
            AgentRole::Orchestrator,
            token,
            FrameScope {
                capability: scope,
                expires_at_micros,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 8,
                max_depth: 3,
                token_budget: 1_000,
            },
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
            session,
            kernel,
            authority,
            frame,
        })
    }

    fn runner(&self) -> Result<KernelToolRunner, polaris_sdk::registry::ToolError> {
        KernelToolRunner::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            self.authority.public_key(),
            CancellationToken::new(),
        )
    }

    fn journal(&self) -> Result<KernelFrameJournal, polaris_kernel::KernelFrameJournalError> {
        KernelFrameJournal::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            polaris_runtime::frames::FrameJournalLimits::default(),
        )
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct ScriptAssembler;
impl FunctionalOperator for ScriptAssembler {
    type In = AssembleRequest;
    type Out = FramePrompt;
    type Err = polaris_runtime::frames::FrameError;

    async fn apply(&self, input: AssembleRequest) -> Result<FramePrompt, Self::Err> {
        let mut messages = vec![ChatMessage::text(
            ChatRole::User,
            "Read sample.txt and report the contents".to_owned(),
        )];
        messages.extend(input.prior_messages);
        Ok(FramePrompt {
            messages,
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read a workspace file".into(),
                input_schema_json: "{}".into(),
                effects: Effects::READ_ONLY,
            }],
            // Explicit synthetic tokenizer upper bound; this fixture has no production tokenizer.
            input_token_upper_bound: TokenCount::new(32),
            context_token_budget: TokenCount::new(4096),
            max_output_tokens: Some(TokenCount::new(64)),
        })
    }
}

#[derive(Clone)]
enum Script {
    Chunks(Vec<ChatChunk>),
    Fail(ModelError),
}

struct ScriptedModel {
    scripts: Mutex<VecDeque<Script>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

struct ScriptedStream(VecDeque<ChatChunk>);
impl ChatChunkSource for ScriptedStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.pop_front().map(Ok)
    }
}

impl ChatModel for ScriptedModel {
    type Stream = ScriptedStream;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        lock_recover(&self.requests).push(request);
        match lock_recover(&self.scripts).pop_front() {
            Some(Script::Chunks(chunks)) => Ok(ScriptedStream(chunks.into())),
            Some(Script::Fail(error)) => Err(error),
            None => Err(ModelError::Protocol("script exhausted".to_owned())),
        }
    }
}

struct AcceptFixture;
impl StepVerifier for AcceptFixture {
    type Error = polaris_runtime::frames::FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime::frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, Self::Error> {
        Ok(Verification::Accepted {
            // Synthetic verifier fixture value; it is not independent verification evidence.
            fingerprint: FastFingerprint::new(7),
            verified_diffs: Vec::new(),
        })
    }
}

fn first_tool_turn() -> Vec<ChatChunk> {
    vec![
        ChatChunk::ToolCallDelta {
            index: 0,
            id: Some(CALL_ID.to_owned()),
            name: Some("read_file".to_owned()),
            arguments_fragment: "{\"path\":\"sample.txt\"}".to_owned(),
        },
        ChatChunk::Finished(FinishReason::ToolCalls),
    ]
}

fn ordinary_final_turn() -> Vec<ChatChunk> {
    vec![
        ChatChunk::Text("The file says: synthetic durable run source".to_owned()),
        ChatChunk::Finished(FinishReason::Stop),
    ]
}

fn run_limits() -> FrameRunLimits {
    FrameRunLimits {
        maximum_attempts: 3,
        maximum_retries: 0,
        maximum_tool_rounds: 2,
        max_goal_bytes: 128,
        max_prompt_bytes: 4_096,
        max_output_bytes: 4_096,
        max_tool_calls: 4,
        max_tool_argument_bytes: 512,
        max_tool_result_bytes: 8_192,
        max_stream_chunks: 16,
    }
}

async fn transcripts(
    fixture: &Fixture,
) -> TestResult<Vec<(u64, FrameStepOutcome, Cid, serde_json::Value)>> {
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let mut events = Vec::new();
    for record in &records {
        if let Event::FrameStepRecorded {
            frame,
            step,
            transcript,
            outcome,
        } = record.event()
        {
            if record.session() != fixture.session {
                return Err("frame event used a foreign session".into());
            }
            if *frame != fixture.frame.tool_binding().frame_id() {
                return Err("frame event used a foreign frame".into());
            }
            let expanded = fixture
                .kernel
                .invoke_tool(
                    "polaris_expand",
                    serde_json::json!({"reference": transcript.to_hex()}),
                )
                .await?;
            let text = expanded
                .value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .ok_or("durable transcript missing from actual CAS")?;
            if polaris_kernel::storage::cas::content_id(text.as_bytes()) != *transcript {
                return Err("transcript bytes do not hash to the event CID".into());
            }
            let payload: serde_json::Value = serde_json::from_str(text)?;
            let expected_step = u32::try_from(events.len())?
                .checked_add(1)
                .ok_or("step overflow")?;
            if *step != expected_step
                || payload.get("step").and_then(serde_json::Value::as_u64) != Some(u64::from(*step))
            {
                return Err(
                    "driver frame steps are not monotonic or do not match the transcript".into(),
                );
            }
            events.push((record.seq().get(), *outcome, *transcript, payload));
        }
    }
    Ok(events)
}

fn require(condition: bool, message: &'static str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn tool_content_is_exact(content: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => {
            value
                .get("value")
                .and_then(|value| value.get("content"))
                .and_then(serde_json::Value::as_str)
                == Some(FILE_BYTES)
        }
        Err(_) => false,
    }
}

fn replay_history_is_exact(request: &ChatRequest) -> bool {
    let assistant_call = request.messages.iter().enumerate().find(|(_, message)| {
        message.role == ChatRole::Assistant
            && message.tool_calls.len() == 1
            && message.tool_calls.first().is_some_and(|call| {
                call.id == CALL_ID
                    && call.name == "read_file"
                    && call.arguments_json == "{\"path\":\"sample.txt\"}"
            })
    });
    let tool_result = request.messages.iter().enumerate().find(|(_, message)| {
        message.role == ChatRole::Tool && message.tool_call_id.as_deref() == Some(CALL_ID)
    });
    match (assistant_call, tool_result) {
        (Some((call_index, _)), Some((result_index, result))) => {
            call_index < result_index && tool_content_is_exact(&result.content)
        }
        _ => false,
    }
}

#[tokio::test]
async fn scripted_tool_turn_and_ordinary_final_turn_are_durably_journaled() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let model = ScriptedModel {
        scripts: Mutex::new(VecDeque::from([
            Script::Chunks(first_tool_turn()),
            Script::Chunks(ordinary_final_turn()),
        ])),
        requests: Arc::clone(&requests),
    };
    let runner = fixture.runner()?;
    let journal = fixture.journal()?;
    let result = run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "inspect sample.txt",
            cancellation: &Cancellation::new(),
            limits: run_limits(),
        },
        &ScriptAssembler,
        &model,
        &runner,
        &AcceptFixture,
        &journal,
    )
    .await?;
    require(
        result.stop == FrameRunStop::ModelTurnComplete
            && result.attempts == 2
            && result.tool_rounds == 1,
        "run_frame did not execute one tool turn followed by an ordinary final turn",
    )?;
    {
        let requests = lock_recover(&requests);
        require(
            requests.len() == 2,
            "synthetic model was not called exactly twice",
        )?;
        let submitted = requests.get(1).ok_or("second model request missing")?;
        require(
            replay_history_is_exact(submitted),
            "second provider request lost the exact call id or actual tool output",
        )?;
    }

    let events = transcripts(&fixture).await?;
    require(
        events.len() == 2,
        "actual event bus did not retain two frame steps",
    )?;
    let first = events.first().ok_or("first frame event missing")?;
    let second = events.get(1).ok_or("second frame event missing")?;
    require(
        first.0 < second.0
            && first.1 == FrameStepOutcome::Accepted
            && second.1 == FrameStepOutcome::Accepted,
        "durable event sequence or ordinary-step outcomes were not monotonic and accepted",
    )?;
    require(
        first
            .3
            .get("result")
            .and_then(|result| result.get("ToolResults"))
            .and_then(|result| result.get("results"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|results| {
                results.iter().any(|entry| {
                    entry
                        .get("tool_call_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(CALL_ID)
                        && entry
                            .get("result")
                            .and_then(|result| result.get("content"))
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(tool_content_is_exact)
                })
            }),
        "first durable transcript did not retain the provider call id and actual file output",
    )?;
    require(
        replay_history_is_exact(&serde_json::from_value::<ChatRequest>(
            second
                .3
                .get("request")
                .ok_or("second request missing")?
                .clone(),
        )?),
        "second durable transcript did not retain the replayed model request history",
    )?;
    require(
        std::fs::read_to_string(fixture.workspace.path().join("sample.txt"))? == FILE_BYTES,
        "readonly run changed the workspace source",
    )?;
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn second_model_turn_failure_is_journaled_as_failed_without_completion_claim() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let model = ScriptedModel {
        scripts: Mutex::new(VecDeque::from([
            Script::Chunks(first_tool_turn()),
            Script::Fail(ModelError::Transport(
                "synthetic second-turn failure".to_owned(),
            )),
        ])),
        requests: Arc::clone(&requests),
    };
    let runner = fixture.runner()?;
    let journal = fixture.journal()?;
    let result = run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "inspect sample.txt",
            cancellation: &Cancellation::new(),
            limits: run_limits(),
        },
        &ScriptAssembler,
        &model,
        &runner,
        &AcceptFixture,
        &journal,
    )
    .await;
    require(
        result.is_err(),
        "synthetic second model turn unexpectedly succeeded",
    )?;
    require(
        lock_recover(&requests).len() == 2,
        "provider failure did not occur on the second attempted model turn",
    )?;
    let events = transcripts(&fixture).await?;
    require(
        events.len() == 2,
        "actual event bus did not retain both success and failure frame records",
    )?;
    let first = events.first().ok_or("first frame event missing")?;
    let second = events.get(1).ok_or("second frame event missing")?;
    require(
        first.0 < second.0
            && first.1 == FrameStepOutcome::Accepted
            && second.1 == FrameStepOutcome::Failed,
        "second-turn failure was not durably distinct from the accepted tool turn",
    )?;
    require(
        second
            .3
            .get("request")
            .and_then(|request| request.get("messages"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message
                        .get("tool_call_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(CALL_ID)
                })
            }),
        "failure transcript did not retain the captured request with replayed tool history",
    )?;
    require(
        std::fs::read_to_string(fixture.workspace.path().join("sample.txt"))? == FILE_BYTES,
        "failed readonly run changed the workspace source",
    )?;
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
