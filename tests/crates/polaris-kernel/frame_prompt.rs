//! Production prompt assembly with the actual PII vault, kernel, catalog and CAS.
//! CountingFixture is a synthetic counting oracle, never production tokenizer evidence.
use polaris_kernel::{
    Kernel, KernelHandle, KernelPromptAssembler, KernelToolRunner, LocalKernelConfig,
    PromptContext, PromptLimits,
};
use polaris_runtime::context::EpistemicScratchpad;
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, AssembleRequest, FrameScope, ModelSelection,
    ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FrameId, ModelId, SessionId, TokenCount},
    registry::{
        ChatMessage, ChatRequest, ChatRole, ChatToolCall, FunctionalOperator, ModelError,
        PromptTokenCount, PromptTokenizer,
    },
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn configured_run_limits() -> polaris_runtime::frames::FrameRunLimits {
    polaris_runtime::frames::FrameRunLimits {
        maximum_attempts: 2,
        maximum_retries: 0,
        maximum_tool_rounds: 1,
        max_goal_bytes: 4096,
        max_prompt_bytes: 262_144,
        max_output_bytes: 65_536,
        max_tool_calls: 8,
        max_tool_argument_bytes: 65_536,
        max_tool_result_bytes: 65_536,
        max_stream_chunks: 16,
    }
}

