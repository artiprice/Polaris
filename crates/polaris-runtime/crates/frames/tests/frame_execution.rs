use bytes::Bytes;
use polaris_runtime_frames::{
    Active, AgentContextFrame, AgentRole, AssembleRequest, Cancellation, ExecutedToolResult,
    FRAME_STEP_RECORD_SCHEMA_VERSION, FrameError, FrameJournal, FrameJournalError, FramePrompt,
    FrameRunInput, FrameRunLimits, FrameRunStop, FrameScope, FrameStepRecord, JournalAdapter,
    ModelSelection, ResourceQuota, ScratchpadRef, StepOutcome, StepTool, StepVerifier, ToolCall,
    ToolResultRef, ToolRunner, Verification, run_frame,
};
use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    contracts::{Event, EventRecord, FrameStepOutcome, Timestamp},
    effort::Effort,
    error::{ErrorKind, HasKind},
    ids::{Cid, FastFingerprint, FrameId, ModelId, Seq, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, ChatToolCall,
        Effects, EventStore, EventStoreError, FinishReason, FunctionalOperator, ModelError,
        ToolSpec, Usage,
    },
};
use polaris_security_capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, Token,
};
use polaris_storage_cas::{Cas, CasConfig};
use polaris_storage_eventlog::{EventLog, EventLogConfig};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "polaris-frame-journal-{}-{}",
            std::process::id(),
            FrameId::new_v7()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

fn frame() -> Result<AgentContextFrame<Active>, Box<dyn Error>> {
    let id = FrameId::new_v7();
    let scope = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("workspace")?),
        max_churn_lines: 20,
        ttl: Duration::from_secs(3_600),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read, Right::ExecuteTool]),
    };
    let authority = Authority::new_session()?;
    let token = authority.issue_root(id, &scope, &TestClock)?;
    let spawned = AgentContextFrame::new_root(
        id,
        AgentRole::Orchestrator,
        token,
        FrameScope {
            capability: scope,
            expires_at_micros: 3_600_000_000,
        },
        ResourceQuota {
            cpu_millis: 10_000,
            memory_mb: 1_024,
            network: Default::default(),
            max_steps: 8,
            max_depth: 3,
            token_budget: 1_000,
        },
        ScratchpadRef::new(Cid::from_bytes([7; 32])),
    );
    let active = spawned.activate(
        ModelSelection {
            model: ModelId::new("test/model")?,
            maximum_effort: Effort::Low,
        },
        Effort::Low,
    )?;
    Ok(active)
}

struct Assembler {
    seen: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
}
impl FunctionalOperator for Assembler {
    type In = AssembleRequest;
    type Out = FramePrompt;
    type Err = FrameError;

    async fn apply(&self, input: AssembleRequest) -> Result<FramePrompt, FrameError> {
        lock_recover(&self.seen).push(input.prior_messages.clone());
        let mut messages = vec![ChatMessage::text(ChatRole::System, "policy".to_owned())];
        messages.extend(input.prior_messages);
        messages.push(ChatMessage::text(ChatRole::User, input.goal));
        Ok(FramePrompt {
            messages,
            tools: vec![ToolSpec {
                name: "read_workspace".into(),
                description: "Read the workspace".into(),
                input_schema_json: "{}".into(),
                effects: Effects::READ_ONLY,
            }],
            input_token_upper_bound: TokenCount::new(20),
            max_output_tokens: Some(TokenCount::new(40)),
        })
    }
}

struct FixedAssembler {
    messages: Vec<ChatMessage>,
}

impl FunctionalOperator for FixedAssembler {
    type In = AssembleRequest;
    type Out = FramePrompt;
    type Err = FrameError;

    async fn apply(&self, _input: AssembleRequest) -> Result<FramePrompt, FrameError> {
        Ok(FramePrompt {
            messages: self.messages.clone(),
            tools: Vec::new(),
            input_token_upper_bound: TokenCount::new(20),
            max_output_tokens: Some(TokenCount::new(40)),
        })
    }
}

struct ScriptedModel {
    scripts: Mutex<VecDeque<Vec<ChatChunk>>>,
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
        let script = lock_recover(&self.scripts)
            .pop_front()
            .ok_or_else(|| ModelError::Protocol("script exhausted".to_owned()))?;
        Ok(ScriptedStream(script.into()))
    }
}

struct Tools {
    result: ToolResultRef,
    calls: Arc<Mutex<Vec<ToolCall>>>,
    result_limits: Arc<Mutex<Vec<usize>>>,
    cancel_after_first: Option<Cancellation>,
}
impl ToolRunner for Tools {
    type Authorized = ToolCall;
    type Error = FrameError;

