//! Scripted model/count/S1 fixtures; real frame driver, workspace, Biscuit, CAS and host bus.
use polaris_kernel::{
    FrameMutationConfig, Kernel, KernelAgentToolRunner, KernelFrameJournal, KernelHandle,
    KernelPromptAssembler, LocalKernelConfig, PromptContext, PromptLimits,
    verifier::VerifierConfig,
};
use polaris_runtime::{
    context::EpistemicScratchpad,
    frames::{
        Active, AgentContextFrame, AgentRole, FrameJournalLimits, FrameRunInput, FrameRunLimits,
        FrameRunStop, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef, StepTool,
        StepVerifier, ToolCall, ToolRunner, Verification,
    },
};
use polaris_sdk::{
    contracts::Event,
    effort::Effort,
    human::{HumanQuestionAnswer, HumanResponse},
    ids::{Cid, FastFingerprint, FrameId, ModelId, Seq, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatModel, ChatRequest, FinishReason, ModelError,
        PromptTokenCount, PromptTokenizer,
    },
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use polaris_system_one::verifier::{
    MutationSafety, SafetyAssessment, SafetyRequest, VerifierError,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct SyntheticSafety;
impl MutationSafety for SyntheticSafety {
    async fn assess(&self, _: &SafetyRequest<'_>) -> Result<SafetyAssessment, VerifierError> {
        Ok(SafetyAssessment::Allow)
    }
}
struct ScriptStream(VecDeque<ChatChunk>);
impl ChatChunkSource for ScriptStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.pop_front().map(Ok)
    }
}

enum ConfiguredTestModel {
    Scripted(ScriptModel),
    Live(Arc<polaris_nebius_cloud::token_factory::LlamaCppChatClient>),
}
enum ConfiguredTestStream {
    Scripted(ScriptStream),
    Live(Box<polaris_nebius_cloud::token_factory::ProviderStream>),
}
impl ChatChunkSource for ConfiguredTestStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        match self {
            Self::Scripted(stream) => stream.next_chunk().await,
            Self::Live(stream) => stream.next_chunk().await,
        }
    }
}
impl ChatModel for ConfiguredTestModel {
    type Stream = ConfiguredTestStream;
    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        match self {
            Self::Scripted(model) => model
                .stream(request)
                .await
                .map(ConfiguredTestStream::Scripted),
            Self::Live(model) => model
                .stream(request)
                .await
                .map(Box::new)
                .map(ConfiguredTestStream::Live),
        }
    }
}
impl PromptTokenizer for ConfiguredTestModel {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        match self {
            Self::Scripted(model) => model.count_prompt(request).await,
            Self::Live(model) => model.count_prompt(request).await,
        }
    }
}
struct ScriptModel {
    turns: Mutex<VecDeque<Vec<ChatChunk>>>,
    requests: Mutex<Vec<ChatRequest>>,
}
impl ScriptModel {
    fn new(turns: Vec<Vec<ChatChunk>>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}
impl ChatModel for ScriptModel {
    type Stream = ScriptStream;
    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        self.requests
            .lock()
            .map_err(|_| ModelError::Protocol("fixture state".into()))?
            .push(request);
        let turn = self
            .turns
            .lock()
            .map_err(|_| ModelError::Protocol("fixture state".into()))?
            .pop_front()
            .ok_or_else(|| ModelError::Protocol("unexpected fixture turn".into()))?;
        Ok(ScriptStream(turn.into()))
    }
}
impl PromptTokenizer for ScriptModel {
    async fn count_prompt(&self, _: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        // Explicit synthetic counting oracle; this is not a live tokenizer acceptance test.
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(64),
            max_context_tokens: TokenCount::new(4096),
        })
    }
}
struct ActualSourceVerifier(PathBuf);
impl StepVerifier for ActualSourceVerifier {
    type Error = polaris_sdk::registry::ToolError;
    async fn verify(
        &self,
        _: &polaris_runtime::frames::FrameView,
        _: &StepTool,
    ) -> Result<Verification, Self::Error> {
        let path = self.0.clone();
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
            .await
            .map_err(|_| Self::Error::Io("fixture source owner".into()))?
            .map_err(|_| Self::Error::Io("fixture source read".into()))?;
        let cid = polaris_storage::cas::content_id(&bytes);
        let prefix: [u8; 8] = cid
            .as_bytes()
            .get(..8)
            .ok_or_else(|| Self::Error::Io("fingerprint prefix".into()))?
            .try_into()
            .map_err(|_| Self::Error::Io("fingerprint width".into()))?;
        Ok(Verification::Accepted {
            fingerprint: FastFingerprint::new(u64::from_le_bytes(prefix)),
            verified_diffs: Vec::new(),
        })
    }
}
fn call(name: &str, id: &str, args: serde_json::Value) -> Vec<ChatChunk> {
    vec![
        ChatChunk::ToolCallDelta {
            index: 0,
            id: Some(id.into()),
            name: Some(name.into()),
            arguments_fragment: args.to_string(),
        },
        ChatChunk::Finished(FinishReason::ToolCalls),
    ]
}
fn question() -> serde_json::Value {
    json!({"questions":[{"id":"route","prompt":"Which route should be used?","required":true,"kind":{"type":"free_text"}}]})
}
fn quota(steps: u32, tokens: u32) -> ResourceQuota {
    ResourceQuota {
        cpu_millis: 10_000,
        memory_mb: 256,
        network: Default::default(),
        max_steps: steps,
        max_depth: 4,
        token_budget: u64::from(tokens),
    }
}
fn selection() -> TestResult<ModelSelection> {
    Ok(ModelSelection {
        model: ModelId::new("fixture/scripted-model")?,
        maximum_effort: Effort::Low,
    })
}
struct Fixture {
    _state: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: KernelHandle,
    frame: AgentContextFrame<Active>,
    token: Token,
    authority: Authority,
    scope: Scope,
    cancel: CancellationToken,
    runner: Arc<KernelAgentToolRunner<SyntheticSafety>>,
}
impl Fixture {
    async fn new(rights: &[Right]) -> TestResult<Self> {
        Self::new_with_mutation_command(rights, None).await
    }
    async fn new_with_mutation_command(
        rights: &[Right],
        command: Option<polaris_sdk::ids::CommandId>,
    ) -> TestResult<Self> {
        Self::new_profile(rights, command, None).await
    }
    async fn new_profile(
        rights: &[Right],
        command: Option<polaris_sdk::ids::CommandId>,
        sandbox: Option<(
            Arc<polaris_openshell::client::OpenShellBackend>,
            polaris_sdk::registry::SandboxSpec,
        )>,
    ) -> TestResult<Self> {
        Self::new_selected_profile(rights, command, sandbox, selection()?).await
    }
    async fn new_selected_profile(
        rights: &[Right],
        command: Option<polaris_sdk::ids::CommandId>,
        sandbox: Option<(
            Arc<polaris_openshell::client::OpenShellBackend>,
            polaris_sdk::registry::SandboxSpec,
        )>,
        model: ModelSelection,
    ) -> TestResult<Self> {
        Self::new_lifetime_profile(rights, command, sandbox, model, Duration::from_secs(300)).await
    }
    async fn new_lifetime_profile(
        rights: &[Right],
        command: Option<polaris_sdk::ids::CommandId>,
        sandbox: Option<(
            Arc<polaris_openshell::client::OpenShellBackend>,
            polaris_sdk::registry::SandboxSpec,
        )>,
        model: ModelSelection,
        lifetime: Duration,
    ) -> TestResult<Self> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(workspace.path().join("sample.rs"), b"fn before() {}\n")?;
        let id = FrameId::new_v7();
        let config =
            LocalKernelConfig::new(state.path(), workspace.path(), SessionId::new_v7(), id);
        let kernel = match sandbox {
            Some((backend, spec)) => Kernel::start_sandbox(config, backend, spec).await?,
            None => Kernel::start(config).await?,
        };
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 64,
            ttl: lifetime,
            allow_unsafe: false,
            diff_blake3: match command {
                Some(command) => Some(polaris_kernel::verifier::mutation_digest(
                    command, id, "sample.rs",
                    polaris_storage::cas::content_id(b"--- a/sample.rs\n+++ b/sample.rs\n@@ -1,1 +1,1 @@\n-fn before() {}\n+fn after() {}\n"),
                    polaris_storage::cas::content_id(b"fn before() {}\n"),
                    polaris_storage::cas::content_id(b"fn after() {}\n"),
                )?),
                None => None,
            },
            rights: Rights::new(rights),
        };
        let token = authority.issue_root(id, &scope, &SystemClock)?;
        let expires = SystemClock
            .now()
            .as_microsecond()
            .checked_add(i64::try_from(lifetime.as_micros())?)
            .ok_or("expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            id,
            AgentRole::Orchestrator,
            token.clone(),
            FrameScope {
                capability: scope.clone(),
                expires_at_micros: expires,
            },
            quota(32, 32_000),
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(model, Effort::Low)?;
        let cancel = CancellationToken::new();
        let runner = Arc::new(KernelAgentToolRunner::new(
            kernel.clone(),
            frame.tool_binding(),
            authority.public_key(),
            Arc::new(SyntheticSafety),
            FrameMutationConfig {
                verifier: VerifierConfig::default(),
                intent: "scripted fixture task".into(),
                cancellation: cancel.clone(),
            },
        )?);
        Ok(Self {
            _state: state,
            workspace,
            kernel,
            frame,
            token,
            authority,
            scope,
            cancel,
            runner,
        })
    }
    async fn assembler(
        &self,
        model: Arc<ScriptModel>,
    ) -> TestResult<KernelPromptAssembler<ScriptModel, KernelAgentToolRunner<SyntheticSafety>>>
    {
        Ok(KernelPromptAssembler::new(
            self.kernel.clone(),
            self.frame.prompt_binding()?,
            self.token.clone(),
            Arc::clone(&self.runner),
            model,
            PromptContext {
                system_prompt: "Use only authorized tools.".into(),
                repo_map_version: "fixture-v1".into(),
                repo_map: "sample.rs".into(),
                active_scope: "sample.rs".into(),
                scratchpad: EpistemicScratchpad::default(),
                tool_names: vec![
                    "read_file".into(),
                    "file_revision".into(),
                    "write_file".into(),
                    "edit_file".into(),
                    "apply_patch".into(),
                    "ask_user".into(),
                ],
            },
            PromptLimits {
                max_input_bytes: 262_144,
                max_messages: 64,
                max_tools: 16,
                max_count_requests: 8,
                count_timeout: Duration::from_secs(3),
                max_input_tokens: TokenCount::new(2048),
                max_output_tokens: TokenCount::new(128),
                max_archive_bytes: 262_144,
            },
        )
        .await?)
    }
    fn journal(&self) -> TestResult<KernelFrameJournal> {
        Ok(KernelFrameJournal::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            FrameJournalLimits::default(),
        )?)
    }
}
async fn pending(kernel: &KernelHandle) -> TestResult<polaris_kernel::HumanRequestSnapshot> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(request) = kernel
                .human_requests()
                .list()
                .await?
                .into_iter()
                .find(|r| r.outcome == polaris_kernel::HumanRequestOutcome::Pending)
            {
                return Ok::<_, polaris_kernel::HumanRequestError>(request);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?
    .map_err(Into::into)
}
fn limits() -> FrameRunLimits {
    FrameRunLimits {
        maximum_attempts: 4,
        maximum_retries: 0,
        maximum_tool_rounds: 4,
        max_goal_bytes: 4096,
        max_prompt_bytes: 262_144,
        max_output_bytes: 65_536,
        max_tool_calls: 8,
        max_tool_argument_bytes: 65_536,
        max_tool_result_bytes: 65_536,
        max_stream_chunks: 16,
    }
}

