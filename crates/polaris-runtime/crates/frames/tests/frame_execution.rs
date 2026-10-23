use bytes::Bytes;
use polaris_runtime_frames::{
    Active, AgentContextFrame, AgentRole, AssembleRequest, Cancellation, ExecutedToolResult,
    FrameError, FrameJournal, FrameJournalError, FramePrompt, FrameRunLimits, FrameRunStop,
    FrameScope, FrameStepRecord, JournalAdapter, ModelSelection, ResourceQuota, ScratchpadRef,
    StepOutcome, StepTool, StepVerifier, ToolCall, ToolResultRef, ToolRunner, Verification,
    run_frame,
};
use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    contracts::{Event, EventRecord, FrameStepOutcome, Timestamp},
    effort::Effort,
    error::{ErrorKind, HasKind},
    ids::{Cid, FastFingerprint, FrameId, ModelId, Seq, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, Effects,
        EventStore, EventStoreError, FinishReason, FunctionalOperator, ModelError, ToolSpec,
    },
};
use polaris_security_capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, Token,
};
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
    ) -> Result<ToolResultRef, FrameError> {
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
}
struct BlockingStream(Cancellation);
impl ChatChunkSource for BlockingStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.cancelled().await;
        None
    }
}
impl ChatModel for BlockingModel {
    type Stream = BlockingStream;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        lock_recover(&self.requests).push(request);
        Ok(BlockingStream(self.cancellation.clone()))
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

fn components(
    scripts: Vec<Vec<ChatChunk>>,
    result: ToolResultRef,
) -> (
    Assembler,
    ScriptedModel,
    Tools,
    Arc<Mutex<Vec<Vec<ChatMessage>>>>,
    Arc<Mutex<Vec<ChatRequest>>>,
    Arc<Mutex<Vec<ToolCall>>>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
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
        },
        seen,
        requests,
        calls,
    )
}

#[tokio::test]
async fn tool_transcript_is_replayed_with_cas_reference_and_call_identity()
-> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, seen, requests, tool_calls) = components(
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
        &mut frame,
        "inspect files",
        &Cancellation::new(),
        base_limits(),
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
async fn mismatched_tool_content_cid_is_rejected_before_verification() -> Result<(), Box<dyn Error>>
{
    let mut frame = frame()?;
    let mut bad_result = sample_result("durable output");
    bad_result.content_cid = Cid::from_bytes([9; 32]);
    let (assemble, chat, tools, _, _, _) =
        components(vec![tool_script(FinishReason::ToolCalls)], bad_result);
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        &mut frame,
        "inspect files",
        &Cancellation::new(),
        base_limits(),
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
    let (assemble, chat, tools, _, _, tool_calls) = components(
        vec![tool_script(FinishReason::Stop)],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let result = run_frame(
        &mut frame,
        "inspect files",
        &Cancellation::new(),
        base_limits(),
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
        frame: FrameId::from_u128(55),
        step: 2,
        goal: "inspect".to_owned(),
        request: None,
        result: None,
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
        frame: FrameId::from_u128(56),
        step: 3,
        goal: "inspect".to_owned(),
        request: None,
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
        outcome: FrameStepOutcome::Accepted,
    };
    journal.record(&record).await?;
    if lock_recover(&event_records).len() != 1 {
        return Err("verified tool transcript was not published after persistence".into());
    }

    let mut mismatched = record;
    if let Some(StepOutcome::ToolResults { results, .. }) = &mut mismatched.result {
        let first = results.first_mut().ok_or("tool result is missing")?;
        first.result.content = "different visible bytes".to_owned();
    }
    match journal.record(&mismatched).await {
        Err(FrameJournalError::ReferencedContentMismatch) => {}
        Err(other) => return Err(format!("unexpected journal error: {other}").into()),
        Ok(_) => {
            return Err("journal accepted content that differs from the referenced blob".into());
        }
    }
    if lock_recover(&event_records).len() != 1 {
        return Err("journal published an event for mismatched referenced content".into());
    }
    Ok(())
}

#[tokio::test]
async fn verifier_retries_stop_at_bound_and_keep_quota_spent() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, requests, _) = components(
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
        &mut frame,
        "inspect files",
        &Cancellation::new(),
        limits,
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
        || lock_recover(&requests).len() != 2
    {
        return Err("rejected attempts refunded steps/tokens or exceeded model-call bound".into());
    }
    Ok(())
}

#[tokio::test]
async fn tool_loop_stops_before_an_attempt_beyond_round_bound() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let (assemble, chat, tools, _, requests, _) = components(
        vec![tool_script(FinishReason::ToolCalls)],
        sample_result("durable output"),
    );
    let journal = TestJournal(Mutex::new(Vec::new()));
    let mut limits = base_limits();
    limits.maximum_tool_rounds = 1;
    let outcome = run_frame(
        &mut frame,
        "inspect files",
        &Cancellation::new(),
        limits,
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
async fn cancellation_records_attempt_and_does_not_refund_quota() -> Result<(), Box<dyn Error>> {
    let mut frame = frame()?;
    let cancellation = Cancellation::new();
    let trigger = cancellation.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let chat = BlockingModel {
        requests,
        cancellation: cancellation.clone(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let assemble = Assembler { seen };
    let tools = Tools {
        result: sample_result("unused"),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let journal = TestJournal(Mutex::new(Vec::new()));
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        trigger.cancel();
    });
    let result = run_frame(
        &mut frame,
        "inspect files",
        &cancellation,
        base_limits(),
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
    {
        return Err("cancellation refunded quota or failed to journal the reserved attempt".into());
    }
    Ok(())
}