    async fn authorize(&self, _token: &Token, call: ToolCall) -> Result<ToolCall, FrameError> {
        lock_recover(&self.calls).push(call.clone());
        Ok(call)
    }

    async fn execute(
        &self,
        _permit: ToolCall,
        _frame: FrameId,
        max_result_bytes: usize,
    ) -> Result<ToolResultRef, FrameError> {
        lock_recover(&self.result_limits).push(max_result_bytes);
        if let Some(cancellation) = &self.cancel_after_first {
            cancellation.cancel();
        }
        Ok(self.result.clone())
    }
}

struct Accept;
impl StepVerifier for Accept {
    type Error = FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime_frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, FrameError> {
        Ok(Verification::Accepted {
            fingerprint: FastFingerprint::new(33),
            verified_diffs: Vec::new(),
        })
    }
}

struct AcceptDiff(Cid);
impl StepVerifier for AcceptDiff {
    type Error = FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime_frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, FrameError> {
        Ok(Verification::Accepted {
            fingerprint: FastFingerprint::new(34),
            verified_diffs: vec![self.0],
        })
    }
}

struct ClaimDone;
impl StepVerifier for ClaimDone {
    type Error = FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime_frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, FrameError> {
        Ok(Verification::ClaimedDone)
    }
}

struct Reject;
impl StepVerifier for Reject {
    type Error = FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime_frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, FrameError> {
        Ok(Verification::Rejected)
    }
}

struct BlockingModel {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    cancellation: Cancellation,
    finish_before_block: bool,
}
struct BlockingStream {
    cancellation: Cancellation,
    sent_partial_call: bool,
    sent_finish: bool,
    finish_before_block: bool,
}
impl ChatChunkSource for BlockingStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        if !self.sent_partial_call {
            self.sent_partial_call = true;
            return Some(Ok(ChatChunk::ToolCallDelta {
                index: 0,
                id: Some("call-partial".to_owned()),
                name: Some("read_workspace".to_owned()),
                arguments_fragment: "{\"path\":".to_owned(),
            }));
        }
        if self.finish_before_block && !self.sent_finish {
            self.sent_finish = true;
            self.cancellation.cancel();
            return Some(Ok(ChatChunk::Finished(FinishReason::ToolCalls)));
        }
        self.cancellation.cancelled().await;
        None
    }
}
impl ChatModel for BlockingModel {
    type Stream = BlockingStream;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        lock_recover(&self.requests).push(request);
        Ok(BlockingStream {
            cancellation: self.cancellation.clone(),
            sent_partial_call: false,
            sent_finish: false,
            finish_before_block: self.finish_before_block,
        })
    }
}

struct TestJournal(Mutex<Vec<FrameStepRecord>>);
impl FrameJournal for TestJournal {
    type Error = FrameError;

    async fn record(&self, record: &FrameStepRecord) -> Result<Cid, FrameError> {
        lock_recover(&self.0).push(record.clone());
        let bytes = serde_json::to_vec(record).map_err(|_| FrameError::Accounting)?;
        Ok(Cid::from_bytes(*blake3::hash(&bytes).as_bytes()))
    }
}

/// Test-only in-memory content store used to inspect the exact transcript bytes.
#[derive(Debug, Clone, Default)]
struct MemoryContentStore(Arc<Mutex<BTreeMap<Cid, Vec<u8>>>>);
impl ContentStore for MemoryContentStore {
    fn put(&self, content: &[u8]) -> Result<Cid, ContentStoreError> {
        let cid = Cid::from_bytes(*blake3::hash(content).as_bytes());
        lock_recover(&self.0).insert(cid, content.to_vec());
        Ok(cid)
    }

    fn get(&self, cid: &Cid, max_bytes: u64) -> Result<Bytes, ContentStoreError> {
        let content = lock_recover(&self.0)
            .get(cid)
            .cloned()
            .ok_or(ContentStoreError::NotFound(*cid))?;
        let size = u64::try_from(content.len()).map_err(|_| ContentStoreError::TooLarge {
            limit: max_bytes,
            size: u64::MAX,
        })?;
        if size > max_bytes {
            return Err(ContentStoreError::TooLarge {
                limit: max_bytes,
                size,
            });
        }
        Ok(Bytes::from(content))
    }

    fn contains(&self, cid: &Cid) -> Result<bool, ContentStoreError> {
        Ok(lock_recover(&self.0).contains_key(cid))
    }
}