/// Reads only explicitly supplied local test credentials; never prints them or copies them to CAS.
async fn live_test_backend() -> TestResult<(
    Arc<polaris_openshell::client::OpenShellBackend>,
    polaris_sdk::registry::SandboxSpec,
)> {
    use polaris_openshell::client::{GatewayTls, OpenShellBackend, OpenShellConfig};
    let mtls = PathBuf::from(std::env::var("POLARIS_TEST_GATEWAY_MTLS_DIR")?);
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    };
    let config = OpenShellConfig::wsl2(tls);
    let backend = Arc::new(OpenShellBackend::connect(config).await?);
    let policy = polaris_openshell::policy::Policy::parse_yaml(include_str!(
        "../../../@prime/tools/sandbox/acceptance-policy.yaml"
    ))?;
    let policy = backend.register_policy(&policy)?;
    let spec = polaris_sdk::registry::SandboxSpec {
        image: Some(std::env::var("POLARIS_TEST_SANDBOX_IMAGE")?),
        policy: Some(policy),
        cpu_millis: Some(1000),
        memory_mib: Some(512),
        ..Default::default()
    };
    Ok((backend, spec))
}

fn execution_fixture(workspace: &std::path::Path) -> TestResult<polaris_kernel::ExecutionRequest> {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../@prime/tools/sandbox/acceptance-fixture");
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "src/lib.rs",
        "tests/falsifier.rs",
    ] {
        let target = workspace.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source.join(name), target)?;
    }
    Ok(polaris_kernel::ExecutionRequest {
        argv: vec![
            "cargo".into(),
            "test".into(),
            "--offline".into(),
            "--locked".into(),
            "--lib".into(),
        ],
        independent_argv: vec![
            "cargo".into(),
            "test".into(),
            "--offline".into(),
            "--locked".into(),
            "--test".into(),
            "falsifier".into(),
        ],
        timeout_secs: 120,
        min_tests: 3,
    })
}

#[tokio::test]
#[ignore = "requires explicit local mTLS directory, immutable sandbox image and running OpenShell gateway"]
async fn configured_test_tool_waits_for_host_then_proves_actual_source() -> TestResult {
    exercise_configured_test_tool(false).await
}

#[tokio::test]
#[ignore = "requires pinned local llama.cpp, explicit local mTLS and current immutable sandbox image; S1 is a synthetic fixture"]
async fn local_model_edits_and_runs_real_sandbox_with_fixture_safety() -> TestResult {
    exercise_configured_test_tool(true).await
}