#[tokio::test]
async fn assembler_masks_goal_before_failed_attempt_journal() -> TestResult {
    use polaris_runtime::frames::{FrameJournalLimits, FrameRunInput};
    use polaris_sdk::{contracts::Event, ids::Seq};
    let mut fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let assembler = fixture
        .assembler(Arc::new(CountingFixture::default()), context(), limits())
        .await?;
    let journal = polaris_kernel::KernelFrameJournal::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
        FrameJournalLimits::default(),
    )?;
    let cancellation = CancellationToken::new();
    let result = assembler
        .run_frame(
            FrameRunInput {
                frame: &mut fixture.frame,
                goal: "Investigate for journal.owner@example.test",
                cancellation: &cancellation,
                limits: configured_run_limits(),
            },
            &UnusedVerifier,
            &journal,
        )
        .await;
    assert!(
        result.is_err(),
        "the synthetic model deliberately fails dispatch"
    );
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let cid = records
        .iter()
        .find_map(|record| match record.event() {
            Event::FrameStepRecorded { transcript, .. } => Some(*transcript),
            _ => None,
        })
        .ok_or("failed attempt was not actually journaled")?;
    let expanded = fixture
        .kernel
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":cid.to_hex()}),
        )
        .await?;
    let text = expanded
        .value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or("actual CAS transcript missing")?;
    assert_eq!(
        polaris_kernel::storage::cas::content_id(text.as_bytes()),
        cid
    );
    assert!(
        !text.contains("journal.owner@example.test"),
        "the raw goal must not leak through the failed-attempt journal path"
    );
    let payload: serde_json::Value = serde_json::from_str(text)?;
    assert!(
        payload
            .get("goal")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|goal| goal.contains("<PII1:")),
        "actual transcript must retain the reversible masked goal"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn configured_agent_frame_runs_degraded_without_s1() -> TestResult {
    use polaris_kernel::{ConfiguredFrameError, ConfiguredFrameServices, SystemOneCoverage};
    use polaris_runtime::frames::FrameRunInput;
    let mut fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let result = fixture
        .kernel
        .run_agent_frame(
            FrameRunInput {
                frame: &mut fixture.frame,
                goal: "Inspect parser owned by degraded.owner@example.test",
                cancellation: &CancellationToken::new(),
                limits: configured_run_limits(),
            },
            ConfiguredFrameServices {
                token: fixture.token.clone(),
                runner: Arc::clone(&fixture.runner),
                model: Arc::clone(&counter),
                context: context(),
                prompt: polaris_kernel::PromptAssemblySettings {
                    limits: limits(),
                    privacy_vault: None,
                },
                role_table: polaris_system_one::watchdog::RoleTable::default(),
                semantic_privacy: polaris_kernel::SemanticPrivacyLimits::default(),
            },
        )
        .await;
    // The fixture refuses dispatch, so the run ends in an execution error after prompt counting.
    match result {
        Ok(outcome) => assert_eq!(outcome.system_one, SystemOneCoverage::Degraded),
        Err(ConfiguredFrameError::Execution(_)) => {}
        Err(other) => return Err(format!("unexpected degraded-mode error: {other:?}").into()),
    }
    let requests = counter
        .requests
        .lock()
        .map_err(|_| "fixture state")?
        .clone();
    assert!(
        !requests.is_empty(),
        "without S1 the frame still reaches the provider"
    );
    for request in &requests {
        for message in &request.messages {
            assert!(
                !message.content.contains("degraded.owner@example.test"),
                "deterministic masking stays fail-closed without S1"
            );
        }
    }
    assert!(
        requests.iter().any(|request| request
            .messages
            .iter()
            .any(|message| message.content.contains("<PII"))),
        "the goal reaches the provider in masked form"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn configured_agent_frame_rejects_foreign_root_before_provider() -> TestResult {
    use polaris_kernel::{ConfiguredFrameError, ConfiguredFrameServices};
    use polaris_runtime::frames::FrameRunInput;
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let mut foreign = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let result = fixture
        .kernel
        .run_agent_frame(
            FrameRunInput {
                frame: &mut foreign.frame,
                goal: "Inspect parser",
                cancellation: &CancellationToken::new(),
                limits: configured_run_limits(),
            },
            ConfiguredFrameServices {
                token: foreign.token.clone(),
                runner: Arc::clone(&foreign.runner),
                model: Arc::clone(&counter),
                context: context(),
                prompt: polaris_kernel::PromptAssemblySettings {
                    limits: limits(),
                    privacy_vault: None,
                },
                role_table: polaris_system_one::watchdog::RoleTable::default(),
                semantic_privacy: polaris_kernel::SemanticPrivacyLimits::default(),
            },
        )
        .await;
    assert!(matches!(result, Err(ConfiguredFrameError::Identity)));
    assert!(
        counter
            .requests
            .lock()
            .map_err(|_| "fixture state")?
            .is_empty()
    );
    assert_eq!(foreign.frame.steps(), 0);
    fixture.kernel.shutdown().await?;
    foreign.kernel.shutdown().await?;
    Ok(())
}

struct SemanticNameDetector {
    unavailable: bool,
}
impl polaris_sdk::system_one::PiiSpanDetector for SemanticNameDetector {
    async fn detect_pii_spans(
        &self,
        text: &str,
    ) -> Result<Vec<polaris_sdk::system_one::PiiEntitySpan>, polaris_sdk::registry::DecisionError>
    {
        if self.unavailable {
            return Err(polaris_sdk::registry::DecisionError::Unavailable(
                "synthetic".into(),
            ));
        }
        text.match_indices("Alice")
            .map(|(start, value)| {
                let end = start.checked_add(value.len()).ok_or_else(|| {
                    polaris_sdk::registry::DecisionError::Model("synthetic bounds".into())
                })?;
                polaris_sdk::system_one::PiiEntitySpan::new(
                    u32::try_from(start).map_err(|_| {
                        polaris_sdk::registry::DecisionError::Model("synthetic bounds".into())
                    })?,
                    u32::try_from(end).map_err(|_| {
                        polaris_sdk::registry::DecisionError::Model("synthetic bounds".into())
                    })?,
                    0.9,
                )
                .map_err(|_| polaris_sdk::registry::DecisionError::Model("synthetic span".into()))
            })
            .collect()
    }
}
#[tokio::test]
async fn assembler_masks_semantic_names_before_tokenizer() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let mut source = context();
    source.system_prompt = "Help Alice".into();
    source.repo_map = "Alice owns the module".into();
    let assembler = fixture
        .assembler(Arc::clone(&counter), source, limits())
        .await?
        .with_semantic_privacy(
            Arc::new(SemanticNameDetector { unavailable: false }),
            polaris_kernel::SemanticPrivacyLimits::default(),
        )?;
    let mut call = ChatMessage::text(ChatRole::Assistant, "Alice requested this".into());
    call.tool_calls.push(ChatToolCall {
        id: "semantic-call".into(),
        name: "read_file".into(),
        arguments_json: "{\"path\":\"Alice\"}".into(),
    });
    let mut result = ChatMessage::text(ChatRole::Tool, "Alice result".into());
    result.tool_call_id = Some("semantic-call".into());
    let prompt = assembler
        .apply(fixture.request("Ask Alice", vec![call, result]))
        .await?;
    {
        let requests = counter.requests.lock().map_err(|_| "fixture state")?;
        for request in requests.iter() {
            assert!(
                !serde_json::to_string(request)?.contains("Alice"),
                "semantic text must be masked before any provider count"
            );
        }
    }
    let original_call = prompt
        .messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .next()
        .ok_or("missing preserved call")?;
    assert_eq!(original_call.id, "semantic-call");
    assert_eq!(original_call.name, "read_file");
    let _: serde_json::Value = serde_json::from_str(&original_call.arguments_json)?;
    let coverage = assembler
        .semantic_privacy_report()?
        .ok_or("missing actual coverage")?;
    assert!(coverage.ner_fields > 0);
    assert_eq!(coverage.degraded_fields, 0);
    fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn assembler_reports_semantic_degradation_without_skipping_dfa() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?
        .with_semantic_privacy(
            Arc::new(SemanticNameDetector { unavailable: true }),
            polaris_kernel::SemanticPrivacyLimits::default(),
        )?;
    let prompt = assembler
        .apply(fixture.request("Contact semantic.canary@example.com", Vec::new()))
        .await?;
    assert!(!serde_json::to_string(&prompt.messages)?.contains("semantic.canary@example.com"));
    let coverage = assembler
        .semantic_privacy_report()?
        .ok_or("missing actual coverage")?;
    assert!(coverage.degraded_fields > 0);
    assert_eq!(coverage.ner_fields, 0);
    fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn assembler_bounds_semantic_field_work() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let assembler = fixture
        .assembler(Arc::new(CountingFixture::default()), context(), limits())
        .await?
        .with_semantic_privacy(
            Arc::new(SemanticNameDetector { unavailable: false }),
            polaris_kernel::SemanticPrivacyLimits {
                max_fields: 1,
                ..polaris_kernel::SemanticPrivacyLimits::default()
            },
        )?;
    let _prompt = assembler
        .apply(fixture.request("email limited.canary@example.com", Vec::new()))
        .await?;
    let coverage = assembler
        .semantic_privacy_report()?
        .ok_or("missing actual coverage")?;
    assert_eq!(coverage.ner_fields, 1);
    assert!(coverage.degraded_fields > 0);
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_restores_exact_private_marker_map() -> TestResult {
    use polaris_kernel::{
        PromptAssemblySettings,
        privacy_snapshot::{KernelPiiSealer, KernelPiiSnapshotStore},
    };
    use polaris_security::{automata::EntropyCfg, secrets::SecretVault};
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let first = assembler
        .apply(fixture.request("Preserve resume.canary@example.com", Vec::new()))
        .await?;
    let store = KernelPiiSnapshotStore::new(
        fixture.kernel.clone(),
        fixture.frame.view().id,
        Arc::new(KernelPiiSealer::new(Arc::new(SecretVault::new()?))),
    )?;
    let snapshot = store.save(Arc::clone(assembler.privacy_vault())).await?;
    let restored = Arc::new(
        store
            .load(
                snapshot,
                EntropyCfg::default(),
                assembler.privacy_vault().limits(),
            )
            .await?,
    );
    let next = KernelPromptAssembler::new_with_privacy(
        fixture.kernel.clone(),
        fixture.frame.prompt_binding()?,
        fixture.token.clone(),
        Arc::clone(&fixture.runner),
        counter,
        context(),
        PromptAssemblySettings {
            limits: limits(),
            privacy_vault: Some(restored),
        },
    )
    .await?;
    let second = next
        .apply(fixture.request("Preserve resume.canary@example.com", Vec::new()))
        .await?;
    assert_eq!(
        first.messages, second.messages,
        "restored map must preserve the exact model-visible prefix and goal markers"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn assembler_rejects_foreign_or_weaker_privacy_vault() -> TestResult {
    use polaris_kernel::PromptAssemblySettings;
    use polaris_security::{
        automata::EntropyCfg,
        pii::{PiiLimits, PiiVault},
    };
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let records = fixture.kernel.context().records(
        polaris_sdk::ids::Seq::new(1),
        polaris_sdk::ids::Seq::new(u64::MAX),
    )?;
    let session = records
        .first()
        .ok_or("missing configured session")?
        .session();
    let matching_capacity = PiiLimits {
        max_input_bytes: limits().max_input_bytes,
        max_output_bytes: limits().max_input_bytes,
        ..PiiLimits::default()
    };
    for (frame, entropy, capacity) in [
        (FrameId::new_v7(), EntropyCfg::default(), matching_capacity),
        (
            fixture.frame.view().id,
            EntropyCfg {
                threshold_milli: 8000,
                ..EntropyCfg::default()
            },
            matching_capacity,
        ),
        (
            fixture.frame.view().id,
            EntropyCfg::default(),
            PiiLimits::default(),
        ),
    ] {
        let vault = Arc::new(PiiVault::new(session, frame, entropy, capacity)?);
        let result = KernelPromptAssembler::new_with_privacy(
            fixture.kernel.clone(),
            fixture.frame.prompt_binding()?,
            fixture.token.clone(),
            Arc::clone(&fixture.runner),
            Arc::new(CountingFixture::default()),
            context(),
            PromptAssemblySettings {
                limits: limits(),
                privacy_vault: Some(vault),
            },
        )
        .await;
        assert!(
            result.is_err(),
            "restored vault may not change actual owner or weaken privacy policy"
        );
    }
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[derive(Debug, Default)]
struct CountingFixture {
    requests: Mutex<Vec<ChatRequest>>,
    reject: bool,
    trim: bool,
    stall: bool,
}
impl PromptTokenizer for CountingFixture {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        if self.stall {
            return std::future::pending().await;
        }
        self.requests
            .lock()
            .map_err(|_| ModelError::Protocol("fixture state".into()))?
            .push(request.clone());
        if self.reject {
            return Err(ModelError::Protocol("fixture unavailable".into()));
        }
        let has_old = request
            .messages
            .iter()
            .any(|message| message.content.contains("old exchange"));
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(if self.trim && has_old { 500 } else { 40 }),
            max_context_tokens: TokenCount::new(256),
        })
    }
}

struct UnusedStream;
impl polaris_sdk::registry::ChatChunkSource for UnusedStream {
    async fn next_chunk(&mut self) -> Option<Result<polaris_sdk::registry::ChatChunk, ModelError>> {
        None
    }
}
impl polaris_sdk::registry::ChatModel for CountingFixture {
    type Stream = UnusedStream;
    async fn stream(&self, _: ChatRequest) -> Result<Self::Stream, ModelError> {
        Err(ModelError::Protocol("unexpected fixture dispatch".into()))
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    kernel: KernelHandle,
    frame: AgentContextFrame<Active>,
    token: Token,
    runner: Arc<KernelToolRunner>,
}
impl Fixture {
    async fn new(role: AgentRole) -> TestResult<Self> {
        Self::new_model(role, "fixture/model", None).await
    }
    async fn new_model(role: AgentRole, model: &str, id: Option<FrameId>) -> TestResult<Self> {
        Self::new_with_rights(
            role,
            model,
            id,
            Rights::new(&[Right::Read, Right::ExecuteTool]),
        )
        .await
    }
    async fn new_with_rights(
        role: AgentRole,
        model: &str,
        id: Option<FrameId>,
        rights: Rights,
    ) -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let id = match id {
            Some(id) => id,
            None => FrameId::new_v7(),
        };
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            SessionId::new_v7(),
            id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights,
        };
        let token = authority.issue_root(id, &scope, &SystemClock)?;
        let expires = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            id,
            role,
            token.clone(),
            FrameScope {
                capability: scope,
                expires_at_micros: expires,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 5,
                max_depth: 3,
                token_budget: 10_000,
            },
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new(model)?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        let runner = Arc::new(KernelToolRunner::new(
            kernel.clone(),
            frame.tool_binding(),
            authority.public_key(),
            CancellationToken::new(),
        )?);
        Ok(Self {
            _root: root,
            _workspace: workspace,
            kernel,
            frame,
            token,
            runner,
        })
    }
    async fn assembler(
        &self,
        counter: Arc<CountingFixture>,
        context: PromptContext,
        limits: PromptLimits,
    ) -> TestResult<KernelPromptAssembler<CountingFixture>> {
        Ok(KernelPromptAssembler::new(
            self.kernel.clone(),
            self.frame.prompt_binding()?,
            self.token.clone(),
            Arc::clone(&self.runner),
            counter,
            context,
            limits,
        )
        .await?)
    }
    fn request(&self, goal: &str, prior_messages: Vec<ChatMessage>) -> AssembleRequest {
        AssembleRequest {
            frame: self.frame.view(),
            goal: goal.into(),
            prior_messages,
        }
    }
}

#[tokio::test]
async fn assembler_masks_structured_arguments_and_rejects_unreadable_archive() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let history = vec![
        ChatMessage::assistant_calls(
            String::new(),
            vec![ChatToolCall {
                id: "call-safe".into(),
                name: "read_file".into(),
                arguments_json: "{\"path\":\"person@example.com\"}".into(),
            }],
            Some("reasoning@example.com".into()),
        ),
        ChatMessage::tool_result("call-safe".into(), "result@example.com".into()),
    ];
    let prompt = assembler.apply(fixture.request("goal", history)).await?;
    let payload = serde_json::to_string(&prompt.messages)?;
    for secret in [
        "person@example.com",
        "reasoning@example.com",
        "result@example.com",
    ] {
        assert!(!payload.contains(secret), "mask every model-bound channel");
    }
    let call = prompt
        .messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .next()
        .ok_or("missing replayed call")?;
    let arguments: serde_json::Value = serde_json::from_str(&call.arguments_json)?;
    assert!(
        arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.contains("<PII1:")),
        "preserve valid argument JSON with masked value"
    );
    let denied = Fixture::new_with_rights(
        AgentRole::Orchestrator,
        "fixture/model",
        None,
        Rights::new(&[Right::ExecuteTool]),
    )
    .await?;
    let assembler = denied
        .assembler(
            Arc::new(CountingFixture {
                trim: true,
                ..CountingFixture::default()
            }),
            context(),
            limits(),
        )
        .await?;
    assert!(
        assembler
            .apply(denied.request(
                "goal",
                vec![ChatMessage::text(
                    ChatRole::Assistant,
                    "old exchange".into()
                )]
            ))
            .await
            .is_err(),
        "archive visibility must not replace missing CAS Read capability"
    );
    fixture.kernel.shutdown().await?;
    denied.kernel.shutdown().await?;
    Ok(())
}