/// Test-only event store that rejects any event whose transcript is not already in CAS.
#[derive(Debug, Clone)]
struct MemoryEventStore {
    content: MemoryContentStore,
    records: Arc<Mutex<Vec<EventRecord>>>,
}
impl EventStore for MemoryEventStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        if let Event::FrameStepRecorded { transcript, .. } = record.event()
            && self.content.get(transcript, u64::MAX).is_err()
        {
            return Err(EventStoreError::Unavailable(
                "frame transcript is absent".to_owned(),
            ));
        }
        let mut records = lock_recover(&self.records);
        records.push(*record);
        let sequence = u64::try_from(records.len())
            .map_err(|_| EventStoreError::Unavailable("test sequence overflow".to_owned()))?;
        Ok(Seq::new(sequence))
    }

    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(lock_recover(&self.records)
            .iter()
            .filter(|record| record.seq() >= from && record.seq() <= to)
            .copied()
            .collect())
    }
}

fn base_limits() -> FrameRunLimits {
    FrameRunLimits {
        maximum_attempts: 3,
        maximum_retries: 1,
        maximum_tool_rounds: 2,
        max_goal_bytes: 128,
        max_prompt_bytes: 2_048,
        max_output_bytes: 2_048,
        max_tool_calls: 4,
        max_tool_argument_bytes: 256,
        max_tool_result_bytes: 512,
        max_stream_chunks: 32,
    }
}

type MessageLog = Arc<Mutex<Vec<Vec<ChatMessage>>>>;
type RequestLog = Arc<Mutex<Vec<ChatRequest>>>;
type ToolCallLog = Arc<Mutex<Vec<ToolCall>>>;
type ResultLimitLog = Arc<Mutex<Vec<usize>>>;
type TestComponents = (
    Assembler,
    ScriptedModel,
    Tools,
    MessageLog,
    RequestLog,
    ToolCallLog,
    ResultLimitLog,
);

fn tool_script(finish: FinishReason) -> Vec<ChatChunk> {
    vec![
        ChatChunk::ToolCallDelta {
            index: 0,
            id: Some("call-1".to_owned()),
            name: Some("read_workspace".to_owned()),
            arguments_fragment: "{}".to_owned(),
        },
        ChatChunk::Finished(finish),
    ]
}

fn sample_result(content: &str) -> ToolResultRef {
    let digest = blake3::hash(content.as_bytes());
    let content_cid = Cid::from_bytes(*digest.as_bytes());
    ToolResultRef {
        cid: content_cid,
        content_cid,
        content: content.to_owned(),
    }
}

fn components(scripts: Vec<Vec<ChatChunk>>, result: ToolResultRef) -> TestComponents {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let result_limits = Arc::new(Mutex::new(Vec::new()));
    (
        Assembler {
            seen: Arc::clone(&seen),
        },
        ScriptedModel {
            scripts: Mutex::new(scripts.into()),
            requests: Arc::clone(&requests),
        },
        Tools {
            result,
            calls: Arc::clone(&calls),
            result_limits: Arc::clone(&result_limits),
            cancel_after_first: None,
        },
        seen,
        requests,
        calls,
        result_limits,
    )
}

#[tokio::test]
async fn oversized_prior_reasoning_and_tool_arguments_fail_before_model_dispatch()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let assistant = ChatMessage::assistant_calls(
        String::new(),
        vec![ChatToolCall {
            id: "call-oversized".to_owned(),
            name: "read_workspace".to_owned(),
            arguments_json: "a".repeat(512),
        }],
        Some("r".repeat(512)),
    );
    let assemble = FixedAssembler {
        messages: vec![assistant],
    };
    let (_, chat, tools, _, requests, _, _) = components(
        vec![vec![ChatChunk::Finished(FinishReason::Stop)]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.max_prompt_bytes = 256;
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect prior transcript",
            cancellation: &Cancellation::new(),
            limits,
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::OutputLimit)) || !lock_recover(&requests).is_empty() {
        return Err("oversized structured prompt data reached model dispatch".into());
    }
    Ok(())
}

#[tokio::test]
async fn empty_message_overhead_counts_toward_prompt_byte_limit() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let assemble = FixedAssembler {
        messages: vec![ChatMessage::text(ChatRole::User, String::new()); 16],
    };
    let (_, chat, tools, _, requests, _, _) = components(
        vec![vec![ChatChunk::Finished(FinishReason::Stop)]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.max_prompt_bytes = 64;
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect message overhead",
            cancellation: &Cancellation::new(),
            limits,
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::OutputLimit)) || !lock_recover(&requests).is_empty() {
        return Err("message structural overhead was excluded from prompt size".into());
    }
    Ok(())
}

