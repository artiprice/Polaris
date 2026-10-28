#![cfg(windows)]

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use polaris_kernel::{
    ConfiguredFrameServices, FrameCompletionCheck, Kernel, KernelToolRunner, LocalKernelConfig,
    LocalS1Config, PromptAssemblySettings, PromptContext, PromptLimits,
};
use polaris_proto::worker_auth::WorkerCredential;
use polaris_runtime::{
    context::EpistemicScratchpad,
    frames::{
        AgentContextFrame, AgentRole, Cancellation, FrameRunInput, FrameRunLimits, FrameRunStop,
        FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    },
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FrameId, ModelId, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatModel, ChatRequest, ChatRole, DecisionError, FinishReason,
        ModelError, PlatformError, PlatformOps, PromptTokenCount, PromptTokenizer,
    },
    system_one::PiiSpanDetector,
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
};
use polaris_system_one_client::{ClientConfig, ClientError, S1Client};

type TestError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), TestError>;
const PRIVACY_CANARY: &str = "host-privacy-canary@example.com";
const READ_CALL_ID: &str = "configured-host-read-1";

struct ScriptedModel {
    count_requests: Arc<Mutex<Vec<ChatRequest>>>,
    dispatch_requests: Arc<Mutex<Vec<ChatRequest>>>,
    scripts: Mutex<VecDeque<Vec<ChatChunk>>>,
}

struct ScriptedStream(VecDeque<ChatChunk>);

impl ChatChunkSource for ScriptedStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.pop_front().map(Ok)
    }
}

impl PromptTokenizer for ScriptedModel {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        lock_recover(&self.count_requests).push(request.clone());
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(32),
            max_context_tokens: TokenCount::new(4_096),
        })
    }
}

impl ChatModel for ScriptedModel {
    type Stream = ScriptedStream;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        lock_recover(&self.dispatch_requests).push(request);
        let chunks = lock_recover(&self.scripts)
            .pop_front()
            .ok_or_else(|| ModelError::Protocol("synthetic script exhausted".to_owned()))?;
        Ok(ScriptedStream(chunks.into()))
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn unique_pipe() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(PathBuf::from(format!(
        r"\\.\pipe\polaris-host-{}-{elapsed}",
        std::process::id()
    )))
}

fn configured_s1(
    root: &std::path::Path,
) -> Result<LocalS1Config, Box<dyn std::error::Error + Send + Sync>> {
    let model = root.join("empty-model");
    std::fs::create_dir(&model)?;
    let ort = root.join("onnxruntime.dll");
    std::fs::write(&ort, b"health-only fixture; inference is not asserted")?;
    let mut config = LocalS1Config::new(
        env!("CARGO_BIN_EXE_polaris-worker"),
        model,
        ort,
        unique_pipe()?,
    );
    config.startup_timeout = Duration::from_secs(20);
    config.poll_interval = Duration::from_millis(20);
    config.health_interval = Duration::from_millis(50);
    config.health_timeout = Duration::from_secs(2);
    config.stop_grace = Duration::from_secs(5);
    Ok(config)
}