async fn exercise_configured_test_tool(live: bool) -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let keepalive = if cfg!(windows) {
        let wsl = polaris_platform::wsl::Wsl::locate()?;
        Some(
            wsl.keepalive(&polaris_platform::wsl::DistroName::new("Ubuntu-24.04")?)
                .await?,
        )
    } else {
        None
    };
    let backend = live_test_backend().await?;
    let (actual_model, selected) = if live {
        let (model, id) = live_llama_model().await?;
        (
            Some(model),
            ModelSelection {
                model: id,
                maximum_effort: Effort::Low,
            },
        )
    } else {
        (None, selection()?)
    };
    let rights: &[Right] = if live {
        &[Right::Read, Right::ExecuteTool, Right::ProposeDiff]
    } else {
        &[Right::Read, Right::ExecuteTool]
    };
    let mut fixture = Fixture::new_selected_profile(rights, None, Some(backend), selected).await?;
    let request = execution_fixture(fixture.workspace.path())?;
    let model = Arc::new(match actual_model {
        Some(model) => ConfiguredTestModel::Live(model),
        None => ConfiguredTestModel::Scripted(ScriptModel::new(vec![
            call(
                "run_tests",
                "ordinary-and-falsifier",
                serde_json::to_value(&request)?,
            ),
            vec![
                ChatChunk::Text("The exact reviewed tests completed.".into()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
        ])),
    });
    let system = if live {
        "State a brief plan, then call file_revision for sample.rs. Use its exact revision with edit_file to replace before with after in sample.rs. Then call run_tests using the exact requested commands. After proof_passed, summarize and stop. Never edit test files."
    } else {
        "Run the fixed fixture tests through the authorized tool."
    };
    let goal = if live {
        format!(
            "Rename before to after in sample.rs, then run the sandbox tests with this exact request: {}",
            serde_json::to_string(&request)?
        )
    } else {
        "Run the approved fixture tests.".to_owned()
    };
    let assembler = KernelPromptAssembler::new(
        fixture.kernel.clone(),
        fixture.frame.prompt_binding()?,
        fixture.token.clone(),
        Arc::clone(&fixture.runner),
        model,
        PromptContext {
            system_prompt: system.into(),
            repo_map_version: "fixture-v1".into(),
            repo_map: "sample.rs contains fn before() {}, src/lib.rs and tests/falsifier.rs contain the fixed acceptance tests.".into(),
            active_scope: "fixture workspace".into(),
            scratchpad: EpistemicScratchpad::default(),
            tool_names: if live {vec!["file_revision".into(),"edit_file".into(),"run_tests".into()]} else {vec!["run_tests".into()]},
        },
        PromptLimits {max_input_tokens:TokenCount::new(3500),max_output_tokens:TokenCount::new(256),count_timeout:Duration::from_secs(60),..PromptLimits::default()},
    )
    .await?;
    let verifier = polaris_kernel::KernelFrameVerifier::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
    )?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let frame_id = fixture.frame.view().id;
    let approve = async {
        let digest =
            tokio::time::timeout(Duration::from_secs(if live { 150 } else { 15 }), async {
                loop {
                    let records = host.context().records(Seq::new(1), Seq::new(u64::MAX))?;
                    if let Some(cid) = records.iter().find_map(|r| match r.event() {
                        Event::BlastRadiusAlert { frame, report } if *frame == frame_id => {
                            Some(*report)
                        }
                        _ => None,
                    }) {
                        return Ok::<_, Box<dyn std::error::Error>>(cid);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await??;
        let bytes = polaris_kernel::KernelEvidence::new(Arc::clone(host.context()))
            .get(&digest)?
            .ok_or("missing review")?;
        let details: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(
            details.get("request"),
            Some(&serde_json::to_value(&request)?),
            "host accepts only the exact fixed fixture commands"
        );
        assert_eq!(details.get("frame"), Some(&json!(frame_id)));
        assert!(
            !host
                .context()
                .records(Seq::new(1), Seq::new(u64::MAX))?
                .iter()
                .any(|r| matches!(r.event(), Event::SandboxExecutionCompleted { .. })),
            "model request cannot execute before host approval"
        );
        host.approve_execution(digest, digest).await?;
        Ok::<_, Box<dyn std::error::Error>>(digest)
    };
    let mut run_limits = limits();
    if live {
        run_limits.max_stream_chunks = 2048;
        run_limits.maximum_attempts = 6;
        run_limits.maximum_tool_rounds = 6;
    }
    let run = async {
        let outcome = assembler
            .run_frame(
                FrameRunInput {
                    frame: &mut fixture.frame,
                    goal: &goal,
                    cancellation: &fixture.cancel,
                    limits: run_limits,
                },
                &verifier,
                &journal,
            )
            .await?;
        verifier.validate_completion(&fixture.frame.view()).await?;
        Ok::<_, Box<dyn std::error::Error>>(outcome)
    };
    let joined = tokio::time::timeout(Duration::from_secs(300), async {
        tokio::try_join!(run, approve)
    })
    .await;
    let result=async {
        let (outcome,review)=joined??;
        assert_eq!(outcome.stop,FrameRunStop::ModelTurnComplete);
        let proof=verifier.validate_completion(&fixture.frame.view()).await?;
        let records=fixture.kernel.context().records(Seq::new(1),Seq::new(u64::MAX))?;
        assert_eq!(records.iter().filter(|r|matches!(r.event(),Event::SandboxExecutionCompleted {frame,..} if *frame==frame_id)).count(),1);
        assert_eq!(records.iter().filter(|r|matches!(r.event(),Event::ProofOfDonePassed {frame,..} if *frame==frame_id)).count(),1);
        assert!(records.iter().any(|r|matches!(r.event(),Event::ToolExecutionCompleted {call,ok:true,..} if *call==review)));
        assert!(records.iter().any(|r|matches!(r.event(),Event::FrameToolResultRecorded {frame,..} if *frame==frame_id)));
        let report=polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context())).get(&proof.report_cid())?.ok_or("missing proof")?;
        let report:serde_json::Value=serde_json::from_slice(&report)?;
        assert_eq!(report.get("review"),Some(&json!(review)));
        assert_eq!(report.get("tree"),Some(&json!(proof.source_cid())));
        if live {
            assert_eq!(std::fs::read(fixture.workspace.path().join("sample.rs"))?, b"fn after() {}\n", "actual S2 edit must reach the workspace before current-source proof");
            assert_eq!(records.iter().filter(|r|matches!(r.event(),Event::DiffApproved {..})).count(),1);
        }
        retain_execution_transcript(&fixture.kernel,&records,&proof,live)?;

        Ok::<_,Box<dyn std::error::Error>>(())
    }.await;
    if result.is_err() {
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../@prime/evidence/phase3-20261027-local/kernel/failed-edit-test");
        std::fs::create_dir_all(&output)?;
        let records = fixture
            .kernel
            .context()
            .records(Seq::new(1), Seq::new(u64::MAX))?;
        let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
        for record in &records {
            if let Event::FrameStepRecorded { transcript, .. } = record.event() {
                let bytes = store.get(transcript)?.ok_or("missing failed transcript")?;
                assert_eq!(polaris_storage::cas::content_id(&bytes), *transcript);
                std::fs::write(output.join(format!("{transcript}.json")), bytes)?;
            }
        }
        std::fs::write(
            output.join(format!("{}-events.json", fixture.frame.view().id)),
            serde_json::to_vec_pretty(&records)?,
        )?;
    }
    let shutdown = fixture.kernel.shutdown().await?;
    assert!(!shutdown.deadline_exceeded);
    assert!(shutdown.journal_error.is_none());
    assert!(
        polaris_kernel::reaper::Reaper::new(fixture.kernel.context().root().join("run"))?
            .leak_report()
            .entries
            .is_empty()
    );
    if let Some(owner) = keepalive {
        owner.shutdown().await?;
    }
    result
}

fn retain_execution_transcript(
    kernel: &KernelHandle,
    records: &[polaris_sdk::contracts::EventRecord],
    proof: &polaris_kernel::VerifiedFrameCompletion,
    live: bool,
) -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    use std::collections::BTreeSet;
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../@prime/evidence/phase3-20261027-local/kernel")
        .join(if live {
            "live-edit-test"
        } else {
            "scripted-test"
        });
    std::fs::create_dir_all(&output)?;
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(kernel.context()));
    let mut pending = vec![proof.report_cid()];
    for record in records {
        match record.event() {
            Event::FrameStepRecorded { transcript, .. } => pending.push(*transcript),
            Event::FrameToolResultRecorded { result, .. } => pending.push(*result),
            _ => {}
        }
    }
    let mut retained = BTreeSet::new();
    while let Some(cid) = pending.pop() {
        if !retained.insert(cid) {
            continue;
        }
        if retained.len() > 128 {
            return Err("fixture evidence exceeds retained-object bound".into());
        }
        let bytes = store
            .get(&cid)?
            .ok_or("missing actual execution evidence")?;
        assert_eq!(polaris_storage::cas::content_id(&bytes), cid);
        // Follow only known evidence fields, never arbitrary model-generated strings or capabilities.
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            for key in [
                "stdout",
                "stderr",
                "independent_report",
                "ordinary_execution",
                "review",
                "content",
                "call",
            ] {
                if let Some(value) = value.get(key).and_then(serde_json::Value::as_str) {
                    pending.push(Cid::from_hex(value)?);
                }
            }
        }
        std::fs::write(output.join(format!("{cid}.blob")), bytes)?;
    }
    let scope = if live {
        "actual local S2/tokenizer, authorized edit, exact host test review, real OpenShell ordinary/falsifier and current-tree proof; semantic S1 assessment is SYNTHETIC"
    } else {
        "scripted S2/tokenizer; real host review, OpenShell ordinary/falsifier and current-tree proof"
    };
    std::fs::write(
        output.join("transcript.json"),
        serde_json::to_vec_pretty(
            &json!({"scope":scope,"source":proof.source_cid(),"proof":proof.report_cid(),"objects":retained,"events":records}),
        )?,
    )?;
    Ok(())
}

async fn live_llama_model() -> TestResult<(
    Arc<polaris_nebius_cloud::token_factory::LlamaCppChatClient>,
    ModelId,
)> {
    use polaris_nebius_cloud::token_factory::{
        BaseUrl, ClientConfig, LlamaCppChatClient, LlamaCppExpectedProfile, ReasoningReplay,
    };
    let model_id = ModelId::new("qwen2.5-1.5b-instruct")?;
    let model = Arc::new(
        LlamaCppChatClient::connect(
            BaseUrl::parse("http://127.0.0.1:18931/v1")?,
            LlamaCppExpectedProfile::new(
                "b11804-c7fc50941",
                model_id.as_str(),
                TokenCount::new(4096),
            )?,
            ClientConfig {
                request_timeout: Duration::from_secs(90),
                connect_timeout: Duration::from_secs(3),
                stream_idle_timeout: Duration::from_secs(60),
                max_sse_frame_bytes: 65536,
                max_events_per_chunk: 64,
                max_chat_stream_bytes: 262144,
                max_text_bytes: 65536,
                max_tool_argument_bytes: 65536,
                max_tool_calls: 8,
                max_json_response_bytes: 262144,
                max_request_bytes: 262144,
                max_429_retries: 0,
                max_5xx_retries: 0,
                retry_base_delay: Duration::from_millis(50),
                max_retry_delay: Duration::from_millis(50),
                reasoning_replay: ReasoningReplay::Disabled,
            },
        )
        .await?,
    );
    Ok((model, model_id))
}