#[tokio::test]
async fn usage_after_finish_is_retained_after_clean_end_of_stream() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("answer".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
            ChatChunk::Usage(Usage {
                prompt_tokens: TokenCount::new(20),
                completion_tokens: TokenCount::new(1),
                cached_tokens: TokenCount::new(0),
            }),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "answer",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await?;
    let records = lock_recover(&journal.0);
    let response = records
        .first()
        .and_then(|record| record.assistant_response.as_ref())
        .ok_or("assistant response was not journaled")?;
    if !response.complete
        || response.finish_reason != Some(FinishReason::Stop)
        || response.usage
            != Some(Usage {
                prompt_tokens: TokenCount::new(20),
                completion_tokens: TokenCount::new(1),
                cached_tokens: TokenCount::new(0),
            })
    {
        return Err("cleanly finished response did not retain trailing usage".into());
    }
    Ok(())
}

#[tokio::test]
async fn usage_before_finish_does_not_stop_stream_collection() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Usage(Usage {
                prompt_tokens: TokenCount::new(20),
                completion_tokens: TokenCount::new(1),
                cached_tokens: TokenCount::new(0),
            }),
            ChatChunk::Text("answer".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "answer",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await?;
    let records = lock_recover(&journal.0);
    let response = records
        .first()
        .and_then(|record| record.assistant_response.as_ref())
        .ok_or("assistant response was not journaled")?;
    if !response.complete
        || response.text != "answer"
        || response.usage
            != Some(Usage {
                prompt_tokens: TokenCount::new(20),
                completion_tokens: TokenCount::new(1),
                cached_tokens: TokenCount::new(0),
            })
    {
        return Err("usage before finish prematurely stopped stream collection".into());
    }
    Ok(())
}

#[tokio::test]
async fn duplicate_finish_is_rejected_and_captured_as_incomplete() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("answer".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
            ChatChunk::Finished(FinishReason::Stop),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "answer",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::InvalidStream)) {
        return Err("duplicate finish marker was accepted".into());
    }
    let records = lock_recover(&journal.0);
    let response = records
        .first()
        .and_then(|record| record.assistant_response.as_ref())
        .ok_or("invalid response was not journaled")?;
    if response.complete || response.text != "answer" {
        return Err("invalid stream capture was marked complete or lost prior text".into());
    }
    Ok(())
}

#[tokio::test]
async fn text_after_finish_is_rejected_and_captured_as_incomplete() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("answer".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
            ChatChunk::Text("late".to_owned()),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "answer",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::InvalidStream)) {
        return Err("post-finish text was accepted".into());
    }
    let records = lock_recover(&journal.0);
    let response = records
        .first()
        .and_then(|record| record.assistant_response.as_ref())
        .ok_or("invalid response was not journaled")?;
    if response.complete || response.text != "answer" {
        return Err("post-finish text was captured or response marked complete".into());
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_after_finish_before_eof_keeps_response_partial() -> Result<(), Box<dyn Error>>
{
    let mut frame = frame()?;
    let cancellation = Cancellation::new();
    let chat = BlockingModel {
        requests: Arc::new(Mutex::new(Vec::new())),
        cancellation: cancellation.clone(),
        finish_before_block: true,
    };
    let assemble = Assembler {
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let tools = Tools {
        result: sample_result("unused"),
        calls: Arc::new(Mutex::new(Vec::new())),
        result_limits: Arc::new(Mutex::new(Vec::new())),
        cancel_after_first: None,
    };
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &cancellation,
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::Cancelled)) {
        return Err("cancellation after finish was not observed before clean EOF".into());
    }
    let records = lock_recover(&journal.0);
    let response = records
        .first()
        .and_then(|record| record.assistant_response.as_ref())
        .ok_or("cancelled response was not journaled")?;
    if response.complete || response.finish_reason != Some(FinishReason::ToolCalls) {
        return Err("finish marker without clean EOF was marked complete".into());
    }
    Ok(())
}