#[tokio::test]
async fn configured_kernel_starts_restarts_and_retires_real_s1_worker() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let s1 = configured_s1(state.path())?;
    let endpoint = s1.endpoint.clone();
    let config = LocalKernelConfig::new(
        state.path(),
        workspace.path(),
        SessionId::new_v7(),
        FrameId::new_v7(),
    )
    .with_s1(s1);
    let kernel = Kernel::start(config).await?;
    let host = kernel
        .s1_host()
        .ok_or("configured kernel omitted its S1 host")?;
    let first = host
        .current_tree()?
        .ok_or("Kernel::start returned before authenticated worker readiness")?;

    let expected_server_pid = first
        .root_pid
        .checked_add(1)
        .ok_or("worker PID cannot be incremented for the rejection fixture")?;
    let wrong_peer = S1Client::connect_named_pipe_owned(
        endpoint,
        ClientConfig::new(Duration::from_secs(2), 262_144)?,
        WorkerCredential::generate()?,
        expected_server_pid,
    )
    .await;
    let wrong_peer_error = match wrong_peer {
        Err(error) => error,
        Ok(_) => return Err("a mismatched named-pipe server PID was accepted".into()),
    };
    assert!(
        matches!(&wrong_peer_error, ClientError::PeerIdentityMismatch),
        "the peer-PID guard must reject before worker RPC: {wrong_peer_error}"
    );
    assert_eq!(
        wrong_peer_error.to_string(),
        "local worker process identity mismatch",
        "identity rejection diagnostics must not expose worker credentials"
    );
    assert_eq!(
        host.current_tree()?,
        Some(first),
        "failed peer identity validation must not affect the owned worker"
    );

    let detector = host.detector();
    let no_pii_model = detector
        .detect_pii_spans("Alice Example at alice@example.test")
        .await;
    assert!(
        matches!(no_pii_model, Err(DecisionError::Unavailable(_))),
        "PII detection must remain unavailable when its optional model is not configured"
    );

    let replacement = host.restart_worker().await?;
    assert_ne!(
        replacement, first,
        "restart must publish a new process-tree generation"
    );
    assert!(matches!(
        kernel.platform().try_wait_tree(&first).await,
        Err(PlatformError::NotFound(_))
    ));

    let shutdown = kernel.shutdown().await?;
    assert!(
        host.current_tree()?.is_none(),
        "shutdown must retire the published worker generation"
    );
    assert!(matches!(
        kernel.platform().try_wait_tree(&replacement).await,
        Err(PlatformError::NotFound(_))
    ));
    drop(shutdown);
    Ok(())
}

#[tokio::test]
async fn invalid_s1_configuration_fails_before_kernel_or_worker_start() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut profile = configured_s1(state.path())?;
    profile.worker_executable = state.path().join("missing-worker.exe");
    let result = Kernel::start(
        LocalKernelConfig::new(
            state.path().join("kernel-state"),
            workspace.path(),
            SessionId::new_v7(),
            FrameId::new_v7(),
        )
        .with_s1(profile),
    )
    .await;
    assert!(
        result.is_err(),
        "a missing worker executable must reject startup"
    );
    assert!(
        !state.path().join("kernel-state").exists(),
        "invalid S1 configuration must fail before opening kernel state"
    );
    Ok(())
}