#[tokio::test]
#[ignore = "requires the pinned local llama.cpp b11804 Qwen server on 127.0.0.1:18931"]
async fn local_model_drives_real_revision_tool_and_continuation() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let (model, model_id) = live_llama_model().await?;
    let mut fixture = Fixture::new_selected_profile(
        &[Right::Read],
        None,
        None,
        ModelSelection {
            model: model_id,
            maximum_effort: Effort::Low,
        },
    )
    .await?;
    let assembler=KernelPromptAssembler::new(fixture.kernel.clone(),fixture.frame.prompt_binding()?,fixture.token.clone(),Arc::clone(&fixture.runner),model,
        PromptContext {system_prompt:"Call file_revision once for sample.rs. After receiving the tool result, answer with its revision. Do not call any tool again.".into(),repo_map_version:"live-fixture-v1".into(),repo_map:"sample.rs".into(),active_scope:"sample.rs".into(),scratchpad:EpistemicScratchpad::default(),tool_names:vec!["file_revision".into()]},
        PromptLimits {max_input_tokens:TokenCount::new(3072),max_output_tokens:TokenCount::new(256),count_timeout:Duration::from_secs(60),..PromptLimits::default()}).await?;
    let verifier = polaris_kernel::KernelFrameVerifier::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
    )?;
    let journal = fixture.journal()?;
    let result=async {
        let mut bounds=limits();bounds.max_stream_chunks=2048;
        let outcome=tokio::time::timeout(Duration::from_secs(180),assembler.run_frame(FrameRunInput {frame:&mut fixture.frame,goal:"Use file_revision on sample.rs now, then report that tool's revision.",cancellation:&fixture.cancel,limits:bounds},&verifier,&journal)).await??;
        assert_eq!(outcome.stop,FrameRunStop::ModelTurnComplete);
        let records=fixture.kernel.context().records(Seq::new(1),Seq::new(u64::MAX))?;
        assert_eq!(records.iter().filter(|r|matches!(r.event(),Event::FrameToolResultRecorded {..})).count(),1,"actual model must invoke the real revision tool exactly once");
        assert!(records.iter().any(|r|matches!(r.event(),Event::ToolExecutionCompleted {ok:true,..})),"actual tool must complete durably");
        assert_eq!(std::fs::read(fixture.workspace.path().join("sample.rs"))?,b"fn before() {}\n");
        let output=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../@prime/evidence/phase3-20261027-local/kernel");std::fs::create_dir_all(&output)?;
        let store=polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
        let mut transcripts=Vec::new();
        for record in &records {
            if let Event::FrameStepRecorded {transcript,..}=record.event() {
                let bytes=store.get(transcript)?.ok_or("missing live frame transcript")?;
                assert_eq!(polaris_storage::cas::content_id(&bytes),*transcript);
                let value:serde_json::Value=serde_json::from_slice(&bytes)?;
                assert!(value.get("assistant_response").and_then(|v|v.get("usage")).is_some_and(|v|!v.is_null()),"actual provider usage must be retained for each turn");
                std::fs::write(output.join(format!("{transcript}.json")),&bytes)?;
                transcripts.push(*transcript);
            }
        }
        assert_eq!(transcripts.len(),2,"one tool turn followed by one actual continuation");
        std::fs::write(output.join("live-readonly-frame.json"),serde_json::to_vec_pretty(&json!({"scope":"actual pinned local model, exact tokenizer, real readonly kernel tool and continuation; no mutation or sandbox acceptance","attempts":outcome.attempts,"transcripts":transcripts,"events":records}))?)?;
        Ok::<_,Box<dyn std::error::Error>>(())
    }.await;
    let shutdown = fixture.kernel.shutdown().await?;
    assert!(!shutdown.deadline_exceeded);
    assert!(shutdown.journal_error.is_none());
    result
}