#[tokio::test]
async fn tool_transcript_is_replayed_with_cas_reference_and_call_identity()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, seen, requests, tool_calls, _) = components(
        vec![
            tool_script(FinishReason::ToolCalls),
            vec![
                ChatChunk::Text("done".to_owned()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
        ],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let outcome = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await?;
    if outcome.stop != FrameRunStop::ModelTurnComplete
        || outcome.attempts != 2
        || outcome.tool_rounds != 1
    {
        return Err("unexpected bounded run outcome".into());
    }
    let calls = lock_recover(&tool_calls);
    if calls.len() != 1 || calls.first().map(|call| call.id.as_str()) != Some("call-1") {
        return Err("tool authorization did not preserve provider call identity".into());
    }
    let seen = lock_recover(&seen);
    let second = seen.get(1).ok_or("second assembly call was not observed")?;
    if second.len() != 2
        || second.first().map(|message| message.role) != Some(ChatRole::Assistant)
        || second
            .get(1)
            .and_then(|message| message.tool_call_id.as_deref())
            != Some("call-1")
    {
        return Err("assistant tool call and linked result were not replayed in order".into());
    }
    let requests = lock_recover(&requests);
    if requests.len() != 2 {
        return Err("expected exactly two model attempts".into());
    }
    if lock_recover(&journal.0).len() != 2 {
        return Err("each reserved step must have one durable journal record".into());
    }
    Ok(())
}

#[tokio::test]
async fn only_verifier_accepted_diff_references_enter_history() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let verified_diff = Cid::from_bytes([35; 32]);
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("verified patch".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect patch",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &AcceptDiff(verified_diff),
        &journal,
    )
    .await?;
    let timeline = frame.diff_timeline();
    if timeline.len() != 1
        || timeline.first().map(|entry| (entry.step.get(), entry.diff)) != Some((1, verified_diff))
    {
        return Err("accepted verifier diff reference was not recorded for its step".into());
    }
    Ok(())
}

#[tokio::test]
async fn claimed_done_remains_unproven_and_keeps_frame_active() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("all finished".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let outcome = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "finish task",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &ClaimDone,
        &journal,
    )
    .await?;
    if outcome.stop != FrameRunStop::ClaimedDone
        || !matches!(outcome.last_step, StepOutcome::ClaimedDone { .. })
        || !frame.diff_timeline().is_empty()
        || frame.steps() != 1
    {
        return Err("completion claim was promoted to proof or changed frame history".into());
    }
    Ok(())
}

#[tokio::test]
async fn mismatched_tool_content_cid_is_rejected_before_verification() -> Result<(), Box<dyn Error>>
{
    let mut frame = frame()?;
    let mut bad_result = sample_result("durable output");
    bad_result.content_cid = Cid::from_bytes([9; 32]);
    let (assemble, chat, tools, _, _, _, _) =
        components(vec![tool_script(FinishReason::ToolCalls)], bad_result);
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    match result {
        Err(FrameError::ToolResultReferenceMismatch) => Ok(()),
        Err(other) => Err(format!("unexpected error: {other}").into()),
        Ok(_) => Err("mismatched content reference was accepted".into()),
    }
}

#[tokio::test]
async fn stop_finish_with_tool_calls_fails_before_authorization() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, _, tool_calls, _) = components(
        vec![tool_script(FinishReason::Stop)],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::InvalidToolCall)) || !lock_recover(&tool_calls).is_empty()
    {
        return Err("inconsistent finish reason reached tool authorization".into());
    }
    Ok(())
}

#[tokio::test]
async fn tool_runner_receives_result_ceiling_and_oversize_is_not_replayed()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, requests, _, result_limits) = components(
        vec![tool_script(FinishReason::ToolCalls)],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.max_tool_result_bytes = 4;
    match run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits,
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await
    {
        Err(FrameError::OutputLimit) => {}
        Err(other) => return Err(format!("unexpected frame error: {other}").into()),
        Ok(_) => return Err("oversized tool output was accepted".into()),
    }
    if lock_recover(&result_limits).as_slice() != [4] || lock_recover(&requests).len() != 1 {
        return Err(
            "tool adapter did not receive the result bound or a second model call ran".into(),
        );
    }
    Ok(())
}