struct UnusedVerifier;
impl polaris_runtime::frames::StepVerifier for UnusedVerifier {
    type Error = polaris_runtime::frames::FrameError;
    async fn verify(
        &self,
        _: &polaris_runtime::frames::FrameView,
        _: &polaris_runtime::frames::StepTool,
    ) -> Result<polaris_runtime::frames::Verification, Self::Error> {
        Err(polaris_runtime::frames::FrameError::Accounting)
    }
}

#[tokio::test]
async fn assembler_run_rejects_replaced_frame_selection() -> TestResult {
    use polaris_runtime::frames::{FrameRunInput, FrameRunLimits};
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let mut replacement = Fixture::new_model(
        AgentRole::Orchestrator,
        "fixture/other",
        Some(fixture.frame.view().id),
    )
    .await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let journal = polaris_kernel::KernelFrameJournal::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
        polaris_runtime::frames::FrameJournalLimits::default(),
    )?;
    let cancellation = CancellationToken::new();
    let run = assembler
        .run_frame(
            FrameRunInput {
                frame: &mut replacement.frame,
                goal: "goal",
                cancellation: &cancellation,
                limits: FrameRunLimits {
                    maximum_attempts: 1,
                    maximum_retries: 0,
                    maximum_tool_rounds: 1,
                    max_goal_bytes: 100,
                    max_prompt_bytes: 65536,
                    max_output_bytes: 1024,
                    max_tool_calls: 4,
                    max_tool_argument_bytes: 1024,
                    max_tool_result_bytes: 1024,
                    max_stream_chunks: 4,
                },
            },
            &UnusedVerifier,
            &journal,
        )
        .await;
    assert!(run.is_err(), "different actual frame model/token must fail");
    assert!(
        counter
            .requests
            .lock()
            .map_err(|_| "fixture state")?
            .is_empty(),
        "reject before counting"
    );
    assert_eq!(
        replacement.frame.steps(),
        0,
        "no attempt is reserved for an identity mismatch"
    );
    fixture.kernel.shutdown().await?;
    replacement.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_enforces_deadline_and_cumulative_archive_bound() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let mut policy = limits();
    policy.count_timeout = Duration::from_millis(20);
    let assembler = fixture
        .assembler(
            Arc::new(CountingFixture {
                stall: true,
                ..CountingFixture::default()
            }),
            context(),
            policy,
        )
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        assembler.apply(fixture.request("goal", Vec::new())),
    )
    .await?;
    assert!(
        result.is_err(),
        "stalled tokenizer must fail inside outer observation timeout"
    );
    let history = vec![
        ChatMessage::text(ChatRole::Assistant, "old exchange one".into()),
        ChatMessage::text(ChatRole::Assistant, "old exchange two".into()),
    ];
    let first_bytes = serde_json::to_vec(&serde_json::json!({"format_version":1,"messages":history.get(..1).ok_or("history range")?}))?.len();
    let second_bytes =
        serde_json::to_vec(&serde_json::json!({"format_version":1,"messages":history}))?.len();
    policy.max_archive_bytes = second_bytes;
    let counter = Arc::new(CountingFixture {
        trim: true,
        ..CountingFixture::default()
    });
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), policy)
        .await?;
    let result = assembler.apply(fixture.request("goal", history)).await;
    assert!(
        result.is_err(),
        "two individually fitting archives must not exceed their cumulative allowance"
    );
    assert!(
        first_bytes < second_bytes,
        "fixture must distinguish individual and total limits"
    );
    assert_eq!(
        counter.requests.lock().map_err(|_| "fixture state")?.len(),
        2,
        "reject second archive before an extra network count"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_rejects_sensitive_protocol_identity_and_oversized_context() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let history = vec![
        ChatMessage::assistant_calls(
            String::new(),
            vec![ChatToolCall {
                id: "private@example.com".into(),
                name: "read_file".into(),
                arguments_json: "{}".into(),
            }],
            None,
        ),
        ChatMessage::tool_result("private@example.com".into(), "result".into()),
    ];
    assert!(
        assembler
            .apply(fixture.request("goal", history))
            .await
            .is_err(),
        "sensitive identity must fail rather than leak or silently break correlation"
    );
    assert!(
        counter
            .requests
            .lock()
            .map_err(|_| "fixture state")?
            .is_empty(),
        "no count before privacy rejection"
    );
    let mut oversized = context();
    oversized.tool_names.clear();
    oversized.repo_map.clear();
    oversized.active_scope.clear();
    oversized.repo_map_version.clear();
    oversized.system_prompt = "\"".repeat(500);
    let mut small = limits();
    small.max_input_bytes = 512;
    assert!(
        fixture
            .assembler(Arc::clone(&counter), oversized, small)
            .await
            .is_err(),
        "encoded context must be bounded during construction"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}
fn context() -> PromptContext {
    PromptContext {
        system_prompt: "Follow verified policy".into(),
        repo_map_version: "fixture-v1".into(),
        repo_map: "owner@example.com".into(),
        active_scope: "src/".into(),
        scratchpad: EpistemicScratchpad::default(),
        tool_names: vec![
            "read_file".into(),
            "polaris_expand".into(),
            "polaris_recall".into(),
        ],
    }
}
fn limits() -> PromptLimits {
    PromptLimits {
        max_input_tokens: TokenCount::new(200),
        max_output_tokens: TokenCount::new(32),
        ..PromptLimits::default()
    }
}

#[tokio::test]
async fn assembler_masks_before_counting_and_dispatch() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let prompt = assembler
        .apply(fixture.request(
            "Contact person@example.com",
            vec![ChatMessage::text(
                ChatRole::Assistant,
                "reply private@example.com".into(),
            )],
        ))
        .await?;
    {
        let requests = counter.requests.lock().map_err(|_| "fixture state")?;
        let request = requests.last().ok_or("no actual count call")?;
        assert_eq!(
            prompt.messages, request.messages,
            "dispatch messages must be counted verbatim"
        );
        assert_eq!(
            prompt.tools, request.tools,
            "dispatch tools must be counted verbatim"
        );
        assert_eq!(
            request.model.as_str(),
            "fixture/model",
            "count actual selected model"
        );
        assert_eq!(request.effort, Effort::Low, "count actual effort");
        let serialized = serde_json::to_string(request)?;
        for secret in [
            "owner@example.com",
            "person@example.com",
            "private@example.com",
        ] {
            assert!(
                !serialized.contains(secret),
                "no detected email may reach the counting boundary"
            );
        }
        assert!(
            serialized.contains("<PII1:"),
            "actual reversible vault marker must replace the emails"
        );
        assert_eq!(
            prompt.input_token_upper_bound,
            TokenCount::new(40),
            "retain actual counter result"
        );
        assert_eq!(
            format!("{assembler:?}"),
            "KernelPromptAssembler(<redacted>)",
            "no policy, path, vault or credentials in Debug"
        );
    }
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_filters_role_tools_before_counting() -> TestResult {
    let fixture = Fixture::new(AgentRole::Hacker).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let prompt = assembler
        .apply(fixture.request("Analyze", Vec::new()))
        .await?;
    assert!(
        prompt.tools.is_empty(),
        "Hacker cannot inherit pure catalog classification for persistent CAS/history reads"
    );
    assert!(
        counter
            .requests
            .lock()
            .map_err(|_| "fixture state")?
            .iter()
            .all(|request| request.tools.is_empty()),
        "filter before any count"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_archives_whole_groups_and_recounts() -> TestResult {
    use polaris_runtime::frames::{ToolCall, ToolRunner};
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture {
        trim: true,
        ..CountingFixture::default()
    });
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let mut assistant = ChatMessage::text(ChatRole::Assistant, "old exchange".into());
    assistant.tool_calls.push(ChatToolCall {
        id: "call-1".into(),
        name: "read_file".into(),
        arguments_json: "{\"path\":\"src/lib.rs\"}".into(),
    });
    let mut result = ChatMessage::text(ChatRole::Tool, "old exchange result".into());
    result.tool_call_id = Some("call-1".into());
    let prompt = assembler
        .apply(fixture.request(
            "Continue",
            vec![
                assistant.clone(),
                result.clone(),
                ChatMessage::text(ChatRole::Assistant, "recent".into()),
            ],
        ))
        .await?;
    assert!(
        !prompt
            .messages
            .iter()
            .any(|message| message.content.contains("old exchange")),
        "remove whole group"
    );
    assert!(
        prompt
            .messages
            .iter()
            .any(|message| message.content == "recent"),
        "retain fitting recent history"
    );
    let notice = prompt
        .messages
        .iter()
        .find(|message| {
            message
                .content
                .contains("UNTRUSTED archived conversation JSON")
        })
        .ok_or("missing actual archive notice")?;
    let marker = notice
        .content
        .split_whitespace()
        .find(|part| part.starts_with("§ref:"))
        .ok_or("missing archive marker")?;
    let reference = marker.trim_end_matches('.');
    let permit = fixture
        .runner
        .authorize(
            &fixture.token,
            ToolCall {
                id: "retrieve".into(),
                name: "polaris_expand".into(),
                arguments: serde_json::json!({"reference": reference}).to_string(),
            },
        )
        .await?;
    let expanded = fixture
        .runner
        .execute(permit, fixture.frame.view().id, 64_000)
        .await?;
    let envelope: serde_json::Value = serde_json::from_str(&expanded.content)?;
    let text = envelope
        .get("value")
        .and_then(|value| value.get("text"))
        .and_then(serde_json::Value::as_str)
        .ok_or("archive is not retrievable")?;
    let archive: serde_json::Value = serde_json::from_str(text)?;
    assert_eq!(
        archive.get("messages"),
        Some(&serde_json::to_value(vec![assistant, result])?),
        "lossless complete sanitized exchange"
    );
    {
        let requests = counter.requests.lock().map_err(|_| "fixture state")?;
        assert!(requests.len() >= 2, "recount after archive notice");
        assert_eq!(
            requests.last().ok_or("missing recount")?.messages,
            prompt.messages,
            "notice and final history counted exactly"
        );
    }
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn assembler_rejects_identity_budget_and_protocol_failures() -> TestResult {
    let fixture = Fixture::new(AgentRole::Orchestrator).await?;
    let counter = Arc::new(CountingFixture::default());
    let assembler = fixture
        .assembler(Arc::clone(&counter), context(), limits())
        .await?;
    let mut wrong = fixture.request("goal", Vec::new());
    wrong.frame.id = FrameId::new_v7();
    assert!(
        assembler.apply(wrong).await.is_err(),
        "foreign frame rejected"
    );
    let orphan = ChatMessage::text(ChatRole::Tool, "orphan".into());
    assert!(
        assembler
            .apply(fixture.request("goal", vec![orphan]))
            .await
            .is_err(),
        "malformed tool history rejected"
    );
    assert!(
        counter
            .requests
            .lock()
            .map_err(|_| "fixture state")?
            .is_empty(),
        "reject before network count"
    );
    let mut tiny = limits();
    tiny.max_input_tokens = TokenCount::new(1);
    let tiny_assembler = fixture
        .assembler(Arc::clone(&counter), context(), tiny)
        .await?;
    assert!(
        tiny_assembler
            .apply(fixture.request("goal", Vec::new()))
            .await
            .is_err(),
        "mandatory prefix cannot be silently discarded"
    );
    let unavailable = fixture
        .assembler(
            Arc::new(CountingFixture {
                reject: true,
                ..CountingFixture::default()
            }),
            context(),
            limits(),
        )
        .await?;
    assert!(
        unavailable
            .apply(fixture.request("goal", Vec::new()))
            .await
            .is_err(),
        "no heuristic fallback"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}