#[tokio::test]
async fn model_loop_mutates_then_waits_for_a_real_host_answer() -> TestResult {
    let mut fixture = Fixture::new(&[Right::Read, Right::ProposeDiff, Right::ExecuteTool]).await?;
    let base = polaris_storage::cas::content_id(b"fn before() {}\n");
    let model = Arc::new(ScriptModel::new(vec![
        call(
            "edit_file",
            "edit-1",
            json!({"path":"sample.rs","old_text":"before","new_text":"after","expected_base":base}),
        ),
        call("ask_user", "ask-1", question()),
        vec![
            ChatChunk::Text("Acknowledged.".into()),
            ChatChunk::Finished(FinishReason::Stop),
        ],
    ]));
    let assembler = fixture.assembler(Arc::clone(&model)).await?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let answer = async {
        let request = pending(&host).await?;
        assert_eq!(
            request.request.origin_step, 2,
            "question must use the reserved actual driver step"
        );
        assert_eq!(
            std::fs::read(fixture.workspace.path().join("sample.rs"))?,
            b"fn after() {}\n"
        );
        host.answer_human_request(
            request.ticket.request_id,
            request.ticket.revision,
            vec![HumanQuestionAnswer {
                question_id: polaris_sdk::human::HumanQuestionId::new("route")?,
                response: HumanResponse::FreeText {
                    text: "Use the reviewed route.".into(),
                },
                notes: None,
            }],
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let run = assembler.run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "Edit and clarify.",
            cancellation: &fixture.cancel,
            limits: limits(),
        },
        &source,
        &journal,
    );
    let (outcome, answered) = tokio::join!(run, answer);
    answered?;
    let outcome = outcome?;
    assert_eq!(outcome.stop, FrameRunStop::ModelTurnComplete);
    assert_eq!(outcome.attempts, 3);
    {
        let requests = model
            .requests
            .lock()
            .map_err(|_| "model requests poisoned")?;
        let last = requests.last().ok_or("missing continuation prompt")?;
        assert!(
            last.messages
                .iter()
                .any(|m| m.content.contains("Use the reviewed route.")),
            "actual answer must reach model continuation"
        );
        assert!(
            requests
                .first()
                .is_some_and(|r| r.tools.iter().any(|t| t.name.as_ref() == "ask_user")),
            "bound runner schema must reach first prompt"
        );
    }
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.event(), Event::DiffApproved { .. }))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.event(), Event::ToolExecutionStarted { .. }))
            .count(),
        2
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.event(), Event::ToolExecutionCompleted { ok: true, .. }))
            .count(),
        2
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn truncated_human_answer_is_retrievable_only_by_its_runner() -> TestResult {
    let mut fixture = Fixture::new(&[Right::Read, Right::ExecuteTool]).await?;
    let model = Arc::new(ScriptModel::new(vec![
        call("ask_user", "large-answer", question()),
        vec![
            ChatChunk::Text("Answer recorded.".into()),
            ChatChunk::Finished(FinishReason::Stop),
        ],
    ]));
    let assembler = fixture.assembler(Arc::clone(&model)).await?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let text = "reviewed detail ".repeat(600);
    let answers = vec![HumanQuestionAnswer {
        question_id: polaris_sdk::human::HumanQuestionId::new("route")?,
        response: HumanResponse::FreeText { text: text.clone() },
        notes: None,
    }];
    let answer = async {
        let request = pending(&host).await?;
        host.answer_human_request(request.ticket.request_id, request.ticket.revision, answers)
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let mut bounds = limits();
    bounds.max_tool_result_bytes = 512;
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let run = assembler.run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "Ask the host for the route.",
            cancellation: &fixture.cancel,
            limits: bounds,
        },
        &source,
        &journal,
    );
    let (outcome, answered) = tokio::join!(run, answer);
    let result = async {
        answered?;
        assert_eq!(outcome?.stop, FrameRunStop::ModelTurnComplete);
        let reply = {
            let requests = model
                .requests
                .lock()
                .map_err(|_| "fixture requests poisoned")?;
            let request = requests.last().ok_or("missing answer continuation")?;
            request
                .messages
                .iter()
                .find(|m| m.role == polaris_sdk::registry::ChatRole::Tool)
                .ok_or("missing tool answer")?
                .content
                .clone()
        };
        let reply: serde_json::Value = serde_json::from_str(&reply)?;
        assert_eq!(reply.get("truncated"), Some(&json!(true)));
        let cid: Cid = serde_json::from_value(
            reply
                .get("answer_cid")
                .ok_or("missing answer reference")?
                .clone(),
        )?;
        let expand = || ToolCall {
            id: "expand-answer".into(),
            name: "polaris_expand".into(),
            arguments: json!({"reference":cid.to_hex()}).to_string(),
        };
        let peer = KernelAgentToolRunner::new(
            fixture.kernel.clone(),
            fixture.frame.tool_binding(),
            fixture.authority.public_key(),
            Arc::new(SyntheticSafety),
            FrameMutationConfig {
                verifier: VerifierConfig::default(),
                intent: "peer fixture".into(),
                cancellation: CancellationToken::new(),
            },
        )?;
        assert!(
            peer.authorize(&fixture.token, expand()).await.is_err(),
            "another runner must not inherit private answer visibility"
        );
        let permit = fixture.runner.authorize(&fixture.token, expand()).await?;
        let expanded = fixture
            .runner
            .execute(permit, fixture.frame.view().id, 65536)
            .await?;
        assert!(
            expanded.content.contains(&text),
            "the actual host answer must remain losslessly retrievable"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    fixture.kernel.shutdown().await?;
    result
}

#[tokio::test]
async fn human_wait_rechecks_expired_execute_authority() -> TestResult {
    let mut fixture = Fixture::new_lifetime_profile(
        &[Right::Read, Right::ExecuteTool],
        None,
        None,
        selection()?,
        Duration::from_secs(2),
    )
    .await?;
    let model = Arc::new(ScriptModel::new(vec![
        call("ask_user", "expired-answer", question()),
        vec![
            ChatChunk::Text("This continuation must not run.".into()),
            ChatChunk::Finished(FinishReason::Stop),
        ],
    ]));
    let assembler = fixture.assembler(Arc::clone(&model)).await?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let answer = async {
        let request = pending(&host).await?;
        tokio::time::sleep(Duration::from_millis(2100)).await;
        host.answer_human_request(
            request.ticket.request_id,
            request.ticket.revision,
            vec![HumanQuestionAnswer {
                question_id: polaris_sdk::human::HumanQuestionId::new("route")?,
                response: HumanResponse::FreeText {
                    text: "Real host answer after capability expiry.".into(),
                },
                notes: None,
            }],
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let run = assembler.run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "Ask for the route.",
            cancellation: &fixture.cancel,
            limits: limits(),
        },
        &source,
        &journal,
    );
    let (outcome, answered) = tokio::join!(run, answer);
    let result = async {
        answered?;
        assert!(
            outcome.is_err(),
            "expired execution authority must not deliver a successful tool result"
        );
        let requests = model
            .requests
            .lock()
            .map_err(|_| "fixture requests poisoned")?;
        assert_eq!(
            requests.len(),
            1,
            "no continuation may use an unauthorized result"
        );
        drop(requests);
        let records = fixture
            .kernel
            .context()
            .records(Seq::new(1), Seq::new(u64::MAX))?;
        assert!(
            records
                .iter()
                .any(|r| matches!(r.event(), Event::HumanRequestAnswered { .. })),
            "actual host answer remains durable"
        );
        assert!(
            records
                .iter()
                .any(|r| matches!(r.event(), Event::ToolExecutionCompleted { ok: false, .. })),
            "failed tool completion remains durable"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    fixture.kernel.shutdown().await?;
    result
}

#[tokio::test]
async fn denied_later_call_prevents_every_mutation() -> TestResult {
    let mut fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let base = polaris_storage::cas::content_id(b"fn before() {}\n");
    let mut turn = call(
        "edit_file",
        "edit-denied-batch",
        json!({"path":"sample.rs","old_text":"before","new_text":"after","expected_base":base}),
    );
    let _finish = turn.pop();
    turn.push(ChatChunk::ToolCallDelta {
        index: 1,
        id: Some("ask-denied-batch".into()),
        name: Some("ask_user".into()),
        arguments_fragment: question().to_string(),
    });
    turn.push(ChatChunk::Finished(FinishReason::ToolCalls));
    let assembler = fixture
        .assembler(Arc::new(ScriptModel::new(vec![turn])))
        .await?;
    let journal = fixture.journal()?;
    let result = assembler
        .run_frame(
            FrameRunInput {
                frame: &mut fixture.frame,
                goal: "Denied batch.",
                cancellation: &fixture.cancel,
                limits: limits(),
            },
            &ActualSourceVerifier(fixture.workspace.path().join("sample.rs")),
            &journal,
        )
        .await;
    assert!(
        result.is_err(),
        "missing ExecuteTool must deny the entire call set"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn before() {}\n"
    );
    assert!(fixture.kernel.human_requests().list().await?.is_empty());
    assert!(
        !fixture
            .kernel
            .context()
            .records(Seq::new(1), Seq::new(u64::MAX))?
            .iter()
            .any(|r| matches!(
                r.event(),
                Event::DiffApproved { .. } | Event::ToolExecutionStarted { .. }
            ))
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn dropping_a_waiting_call_persists_request_interruption() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ExecuteTool]).await?;
    let assembler = fixture
        .assembler(Arc::new(ScriptModel::new(vec![call(
            "ask_user",
            "ask-drop",
            question(),
        )])))
        .await?;
    let journal = fixture.journal()?;
    let mut frame = fixture.frame;
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let cancel = fixture.cancel.clone();
    let task = tokio::spawn(async move {
        assembler
            .run_frame(
                FrameRunInput {
                    frame: &mut frame,
                    goal: "Clarify.",
                    cancellation: &cancel,
                    limits: limits(),
                },
                &source,
                &journal,
            )
            .await
    });
    let request = pending(&fixture.kernel).await?;
    task.abort();
    let stopped = task.await;
    assert!(stopped.is_err(), "caller must actually have been dropped");
    let outcome = tokio::time::timeout(
        Duration::from_secs(4),
        fixture
            .kernel
            .human_requests()
            .wait(request.ticket.request_id, CancellationToken::new()),
    )
    .await??;
    assert!(matches!(
        outcome,
        polaris_kernel::HumanRequestOutcome::Interrupted { .. }
    ));
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn waiting_question_allows_sibling_workspace_reads() -> TestResult {
    let mut fixture =
        Fixture::new(&[Right::Read, Right::ExecuteTool, Right::SpawnSubagent]).await?;
    let mut child_scope = fixture.scope.clone();
    child_scope.ttl = Duration::from_secs(60);
    let child = fixture
        .frame
        .spawn_child(
            AgentRole::SubAgent,
            child_scope,
            quota(4, 2000),
            ScratchpadRef::new(Cid::from_bytes([2; 32])),
            &SystemClock,
        )?
        .activate(selection()?, Effort::Low)?;
    let sibling = KernelAgentToolRunner::new(
        fixture.kernel.clone(),
        child.tool_binding(),
        fixture.authority.public_key(),
        Arc::new(SyntheticSafety),
        FrameMutationConfig {
            verifier: VerifierConfig::default(),
            intent: "sibling fixture".into(),
            cancellation: CancellationToken::new(),
        },
    )?;
    let assembler = fixture
        .assembler(Arc::new(ScriptModel::new(vec![call(
            "ask_user",
            "ask-sibling",
            question(),
        )])))
        .await?;
    let journal = fixture.journal()?;
    let mut frame = fixture.frame;
    let cancel = fixture.cancel.clone();
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let task = tokio::spawn(async move {
        assembler
            .run_frame(
                FrameRunInput {
                    frame: &mut frame,
                    goal: "Wait.",
                    cancellation: &cancel,
                    limits: limits(),
                },
                &source,
                &journal,
            )
            .await
    });
    let request = pending(&fixture.kernel).await?;
    let permit = sibling
        .authorize(
            child.capability_token(),
            ToolCall {
                id: "sibling-read".into(),
                name: "read_file".into(),
                arguments: json!({"path":"sample.rs"}).to_string(),
            },
        )
        .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        sibling.execute(permit, child.view().id, 8192),
    )
    .await??;
    assert!(result.content.contains("before"));
    assert_eq!(
        fixture
            .kernel
            .human_requests()
            .outcome(request.ticket.request_id)?,
        Some(polaris_kernel::HumanRequestOutcome::Pending)
    );
    fixture.cancel.cancel();
    let _ended = tokio::time::timeout(Duration::from_secs(4), task).await??;
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[derive(Debug)]
struct HeldSafety {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl MutationSafety for HeldSafety {
    async fn assess(&self, _: &SafetyRequest<'_>) -> Result<SafetyAssessment, VerifierError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(SafetyAssessment::Allow)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_runners_share_one_workspace_mutation_lease() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let safety = Arc::new(HeldSafety {
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
        calls: Arc::clone(&calls),
    });
    let make_runner = || {
        KernelAgentToolRunner::new(
            fixture.kernel.clone(),
            fixture.frame.tool_binding(),
            fixture.authority.public_key(),
            Arc::clone(&safety),
            FrameMutationConfig {
                verifier: VerifierConfig::default(),
                intent: "lease contention fixture".into(),
                cancellation: CancellationToken::new(),
            },
        )
    };
    let a = Arc::new(make_runner()?);
    let b = Arc::new(make_runner()?);
    let base = polaris_storage::cas::content_id(b"fn before() {}\n");
    let proposal = |id: &str, name: &str| ToolCall {
        id: id.into(),
        name: "edit_file".into(),
        arguments:
            json!({"path":"sample.rs","old_text":"before","new_text":name,"expected_base":base})
                .to_string(),
    };
    let pa = a.authorize(&fixture.token, proposal("a", "first")).await?;
    let pb = b.authorize(&fixture.token, proposal("b", "second")).await?;
    let frame = fixture.frame.view().id;
    let a_task = tokio::spawn(async move { a.execute(pa, frame, 8192).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified()).await?;
    let b_task = tokio::spawn(async move { b.execute(pb, frame, 8192).await });
    let _second = tokio::time::timeout(Duration::from_millis(200), entered.notified()).await;
    release.notify_waiters();
    let a_result = tokio::time::timeout(Duration::from_secs(3), a_task).await??;
    let b_result = tokio::time::timeout(Duration::from_secs(3), b_task).await??;
    assert!(a_result.is_ok(), "first holder must commit");
    let b_content = b_result
        .map(|result| result.content)
        .map_err(|error| format!("a stale base is reported to the model, not raised: {error}"))?;
    assert!(
        b_content.contains("\"rejected\"") && b_content.contains("stale"),
        "second holder must observe the committed new base: {b_content}"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "independent runners must share actual lease ownership before semantic assessment"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn first() {}\n"
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn write_requires_explicit_absence_or_revision() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let permit = fixture
        .runner
        .authorize(
            &fixture.token,
            ToolCall {
                id: "missing-base".into(),
                name: "write_file".into(),
                arguments: json!({"path":"created.rs","contents":"fn created() {}\n"}).to_string(),
            },
        )
        .await?;
    let result = fixture
        .runner
        .execute(permit, fixture.frame.view().id, 8192)
        .await?;
    assert!(
        result.content.contains("\"rejected\""),
        "omitting expected_base must not silently authorize create-new: {}",
        result.content
    );
    assert!(!fixture.workspace.path().join("created.rs").exists());
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn mutation_result_budget_is_rejected_during_admission() -> TestResult {
    let mut fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let base = polaris_storage::cas::content_id(b"fn before() {}\n");
    let assembler = fixture
        .assembler(Arc::new(ScriptModel::new(vec![call(
            "edit_file",
            "small-budget",
            json!({"path":"sample.rs","old_text":"before","new_text":"after","expected_base":base}),
        )])))
        .await?;
    let journal = fixture.journal()?;
    let mut budget = limits();
    budget.max_tool_result_bytes = 512;
    let result = assembler
        .run_frame(
            FrameRunInput {
                frame: &mut fixture.frame,
                goal: "Bounded edit.",
                cancellation: &fixture.cancel,
                limits: budget,
            },
            &ActualSourceVerifier(fixture.workspace.path().join("sample.rs")),
            &journal,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(polaris_runtime::frames::FrameError::Backend(
                polaris_sdk::error::ErrorKind::Policy,
                "tool authorization"
            ))
        ),
        "known receipt budget failure belongs in whole-set admission"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn before() {}\n"
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn shutdown_interrupts_and_drains_waiting_request_owners() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ExecuteTool]).await?;
    let assembler = fixture
        .assembler(Arc::new(ScriptModel::new(vec![call(
            "ask_user",
            "ask-shutdown",
            question(),
        )])))
        .await?;
    let journal = fixture.journal()?;
    let mut frame = fixture.frame;
    let cancel = fixture.cancel.clone();
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let task = tokio::spawn(async move {
        assembler
            .run_frame(
                FrameRunInput {
                    frame: &mut frame,
                    goal: "Wait for shutdown.",
                    cancellation: &cancel,
                    limits: limits(),
                },
                &source,
                &journal,
            )
            .await
    });
    let request = pending(&fixture.kernel).await?;
    let _report = tokio::time::timeout(Duration::from_secs(4), fixture.kernel.shutdown()).await??;
    let ended = tokio::time::timeout(Duration::from_secs(2), task).await??;
    assert!(ended.is_err(), "shutdown must end the frame operation");
    assert!(matches!(
        fixture
            .kernel
            .human_requests()
            .outcome(request.ticket.request_id)?,
        Some(polaris_kernel::HumanRequestOutcome::Interrupted { .. })
    ));
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    assert_eq!(records.iter().filter(|r|matches!(r.event(),Event::HumanRequestInterrupted { request_id,.. } if *request_id==request.ticket.request_id)).count(),1);
    assert_eq!(
        records
            .iter()
            .filter(|r| matches!(r.event(), Event::ToolExecutionCompleted { ok: false, .. }))
            .count(),
        1
    );
    Ok(())
}
fn held_service(
    fixture: &Fixture,
    safety: Arc<HeldSafety>,
    cancel: CancellationToken,
) -> TestResult<Arc<polaris_kernel::FrameMutationService<HeldSafety>>> {
    Ok(Arc::new(polaris_kernel::FrameMutationService::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
        fixture.authority.public_key(),
        safety,
        FrameMutationConfig {
            verifier: VerifierConfig {
                safety_timeout: Duration::from_secs(20),
                ..VerifierConfig::default()
            },
            intent: "held mutation fixture".into(),
            cancellation: cancel,
        },
    )?))
}
fn candidate(name: &str) -> polaris_kernel::FrameMutationRequest {
    polaris_kernel::FrameMutationRequest::EditFile {
        path: "sample.rs".into(),
        old_text: "before".into(),
        new_text: name.into(),
        replace_all: false,
        expected_base: polaris_storage::cas::content_id(b"fn before() {}\n"),
    }
}
#[tokio::test]
async fn contended_mutations_do_not_reserve_all_read_slots() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let service = held_service(
        &fixture,
        Arc::new(HeldSafety {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        CancellationToken::new(),
    )?;
    let first = service.prepare(&fixture.token, candidate("held")).await?;
    let mut waiting = Vec::new();
    for _ in 0..64 {
        waiting.push(service.prepare(&fixture.token, candidate("queued")).await?);
    }
    let first_owner = Arc::clone(&service);
    let first_token = fixture.token.clone();
    let first_task = tokio::spawn(async move { first_owner.apply(first, &first_token).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified()).await?;
    let began = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for prepared in waiting {
        let owner = Arc::clone(&service);
        let token = fixture.token.clone();
        let started = Arc::clone(&began);
        tasks.push(tokio::spawn(async move {
            started.fetch_add(1, std::sync::atomic::Ordering::Release);
            owner.apply(prepared, &token).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while began.load(std::sync::atomic::Ordering::Acquire) < 64 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let read = tokio::time::timeout(
        Duration::from_millis(200),
        fixture
            .kernel
            .invoke_tool("read_file", json!({"path":"sample.rs"})),
    )
    .await;
    release.notify_waiters();
    let first_result = tokio::time::timeout(Duration::from_secs(5), first_task).await??;
    for task in tasks {
        let result = tokio::time::timeout(Duration::from_secs(5), task).await??;
        assert!(
            result.is_err(),
            "queued base must be stale after first commit"
        );
    }
    first_result?;
    assert!(
        matches!(read, Ok(Ok(_))),
        "queued writers must not consume immutable-read slots"
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn cancelling_a_queued_mutation_does_not_wait_for_current_holder() -> TestResult {
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let first = held_service(
        &fixture,
        Arc::new(HeldSafety {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        CancellationToken::new(),
    )?;
    let cancelled = CancellationToken::new();
    let second = Arc::new(polaris_kernel::FrameMutationService::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
        fixture.authority.public_key(),
        Arc::new(SyntheticSafety),
        FrameMutationConfig {
            verifier: VerifierConfig::default(),
            intent: "cancel queued fixture".into(),
            cancellation: cancelled.clone(),
        },
    )?);
    let a = first.prepare(&fixture.token, candidate("held")).await?;
    let b = second.prepare(&fixture.token, candidate("queued")).await?;
    let token = fixture.token.clone();
    let first_task = tokio::spawn(async move { first.apply(a, &token).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified()).await?;
    let began = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = Arc::clone(&began);
    let token = fixture.token.clone();
    let mut second_task = tokio::spawn(async move {
        started.store(true, std::sync::atomic::Ordering::Release);
        second.apply(b, &token).await
    });
    while !began.load(std::sync::atomic::Ordering::Acquire) {
        tokio::task::yield_now().await;
    }
    cancelled.cancel();
    let early = tokio::time::timeout(Duration::from_millis(200), &mut second_task).await;
    release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(5), first_task).await???;
    let prompt = match early {
        Ok(result) => {
            assert!(result?.is_err(), "cancelled queue entry must reject");
            true
        }
        Err(_) => {
            let result = tokio::time::timeout(Duration::from_secs(5), second_task).await??;
            assert!(result.is_err());
            false
        }
    };
    assert!(
        prompt,
        "queued cancellation must complete before the current holder is released"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn held() {}\n"
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn targeted_runner_cancel_preserves_shared_parent_and_peer() -> TestResult {
    let mut fixture =
        Fixture::new(&[Right::Read, Right::ExecuteTool, Right::SpawnSubagent]).await?;
    let mut scope = fixture.scope.clone();
    scope.ttl = Duration::from_secs(60);
    let mut child_quota = quota(4, 2000);
    child_quota.memory_mb = 32;
    child_quota.cpu_millis = 1000;
    let first = fixture
        .frame
        .spawn_child(
            AgentRole::SubAgent,
            scope.clone(),
            child_quota.clone(),
            ScratchpadRef::new(Cid::from_bytes([3; 32])),
            &SystemClock,
        )?
        .activate(selection()?, Effort::Low)?;
    let peer = fixture
        .frame
        .spawn_child(
            AgentRole::SubAgent,
            scope,
            child_quota,
            ScratchpadRef::new(Cid::from_bytes([4; 32])),
            &SystemClock,
        )?
        .activate(selection()?, Effort::Low)?;
    let parent = CancellationToken::new();
    let runner = |frame: &AgentContextFrame<Active>| {
        KernelAgentToolRunner::new(
            fixture.kernel.clone(),
            frame.tool_binding(),
            fixture.authority.public_key(),
            Arc::new(SyntheticSafety),
            FrameMutationConfig {
                verifier: VerifierConfig::default(),
                intent: "isolated child fixture".into(),
                cancellation: parent.clone(),
            },
        )
    };
    let first_runner = runner(&first)?;
    let peer_runner = runner(&peer)?;
    first_runner.cancel();
    assert!(
        !parent.is_cancelled(),
        "runner cancellation must never propagate to a supplied parent"
    );
    let read = || ToolCall {
        id: "peer-read".into(),
        name: "read_file".into(),
        arguments: json!({"path":"sample.rs"}).to_string(),
    };
    assert!(
        first_runner
            .authorize(first.capability_token(), read())
            .await
            .is_err()
    );
    let permit = peer_runner
        .authorize(peer.capability_token(), read())
        .await?;
    let result = peer_runner.execute(permit, peer.view().id, 8192).await?;
    assert!(
        result.content.contains("before"),
        "peer must remain operational"
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn run_tests_without_execution_profile_fails_before_activity() -> TestResult {
    use polaris_kernel::FrameToolAccess;
    let fixture = Fixture::new(&[Right::Read, Right::ExecuteTool]).await?;
    assert!(
        fixture.runner.tool_specification("run_tests").is_some(),
        "real test tool must be exposed"
    );
    let result = fixture.runner.authorize(&fixture.token, ToolCall {
        id:"test-no-profile".into(), name:"run_tests".into(),
        arguments: json!({"argv":["cargo","test"],"independent_argv":["cargo","test","--test","falsifier"],"timeout_secs":60,"min_tests":1}).to_string(),
    }).await;
    assert!(
        matches!(result, Err(polaris_sdk::registry::ToolError::Forbidden(_))),
        "unconfigured execution must be a policy denial during admission"
    );
    assert!(
        !fixture
            .kernel
            .context()
            .records(Seq::new(1), Seq::new(u64::MAX))?
            .iter()
            .any(|r| matches!(
                r.event(),
                Event::ToolExecutionStarted { .. } | Event::BlastRadiusAlert { .. }
            )),
        "rejected admission must not start activity or persist reviews"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn custom_results_bind_exact_call_and_content() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let fixture = Fixture::new(&[Right::Read]).await?;
    let permit = fixture
        .runner
        .authorize(
            &fixture.token,
            ToolCall {
                id: "revision-identity".into(),
                name: "file_revision".into(),
                arguments: json!({"path":"sample.rs"}).to_string(),
            },
        )
        .await?;
    let result = fixture
        .runner
        .execute(permit, fixture.frame.view().id, 65536)
        .await?;
    assert_ne!(
        result.cid, result.content_cid,
        "evidence envelope must be separate from model-visible content"
    );
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
    let raw = store.get(&result.cid)?.ok_or("missing result evidence")?;
    let envelope: serde_json::Value = serde_json::from_slice(&raw)?;
    assert_eq!(envelope.get("version").ok_or("missing version")?, 1);
    assert_eq!(
        envelope.get("frame").ok_or("missing frame")?,
        &json!(fixture.frame.view().id)
    );
    assert_eq!(
        envelope.get("content").ok_or("missing content")?,
        &json!(result.content_cid)
    );
    let call: Cid = serde_json::from_value(envelope.get("call").ok_or("missing call")?.clone())?;
    let call_bytes = store.get(&call)?.ok_or("missing call")?;
    let stored: serde_json::Value = serde_json::from_slice(&call_bytes)?;
    assert_eq!(stored.get("id").ok_or("missing id")?, "revision-identity");
    assert_eq!(
        stored
            .get("arguments")
            .and_then(|v| v.get("path"))
            .ok_or("missing argument path")?,
        "sample.rs"
    );
    assert_eq!(
        store.get(&result.content_cid)?.ok_or("missing content")?,
        result.content.as_bytes()
    );
    assert!(fixture.kernel.context().records(Seq::new(1),Seq::new(u64::MAX))?.iter().any(|r|matches!(r.event(),Event::ToolExecutionCompleted {call:actual,ok:true,..} if *actual==call)),"result must correspond to a successful durable execution");
    fixture.kernel.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn mutation_command_id_preserves_exact_diff_authority() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let command = polaris_sdk::ids::CommandId::new_v7();
    let fixture =
        Fixture::new_with_mutation_command(&[Right::Read, Right::ProposeDiff], Some(command))
            .await?;
    let permit=fixture.runner.authorize(&fixture.token,ToolCall {id:"exact-edit".into(),name:"edit_file".into(),arguments:json!({"path":"sample.rs","old_text":"before","new_text":"after","expected_base":polaris_storage::cas::content_id(b"fn before() {}\n"),"command_id":command}).to_string()}).await?;
    let result = fixture
        .runner
        .execute(permit, fixture.frame.view().id, 65536)
        .await?;
    let content: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(content.get("status").ok_or("missing status")?, "applied");
    let receipt: Cid =
        serde_json::from_value(content.get("receipt").ok_or("missing receipt")?.clone())?;
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
    let raw = store.get(&receipt)?.ok_or("missing receipt")?;
    let receipt: serde_json::Value = serde_json::from_slice(&raw)?;
    assert_eq!(
        receipt.get("command").ok_or("missing command")?,
        &json!(command),
        "trusted exact-diff command identity must reach the verifier unchanged"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn after() {}\n"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn host_cancels_waiting_child_preserving_root_and_sibling() -> TestResult {
    let mut fixture =
        Fixture::new(&[Right::Read, Right::ExecuteTool, Right::SpawnSubagent]).await?;
    let mut child_scope = fixture.scope.clone();
    child_scope.ttl = Duration::from_secs(60);
    let mut child_quota = quota(4, 2000);
    child_quota.memory_mb = 32;
    child_quota.cpu_millis = 1000;
    let child = fixture
        .frame
        .spawn_child(
            AgentRole::SubAgent,
            child_scope,
            child_quota,
            ScratchpadRef::new(Cid::from_bytes([5; 32])),
            &SystemClock,
        )?
        .activate(selection()?, Effort::Low)?;
    let runner = Arc::new(KernelAgentToolRunner::new(
        fixture.kernel.clone(),
        child.tool_binding(),
        fixture.authority.public_key(),
        Arc::new(SyntheticSafety),
        FrameMutationConfig {
            verifier: VerifierConfig::default(),
            intent: "targeted host cancellation".into(),
            cancellation: fixture.cancel.clone(),
        },
    )?);
    let question_model = Arc::new(ScriptModel::new(vec![call(
        "ask_user",
        "cancel-me",
        question(),
    )]));
    let assembler = KernelPromptAssembler::new(
        fixture.kernel.clone(),
        child.prompt_binding()?,
        child.capability_token().clone(),
        Arc::clone(&runner),
        question_model,
        PromptContext {
            system_prompt: "Clarify".into(),
            repo_map_version: "v1".into(),
            repo_map: "sample.rs".into(),
            active_scope: "sample.rs".into(),
            scratchpad: EpistemicScratchpad::default(),
            tool_names: vec!["ask_user".into()],
        },
        PromptLimits::default(),
    )
    .await?;
    let journal = KernelFrameJournal::new(
        fixture.kernel.clone(),
        child.tool_binding(),
        FrameJournalLimits::default(),
    )?;
    let host = fixture.kernel.clone();
    let target = child.view().id;
    let task = tokio::spawn(async move {
        let mut child = child;
        assembler
            .run_frame(
                FrameRunInput {
                    frame: &mut child,
                    goal: "Clarify",
                    cancellation: &CancellationToken::new(),
                    limits: limits(),
                },
                &ActualSourceVerifier(PathBuf::from("unused")),
                &journal,
            )
            .await
    });
    let pending = pending(&host).await?;
    let receipt = host.cancel_frame(target).await?;
    assert!(receipt.accepted_at.get() > 0);
    let result = tokio::time::timeout(Duration::from_secs(3), task).await??;
    assert!(
        result.is_err() || result.is_ok_and(|r| r.stop != FrameRunStop::ModelTurnComplete),
        "cancelled child must not complete a model turn"
    );
    assert!(
        matches!(
            host.human_requests()
                .list()
                .await?
                .into_iter()
                .find(|r| r.ticket.request_id == pending.ticket.request_id)
                .ok_or("missing interrupted request")?
                .outcome,
            polaris_kernel::HumanRequestOutcome::Interrupted { .. }
        ),
        "host acknowledgement must include durable pending-question interruption"
    );
    assert!(
        !fixture.cancel.is_cancelled(),
        "target cancellation must preserve caller parent"
    );
    let permit = fixture
        .runner
        .authorize(
            &fixture.token,
            ToolCall {
                id: "root-still-progresses".into(),
                name: "read_file".into(),
                arguments: json!({"path":"sample.rs"}).to_string(),
            },
        )
        .await?;
    fixture
        .runner
        .execute(permit, fixture.frame.view().id, 65536)
        .await?;
    host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn edit_file_tolerates_whitespace_drift_and_explains_misses() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let base = polaris_storage::cas::content_id(b"fn before() {}\n");
    let frame = fixture.frame.view().id;
    let call = |id: &str, old_text: &str| {
        ToolCall {
        id: id.into(),
        name: "edit_file".into(),
        arguments: json!({"path":"sample.rs","old_text":old_text,"new_text":"fn after() {}","expected_base":base}).to_string(),
    }
    };

    let permit = fixture
        .runner
        .authorize(&fixture.token, call("miss", "fn missing() {}"))
        .await?;
    let missed = fixture.runner.execute(permit, frame, 8192).await?;
    assert!(
        missed.content.contains("\"rejected\""),
        "{}",
        missed.content
    );
    assert!(missed.content.contains("not found"), "{}", missed.content);
    assert!(
        missed.content.contains("1| fn before() {}"),
        "the hint shows the closest line: {}",
        missed.content
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn before() {}\n",
        "a missed edit changes nothing"
    );

    let permit = fixture
        .runner
        .authorize(&fixture.token, call("drift", "fn before() {}   \n"))
        .await?;
    let applied = fixture.runner.execute(permit, frame, 8192).await?;
    let value: serde_json::Value = serde_json::from_str(&applied.content)?;
    assert_eq!(value["status"], "applied", "{}", applied.content);
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("sample.rs"))?,
        b"fn after() {}\n"
    );
    let receipt: polaris_sdk::ids::Cid = serde_json::from_value(value["receipt"].clone())?;
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
    let stored: serde_json::Value =
        serde_json::from_slice(&store.get(&receipt)?.ok_or("missing receipt")?)?;
    assert_eq!(stored["schema"], 2);
    assert_eq!(stored["match_level"], "trailing_whitespace");
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn production_verifier_accepts_real_mutation_and_rejects_forged_link() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let fixture = Fixture::new(&[Right::Read, Right::ProposeDiff]).await?;
    let permit=fixture.runner.authorize(&fixture.token,ToolCall {id:"verified-mutation".into(),name:"edit_file".into(),arguments:json!({"path":"sample.rs","old_text":"before","new_text":"after","expected_base":polaris_storage::cas::content_id(b"fn before() {}\n")}).to_string()}).await?;
    let result = fixture
        .runner
        .execute(permit, fixture.frame.view().id, 65536)
        .await?;
    let verifier = polaris_kernel::KernelFrameVerifier::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
    )?;
    let mut step = StepTool {
        reasoning: String::new(),
        finish_reason: FinishReason::ToolCalls,
        text: String::new(),
        tool_results: vec![polaris_runtime::frames::ExecutedToolResult {
            tool_call_id: "verified-mutation".into(),
            result: result.clone(),
        }],
    };
    let verified = verifier.verify_step(&fixture.frame.view(), &step).await?;
    assert_eq!(
        verified.verified_diffs().len(),
        1,
        "actual successful mutation must contribute its exact diff"
    );
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
    let raw = store.get(&result.cid)?.ok_or("missing envelope")?;
    let mut envelope: serde_json::Value = serde_json::from_slice(&raw)?;
    let mut changed: serde_json::Value = serde_json::from_str(&result.content)?;
    *changed.get_mut("path").ok_or("missing mutation path")? = json!("other.rs");
    let content = changed.to_string();
    let content_cid = store.put(content.as_bytes())?;
    *envelope
        .get_mut("content")
        .ok_or("missing envelope content")? = json!(content_cid);
    let forged = store.put(&serde_json::to_vec(&envelope)?)?;
    step.tool_results = vec![polaris_runtime::frames::ExecutedToolResult {
        tool_call_id: "verified-mutation".into(),
        result: polaris_runtime::frames::ToolResultRef {
            cid: forged,
            content_cid,
            content,
        },
    }];
    assert!(
        verifier
            .verify_step(&fixture.frame.view(), &step)
            .await
            .is_err(),
        "unrecorded CAS envelope cannot substitute a result"
    );
    step.tool_results = vec![polaris_runtime::frames::ExecutedToolResult {
        tool_call_id: "verified-mutation".into(),
        result,
    }];
    std::fs::write(
        fixture.workspace.path().join("sample.rs"),
        b"fn externally_changed() {}\n",
    )?;
    assert!(
        verifier
            .verify_step(&fixture.frame.view(), &step)
            .await
            .is_err(),
        "stale approval must not verify changed source"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

impl Fixture {
    /// Swap in a runner that puts escalated changes to the host as review questions.
    fn review_changes(&mut self) -> TestResult {
        self.runner = Arc::new(
            KernelAgentToolRunner::new(
                self.kernel.clone(),
                self.frame.tool_binding(),
                self.authority.public_key(),
                Arc::new(SyntheticSafety),
                FrameMutationConfig {
                    verifier: VerifierConfig::default(),
                    intent: "scripted fixture task".into(),
                    cancellation: self.cancel.clone(),
                },
            )?
            .with_change_review(),
        );
        Ok(())
    }
}

fn review_answer(
    question: &polaris_sdk::human::HumanQuestion,
    option: &str,
    notes: Option<&str>,
) -> TestResult<Vec<HumanQuestionAnswer>> {
    Ok(vec![HumanQuestionAnswer {
        question_id: question.id.clone(),
        response: HumanResponse::Single {
            selection: polaris_sdk::human::HumanSelection::Option(
                polaris_sdk::human::HumanOptionId::new(option)?,
            ),
        },
        notes: notes.map(str::to_owned),
    }])
}

#[tokio::test]
async fn escalated_change_waits_for_review_then_applies_with_human_evidence() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let mut fixture = Fixture::new(&[Right::Read, Right::ProposeDiff, Right::ExecuteTool]).await?;
    fixture.review_changes()?;
    let model = Arc::new(ScriptModel::new(vec![
        call(
            "write_file",
            "write-notes",
            json!({"path":"notes.md","contents":"# Notes\n\nhello\n","expected_base":null}),
        ),
        vec![
            ChatChunk::Text("Written.".into()),
            ChatChunk::Finished(FinishReason::Stop),
        ],
    ]));
    let assembler = fixture.assembler(Arc::clone(&model)).await?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let notes = fixture.workspace.path().join("notes.md");
    let answer = async {
        let request = pending(&host).await?;
        let question = request.request.questions.first().ok_or("no question")?;
        assert!(
            question
                .id
                .as_str()
                .starts_with(polaris_kernel::CHANGE_REVIEW_PREFIX)
        );
        assert!(
            question.prompt.starts_with("Create `notes.md`?")
                && question.prompt.contains("```diff")
                && question.prompt.contains("+hello"),
            "{}",
            question.prompt
        );
        assert!(!notes.exists(), "nothing is written before the answer");
        host.answer_human_request(
            request.ticket.request_id,
            request.ticket.revision,
            review_answer(question, polaris_kernel::CHANGE_REVIEW_APPLY, None)?,
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let run = assembler.run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "Write the notes.",
            cancellation: &fixture.cancel,
            limits: limits(),
        },
        &source,
        &journal,
    );
    let (outcome, answered) = tokio::join!(run, answer);
    answered?;
    assert_eq!(outcome?.stop, FrameRunStop::ModelTurnComplete);
    assert_eq!(std::fs::read(&notes)?, b"# Notes\n\nhello\n");

    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let result = records
        .iter()
        .find_map(|r| match r.event() {
            Event::FrameToolResultRecorded { result, .. } => Some(*result),
            _ => None,
        })
        .ok_or("no recorded tool result")?;
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(fixture.kernel.context()));
    let envelope: serde_json::Value =
        serde_json::from_slice(&store.get(&result)?.ok_or("missing envelope")?)?;
    let content_cid: Cid = serde_json::from_value(envelope["content"].clone())?;
    let content = String::from_utf8(store.get(&content_cid)?.ok_or("missing content")?)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    assert_eq!(value["status"], "applied", "{value}");
    let approval: Cid = serde_json::from_value(value["approval"].clone())?;
    let evidence: serde_json::Value =
        serde_json::from_slice(&store.get(&approval)?.ok_or("missing evidence")?)?;
    assert_eq!(evidence["safety_result"], "human_approved");
    assert_eq!(evidence["analysis"], "text");

    let verifier = polaris_kernel::KernelFrameVerifier::new(
        fixture.kernel.clone(),
        fixture.frame.tool_binding(),
    )?;
    let step = StepTool {
        reasoning: String::new(),
        finish_reason: FinishReason::ToolCalls,
        text: String::new(),
        tool_results: vec![polaris_runtime::frames::ExecutedToolResult {
            tool_call_id: "write-notes".into(),
            result: polaris_runtime::frames::ToolResultRef {
                cid: result,
                content_cid,
                content,
            },
        }],
    };
    let verified = verifier.verify_step(&fixture.frame.view(), &step).await?;
    assert_eq!(verified.verified_diffs().len(), 1);
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn declined_change_writes_nothing_and_returns_the_reason_to_the_model() -> TestResult {
    let mut fixture = Fixture::new(&[Right::Read, Right::ProposeDiff, Right::ExecuteTool]).await?;
    fixture.review_changes()?;
    let model = Arc::new(ScriptModel::new(vec![
        call(
            "write_file",
            "write-notes",
            json!({"path":"notes.md","contents":"hello\n","expected_base":null}),
        ),
        vec![
            ChatChunk::Text("Understood.".into()),
            ChatChunk::Finished(FinishReason::Stop),
        ],
    ]));
    let assembler = fixture.assembler(Arc::clone(&model)).await?;
    let journal = fixture.journal()?;
    let host = fixture.kernel.clone();
    let answer = async {
        let request = pending(&host).await?;
        let question = request.request.questions.first().ok_or("no question")?;
        host.answer_human_request(
            request.ticket.request_id,
            request.ticket.revision,
            review_answer(
                question,
                polaris_kernel::CHANGE_REVIEW_DECLINE,
                Some("Name it TODO.md instead."),
            )?,
        )
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let source = ActualSourceVerifier(fixture.workspace.path().join("sample.rs"));
    let run = assembler.run_frame(
        FrameRunInput {
            frame: &mut fixture.frame,
            goal: "Write the notes.",
            cancellation: &fixture.cancel,
            limits: limits(),
        },
        &source,
        &journal,
    );
    let (outcome, answered) = tokio::join!(run, answer);
    answered?;
    assert_eq!(outcome?.stop, FrameRunStop::ModelTurnComplete);
    assert!(!fixture.workspace.path().join("notes.md").exists());
    {
        let requests = model
            .requests
            .lock()
            .map_err(|_| "model requests poisoned")?;
        let last = requests.last().ok_or("missing continuation prompt")?;
        assert!(
            last.messages.iter().any(|m| m.content.contains("declined")
                && m.content.contains("Name it TODO.md instead.")),
            "the decline and the note must reach the model"
        );
    }
    fixture.kernel.shutdown().await?;
    Ok(())
}