#[tokio::test]
async fn child_scope_only_narrows_and_only_unused_quota_returns() -> Result<(), Box<dyn Error>> {
    let mut parent = frame()?;
    let child_quota = ResourceQuota {
        cpu_millis: 1_000,
        memory_mb: 128,
        network: Default::default(),
        max_steps: 2,
        max_depth: 2,
        token_budget: 200,
    };
    let broad_scope = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("work")?),
        max_churn_lines: 10,
        ttl: Duration::from_secs(30),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    if !matches!(
        parent.spawn_child(
            AgentRole::SubAgent,
            broad_scope,
            child_quota.clone(),
            ScratchpadRef::new(Cid::from_bytes([8; 32])),
            &TestClock,
        ),
        Err(FrameError::Scope(_))
    ) {
        return Err("broader child resource prefix was accepted".into());
    }
    if parent.remaining_quota().max_steps != 8 || parent.remaining_quota().token_budget != 1_000 {
        return Err("rejected child scope reserved parent quota".into());
    }

    let narrow_scope = Scope {
        resource: ResourceScope::Exact(ResourcePath::new("workspace/file.rs")?),
        max_churn_lines: 10,
        ttl: Duration::from_secs(30),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    let mut child = parent
        .spawn_child(
            AgentRole::SubAgent,
            narrow_scope,
            child_quota,
            ScratchpadRef::new(Cid::from_bytes([9; 32])),
            &TestClock,
        )?
        .activate(
            ModelSelection {
                model: ModelId::new("test/model")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
    let (assemble, chat, tools, _, _, _, _) = components(
        vec![vec![
            ChatChunk::Text("checked".to_owned()),
            ChatChunk::Finished(FinishReason::Stop),
        ]],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let outcome = run_frame(
        FrameRunInput {
            frame: &mut child,
            goal: "inspect one file",
            cancellation: &Cancellation::new(),
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await?;
    if outcome.attempts != 1
        || child.remaining_quota().max_steps != 1
        || child.remaining_quota().token_budget != 140
    {
        return Err("child did not retain correct spent and available quota".into());
    }
    drop(child);
    let remaining = parent.remaining_quota();
    if remaining.cpu_millis != 10_000
        || remaining.memory_mb != 1_024
        || remaining.max_steps != 7
        || remaining.token_budget != 940
    {
        return Err("child returned spent quota or failed to return unused quota".into());
    }
    Ok(())
}

#[test]
fn content_reference_failures_are_fatal_integrity_errors() -> Result<(), Box<dyn Error>> {
    let kind = FrameError::ToolResultReferenceMismatch.kind();
    if kind != ErrorKind::Fatal {
        return Err("content reference mismatch must be fatal".into());
    }
    Ok(())
}

#[tokio::test]
async fn journal_persists_transcript_before_publishing_event() -> Result<(), Box<dyn Error>> {
    let content = MemoryContentStore::default();
    let event_records = Arc::new(Mutex::new(Vec::new()));
    let events = MemoryEventStore {
        content: content.clone(),
        records: Arc::clone(&event_records),
    };
    let journal = JournalAdapter::new(content.clone(), events, SessionId::from_u128(44));
    let record = FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame: FrameId::from_u128(55),
        step: 2,
        goal: "inspect".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: Vec::new(),
        outcome: FrameStepOutcome::Cancelled,
    };
    let transcript = journal.record(&record).await?;
    let persisted = content.get(&transcript, 4_096)?;
    if persisted.is_empty() {
        return Err("journal returned a content id for an empty payload".into());
    }
    let records = lock_recover(&event_records);
    let event = records.first().ok_or("frame step event was not appended")?;
    if event.session() != SessionId::from_u128(44)
        || !matches!(
            event.event(),
            Event::FrameStepRecorded {
                frame,
                step: 2,
                transcript: event_cid,
                outcome: FrameStepOutcome::Cancelled,
            } if *frame == FrameId::from_u128(55) && *event_cid == transcript
        )
    {
        return Err("event did not reference the persisted transcript and terminal state".into());
    }
    Ok(())
}

#[tokio::test]
async fn durable_journal_requires_tool_content_to_exist_and_match() -> Result<(), Box<dyn Error>> {
    let content = MemoryContentStore::default();
    let event_records = Arc::new(Mutex::new(Vec::new()));
    let events = MemoryEventStore {
        content: content.clone(),
        records: Arc::clone(&event_records),
    };
    let journal = JournalAdapter::new(content.clone(), events, SessionId::from_u128(45));
    let result_content = "model-visible bytes";
    let content_cid = content.put(result_content.as_bytes())?;
    let stored_result = content.put(b"complete tool output")?;
    let record = FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame: FrameId::from_u128(56),
        step: 3,
        goal: "inspect".to_owned(),
        request: None,
        assistant_response: None,
        result: Some(StepOutcome::ToolResults {
            assistant_message: ChatMessage::text(ChatRole::Assistant, "reading".to_owned()),
            results: vec![ExecutedToolResult {
                tool_call_id: "call-2".to_owned(),
                result: ToolResultRef {
                    cid: stored_result,
                    content_cid,
                    content: result_content.to_owned(),
                },
            }],
        }),
        executed_tool_results: Vec::new(),
        outcome: FrameStepOutcome::Accepted,
    };
    let mut unsupported = record.clone();
    unsupported.schema = FRAME_STEP_RECORD_SCHEMA_VERSION + 1;
    match journal.record(&unsupported).await {
        Err(FrameJournalError::UnsupportedSchema(schema))
            if schema == FRAME_STEP_RECORD_SCHEMA_VERSION + 1 => {}
        Err(other) => return Err(format!("unexpected schema error: {other}").into()),
        Ok(_) => return Err("journal accepted an unsupported transcript schema".into()),
    }
    if !lock_recover(&event_records).is_empty() {
        return Err("unsupported transcript schema appended an event".into());
    }
    journal.record(&record).await?;
    if lock_recover(&event_records).len() != 1 {
        return Err("verified tool transcript was not published after persistence".into());
    }

    let interrupted_results = match record.result.as_ref() {
        Some(StepOutcome::ToolResults { results, .. }) => results.clone(),
        _ => return Err("expected the successful tool result record".into()),
    };
    let interrupted = FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame: FrameId::from_u128(56),
        step: 4,
        goal: "inspect after cancellation".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: interrupted_results,
        outcome: FrameStepOutcome::Cancelled,
    };
    journal.record(&interrupted).await?;
    if lock_recover(&event_records).len() != 2 {
        return Err("interrupted tool result was not journaled".into());
    }

    let mut mismatched = interrupted;
    if let Some(first) = mismatched.executed_tool_results.first_mut() {
        first.result.content = "different visible bytes".to_owned();
    } else {
        return Err("interrupted tool result is missing".into());
    }
    match journal.record(&mismatched).await {
        Err(FrameJournalError::ReferencedContentMismatch) => {}
        Err(other) => return Err(format!("unexpected journal error: {other}").into()),
        Ok(_) => {
            return Err("journal accepted content that differs from the referenced blob".into());
        }
    }
    if lock_recover(&event_records).len() != 2 {
        return Err("journal published an event for mismatched referenced content".into());
    }
    Ok(())
}

#[tokio::test]
async fn real_cas_and_event_log_reopen_the_versioned_frame_transcript() -> Result<(), Box<dyn Error>>
{
    let directory = TempDir::new()?;
    let cas = Cas::open(CasConfig::new(directory.path().join("cas")))?;
    let session = SessionId::from_u128(57);
    let model_visible = "persisted model-visible result";
    let content_cid = cas.put(model_visible.as_bytes())?;
    let tool_cid = cas.put(b"complete raw tool result")?;
    let record = FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame: FrameId::from_u128(58),
        step: 4,
        goal: "read one file".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: vec![ExecutedToolResult {
            tool_call_id: "call-durable-1".to_owned(),
            result: ToolResultRef {
                cid: tool_cid,
                content_cid,
                content: model_visible.to_owned(),
            },
        }],
        outcome: FrameStepOutcome::Cancelled,
    };
    let event_path = directory.path().join("events.redb");
    let transcript = {
        let events = EventLog::open(EventLogConfig::new(&event_path))?;
        let journal = JournalAdapter::new(cas.clone(), events, session);
        journal.record(&record).await?
    };

    let events = EventLog::open(EventLogConfig::new(&event_path))?;
    let replayed = events.read_range(Seq::new(1), Seq::new(1))?;
    let event = replayed
        .first()
        .ok_or("event was not durable after reopen")?;
    if !matches!(
        event.event(),
        Event::FrameStepRecorded {
            frame,
            step: 4,
            transcript: stored_transcript,
            outcome: FrameStepOutcome::Cancelled,
        } if *frame == FrameId::from_u128(58) && *stored_transcript == transcript
    ) {
        return Err("reopened event did not preserve the transcript reference and outcome".into());
    }
    drop(events);

    let bytes = cas.get(&transcript)?;
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes)?;
    let tool_results = snapshot
        .get("executed_tool_results")
        .and_then(serde_json::Value::as_array)
        .ok_or("versioned transcript omitted interrupted tool results")?;
    let first_result = tool_results.first().ok_or("interrupted result was empty")?;
    if snapshot.get("schema").and_then(serde_json::Value::as_u64)
        != Some(u64::from(FRAME_STEP_RECORD_SCHEMA_VERSION))
        || snapshot.get("outcome").and_then(serde_json::Value::as_str) != Some("cancelled")
        || first_result
            .get("tool_call_id")
            .and_then(serde_json::Value::as_str)
            != Some("call-durable-1")
        || first_result
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(serde_json::Value::as_str)
            != Some(model_visible)
    {
        return Err(format!(
            "CAS transcript lost its schema, cancellation state, call id, or result: {snapshot}"
        )
        .into());
    }
    Ok(())
}

