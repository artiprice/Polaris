//! Frame-verification integration tests use the real temporary workspace, kernel CAS and event log.
//! No synthetic proof, completion event, or mutation approval is injected.
use polaris_kernel::{FrameVerificationError, Kernel, KernelFrameVerifier, LocalKernelConfig};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    StepTool, StepVerifier, ToolResultRef, Verification,
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FrameId, ModelId, SessionId},
    registry::FinishReason,
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
};
use std::{error::Error, time::Duration};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Fixture {
    _state: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: polaris_kernel::KernelHandle,
    frame: AgentContextFrame<Active>,
    verifier: KernelFrameVerifier,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(workspace.path().join("source.rs"), "fn current() {}\n")?;
        let frame_id = FrameId::new_v7();
        let session_id = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            state.path(),
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
            rights: Rights::new(&[Right::Read]),
        };
        let token = authority.issue_root(frame_id, &scope, &SystemClock)?;
        let expires_at_micros = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
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
                token_budget: 1000,
            },
            ScratchpadRef::new(Cid::from_bytes([7; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new("test/frame-verifier")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        let verifier = KernelFrameVerifier::new(kernel.clone(), frame.tool_binding())?;
        Ok(Self {
            _state: state,
            workspace,
            kernel,
            frame,
            verifier,
        })
    }
}

fn output(text: &str) -> StepTool {
    StepTool {
        reasoning: String::new(),
        finish_reason: FinishReason::Stop,
        text: text.to_owned(),
        tool_results: Vec::new(),
    }
}

#[tokio::test]
async fn ordinary_step_returns_fingerprint_of_actual_manifest() -> TestResult {
    let fixture = Fixture::new().await?;
    let before = std::fs::read(fixture.workspace.path().join("source.rs"))?;
    let events_before = fixture.kernel.context().records(
        polaris_sdk::ids::Seq::new(1),
        polaris_sdk::ids::Seq::new(u64::MAX),
    )?;
    let decision = fixture
        .verifier
        .verify(&fixture.frame.view(), &output("continuing"))
        .await?;
    assert!(
        matches!(decision, Verification::Accepted { verified_diffs, .. } if verified_diffs.is_empty())
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("source.rs"))?,
        before
    );
    assert_eq!(
        fixture.kernel.context().records(
            polaris_sdk::ids::Seq::new(1),
            polaris_sdk::ids::Seq::new(u64::MAX)
        )?,
        events_before,
        "verification must append no event"
    );
    Ok(())
}

#[tokio::test]
async fn wrong_frame_is_rejected_before_using_session_evidence() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut foreign = fixture.frame.view();
    foreign.id = FrameId::new_v7();
    assert!(matches!(
        fixture
            .verifier
            .verify(&foreign, &output("continuing"))
            .await,
        Err(FrameVerificationError::Binding)
    ));
    Ok(())
}

#[tokio::test]
async fn fabricated_mutation_result_without_durable_call_and_approval_is_rejected() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut candidate = output("mutation applied");
    candidate.tool_results.push(polaris_runtime::frames::ExecutedToolResult {
        tool_call_id: "untrusted-call".to_owned(),
        result: ToolResultRef {
            cid: Cid::from_bytes([4; 32]),
            content_cid: Cid::from_bytes([4; 32]),
            content: r#"{"status":"applied","diff":"0101010101010101010101010101010101010101010101010101010101010101"}"#.to_owned(),
        },
    });
    assert!(matches!(
        fixture
            .verifier
            .verify(&fixture.frame.view(), &candidate)
            .await,
        Err(FrameVerificationError::Evidence)
    ));
    Ok(())
}

#[tokio::test]
async fn completion_requires_real_passed_event_for_current_frame_and_tree() -> TestResult {
    let fixture = Fixture::new().await?;
    assert!(matches!(
        fixture
            .verifier
            .validate_completion(&fixture.frame.view())
            .await,
        Err(FrameVerificationError::CompletionUnavailable)
    ));
    Ok(())
}

#[tokio::test]
async fn stale_source_cannot_reuse_a_missing_completion_witness() -> TestResult {
    let fixture = Fixture::new().await?;
    std::fs::write(
        fixture.workspace.path().join("source.rs"),
        "fn changed() {}\n",
    )?;
    assert!(matches!(
        fixture
            .verifier
            .validate_completion(&fixture.frame.view())
            .await,
        Err(FrameVerificationError::CompletionUnavailable)
    ));
    Ok(())
}