#[tokio::test]
async fn configured_host_runs_bound_frame_without_inventing_completion_proof() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(
        workspace.path().join("sample.txt"),
        "actual read-only worker fixture\n",
    )?;
    let frame_id = FrameId::new_v7();
    let session_id = SessionId::new_v7();
    let kernel = Kernel::start(
        LocalKernelConfig::new(state.path(), workspace.path(), session_id, frame_id)
            .with_s1(configured_s1(state.path())?),
    )
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
    let mut frame = AgentContextFrame::new_root(
        frame_id,
        AgentRole::Orchestrator,
        token.clone(),
        FrameScope {
            capability: scope,
            expires_at_micros,
        },
        ResourceQuota {
            cpu_millis: 10_000,
            memory_mb: 128,
            network: Default::default(),
            max_steps: 4,
            max_depth: 1,
            token_budget: 10_000,
        },
        ScratchpadRef::new(Cid::from_bytes([3; 32])),
    )
    .activate(
        ModelSelection {
            model: ModelId::new("test/configured-host")?,
            maximum_effort: Effort::Low,
        },
        Effort::Low,
    )?;
    let runner = Arc::new(KernelToolRunner::new(
        kernel.clone(),
        frame.tool_binding(),
        authority.public_key(),
        tokio_util::sync::CancellationToken::new(),
    )?);
    let count_requests = Arc::new(Mutex::new(Vec::new()));
    let dispatch_requests = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(ScriptedModel {
        count_requests: Arc::clone(&count_requests),
        dispatch_requests: Arc::clone(&dispatch_requests),
        scripts: Mutex::new(VecDeque::from([
            vec![
                ChatChunk::ToolCallDelta {
                    index: 0,
                    id: Some(READ_CALL_ID.to_owned()),
                    name: Some("read_file".to_owned()),
                    arguments_fragment: "{\"path\":\"sample.txt\"}".to_owned(),
                },
                ChatChunk::Finished(FinishReason::ToolCalls),
            ],
            vec![
                ChatChunk::Text("The read completed without independent proof.".to_owned()),
                ChatChunk::Finished(FinishReason::Stop),
            ],
        ])),
    });
    let prompt_context = PromptContext {
        system_prompt: format!("Protect {PRIVACY_CANARY}; read the requested source."),
        repo_map_version: "configured-host-fixture-v1".to_owned(),
        repo_map: format!("The private contact is {PRIVACY_CANARY}."),
        active_scope: "sample.txt".to_owned(),
        scratchpad: EpistemicScratchpad::default(),
        tool_names: vec!["read_file".to_owned()],
    };
    let limits = FrameRunLimits {
        maximum_attempts: 2,
        maximum_retries: 0,
        maximum_tool_rounds: 2,
        max_goal_bytes: 512,
        max_prompt_bytes: 32_768,
        max_output_bytes: 4_096,
        max_tool_calls: 4,
        max_tool_argument_bytes: 512,
        max_tool_result_bytes: 8_192,
        max_stream_chunks: 16,
    };
    let cancellation = Cancellation::new();
    let goal = format!("Read sample.txt; keep {PRIVACY_CANARY} private.");
    let outcome = kernel
        .run_agent_frame(
            FrameRunInput {
                frame: &mut frame,
                goal: &goal,
                cancellation: &cancellation,
                limits,
            },
            ConfiguredFrameServices {
                token,
                runner,
                model,
                context: prompt_context,
                prompt: PromptAssemblySettings {
                    limits: PromptLimits::default(),
                    privacy_vault: None,
                },
                role_table: Default::default(),
                semantic_privacy: polaris_kernel::SemanticPrivacyLimits::default(),
            },
        )
        .await?;

    assert_eq!(outcome.execution.tool_rounds, 1);
    assert!(matches!(
        outcome.execution.stop,
        FrameRunStop::ModelTurnComplete
    ));
    assert!(matches!(
        outcome.completion,
        FrameCompletionCheck::Unverified(_)
    ));
    let privacy = outcome
        .semantic_privacy
        .as_ref()
        .ok_or("configured run omitted observed semantic privacy coverage")?;
    assert_eq!(privacy.ner_fields, 0);
    assert!(
        privacy.degraded_fields > 0,
        "unconfigured optional PII model must report DFA fallback coverage"
    );
    assert!(
        outcome.privacy_vault.belongs_to(session_id, frame_id),
        "configured run must retain the marker vault bound to this host session and frame"
    );
    let count_requests = lock_recover(&count_requests);
    assert!(
        !count_requests.is_empty(),
        "the actual prompt tokenizer must run"
    );
    for request in count_requests.iter() {
        assert!(
            !serde_json::to_string(request)?.contains(PRIVACY_CANARY),
            "DFA/semantic masking must precede tokenizer input"
        );
    }
    let dispatch_requests = lock_recover(&dispatch_requests);
    assert_eq!(
        dispatch_requests.len(),
        2,
        "read tool result must drive a follow-up turn"
    );
    for request in dispatch_requests.iter() {
        assert!(
            !serde_json::to_string(request)?.contains(PRIVACY_CANARY),
            "DFA/semantic masking must precede provider dispatch"
        );
    }
    let followup = dispatch_requests
        .get(1)
        .ok_or("missing tool follow-up request")?;
    assert!(
        followup.messages.iter().any(|message| {
            message.role == ChatRole::Tool
                && message.tool_call_id.as_deref() == Some(READ_CALL_ID)
                && message.content.contains("actual read-only worker fixture")
        }),
        "second provider request must contain the actual readonly tool result"
    );
    drop(dispatch_requests);
    drop(count_requests);
    kernel.shutdown().await?;
    Ok(())
}