#[tokio::test]
async fn verifier_retries_stop_at_bound_and_keep_quota_spent() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, requests, _, _) = components(
        vec![
            vec![
                ChatChunk::Text("try one".to_owned()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
            vec![
                ChatChunk::Text("try two".to_owned()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
            vec![
                ChatChunk::Text("must not be requested".to_owned()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
        ],
        sample_result("unused"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.maximum_retries = 1;
    let outcome = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits,
        },
        &assemble,
        &chat,
        &tools,
        &Reject,
        &journal,
    )
    .await?;
    if outcome.stop != FrameRunStop::RetryLimit || outcome.attempts != 2 || outcome.retries != 1 {
        return Err("verifier retries did not stop at the configured bound".into());
    }
    if frame.steps() != 2
        || frame.remaining_quota().max_steps != 6
        || frame.remaining_quota().token_budget != 880
        || !frame.diff_timeline().is_empty()
        || lock_recover(&requests).len() != 2
    {
        return Err("rejected attempts refunded steps/tokens or exceeded model-call bound".into());
    }
    Ok(())
}

#[tokio::test]
async fn tool_loop_stops_before_an_attempt_beyond_round_bound() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, requests, _, _) = components(
        vec![tool_script(FinishReason::ToolCalls)],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.maximum_tool_rounds = 1;
    let outcome = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &Cancellation::new(),
            limits,
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await?;
    if outcome.stop != FrameRunStop::ToolRoundLimit
        || outcome.attempts != 1
        || outcome.tool_rounds != 1
        || lock_recover(&requests).len() != 1
    {
        return Err("tool loop ran past its configured round bound".into());
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_records_an_incomplete_assistant_stream_as_partial_and_keeps_reserved_quota()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let cancellation = Cancellation::new();
    let trigger = cancellation.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let chat = BlockingModel {
        requests,
        cancellation: cancellation.clone(),
        finish_before_block: false,
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let assemble = Assembler { seen };
    let tools = Tools {
        result: sample_result("unused"),
        calls: Arc::new(Mutex::new(Vec::new())),
        result_limits: Arc::new(Mutex::new(Vec::new())),
        cancel_after_first: None,
    };
    let journal = TestJournal(Mutex::new(Vec::new()));
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        trigger.cancel();
    });
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "inspect files",
            cancellation: &cancellation,
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::Cancelled))
        || frame.steps() != 1
        || frame.remaining_quota().max_steps != 7
        || frame.remaining_quota().token_budget != 940
        || lock_recover(&journal.0)
            .first()
            .map(|record| record.outcome)
            != Some(FrameStepOutcome::Cancelled)
        || lock_recover(&journal.0)
            .first()
            .and_then(|record| record.request.as_ref())
            .is_none()
        || lock_recover(&journal.0)
            .first()
            .and_then(|record| record.assistant_response.as_ref())
            .is_none_or(|response| {
                response.complete
                    || response.finish_reason.is_some()
                    || response.tool_calls.first().is_none_or(|call| {
                        call.id.as_deref() != Some("call-partial")
                            || call.name.as_deref() != Some("read_workspace")
                            || call.arguments != "{\"path\":"
                    })
            })
    {
        return Err("cancellation refunded quota or failed to journal the reserved attempt".into());
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_after_tool_execution_journals_the_partial_result()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let cancellation = Cancellation::new();
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let result_limits = Arc::new(Mutex::new(Vec::new()));
    let result = sample_result("first completed tool");
    let (assemble, chat, _, _, _, _, _) = components(
        vec![vec![
            ChatChunk::ToolCallDelta {
                index: 0,
                id: Some("call-1".to_owned()),
                name: Some("read_workspace".to_owned()),
                arguments_fragment: "{}".to_owned(),
            },
            ChatChunk::ToolCallDelta {
                index: 1,
                id: Some("call-2".to_owned()),
                name: Some("read_workspace".to_owned()),
                arguments_fragment: "{}".to_owned(),
            },
            ChatChunk::Finished(FinishReason::ToolCalls),
        ]],
        result.clone(),
    );
    let tools = Tools {
        result,
        calls: tool_calls,
        result_limits,
        cancel_after_first: Some(cancellation.clone()),
    };
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        FrameRunInput {
            frame: &mut frame,
            goal: "read two files",
            cancellation: &cancellation,
            limits: base_limits(),
        },
        &assemble,
        &chat,
        &tools,
        &Accept,
        &journal,
    )
    .await;
    if !matches!(result, Err(FrameError::Cancelled)) {
        return Err("cancellation between authorized tool calls was not observed".into());
    }
    let records = lock_recover(&journal.0);
    let record = records
        .first()
        .ok_or("interrupted step was not journaled")?;
    let partial = record
        .executed_tool_results
        .first()
        .ok_or("completed tool result was omitted from interrupted transcript")?;
    if records.len() != 1
        || record.result.is_some()
        || record.outcome != FrameStepOutcome::Cancelled
        || record.executed_tool_results.len() != 1
        || partial.tool_call_id != "call-1"
    {
        return Err("interrupted transcript did not preserve the completed tool result".into());
    }
    let assistant_response = record
        .assistant_response
        .as_ref()
        .ok_or("completed assistant response was omitted from interrupted transcript")?;
    let first_call = assistant_response
        .tool_calls
        .first()
        .ok_or("completed assistant tool call was omitted")?;
    if !assistant_response.complete
        || assistant_response.finish_reason != Some(FinishReason::ToolCalls)
        || first_call.id.as_deref() != Some("call-1")
        || first_call.name.as_deref() != Some("read_workspace")
        || first_call.arguments != "{}"
    {
        return Err("interrupted transcript lost the bounded assistant tool call".into());
    }
    Ok(())
}
