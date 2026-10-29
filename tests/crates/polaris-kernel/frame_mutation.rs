//! Real-workspace mutation integration tests. The safety assessor is an explicit synthetic
//! fixture; capability checks, parser metrics, hybrid verifier, leases and CAS are real.
use polaris_kernel::{
    FrameMutationConfig, FrameMutationRequest, FrameMutationService, Kernel, LocalKernelConfig,
    verifier::VerifierConfig,
};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FrameId, ModelId, SessionId},
};
use polaris_security::capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock,
};
use polaris_system_one::verifier::{
    MutationSafety, SafetyAssessment, SafetyRequest, VerifierError,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy)]
struct SyntheticAllow;

impl MutationSafety for SyntheticAllow {
    fn assess(
        &self,
        _request: &SafetyRequest<'_>,
    ) -> impl std::future::Future<Output = Result<SafetyAssessment, VerifierError>> + Send {
        std::future::ready(Ok(SafetyAssessment::Allow))
    }
}

#[derive(Debug, Clone, Copy)]
struct SyntheticNeedsApproval;

impl MutationSafety for SyntheticNeedsApproval {
    fn assess(
        &self,
        _request: &SafetyRequest<'_>,
    ) -> impl std::future::Future<Output = Result<SafetyAssessment, VerifierError>> + Send {
        std::future::ready(Ok(SafetyAssessment::NeedsApproval {
            reason: polaris_system_one::verifier::DegradedReason::EngineUnavailable,
        }))
    }
}

struct Fixture {
    _state: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: polaris_kernel::KernelHandle,
    authority: Authority,
    frame: AgentContextFrame<Active>,
    token: polaris_security::capability::Token,
}

impl Fixture {
    async fn new(rights: &[Right]) -> TestResult<Self> {
        Self::new_with_resource(rights, ResourceScope::Exact(ResourcePath::new("base.rs")?)).await
    }

    async fn new_with_resource(
        rights: &[Right],
        resource_scope: ResourceScope,
    ) -> TestResult<Self> {
        Self::new_with_role(rights, resource_scope, AgentRole::Orchestrator).await
    }

    async fn new_with_role(
        rights: &[Right],
        resource_scope: ResourceScope,
        role: AgentRole,
    ) -> TestResult<Self> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(workspace.path().join("base.rs"), "fn before() {}\n")?;
        let id = FrameId::new_v7();
        let session = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            state.path(),
            workspace.path(),
            session,
            id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: resource_scope,
            max_churn_lines: 32,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(rights),
        };
        let token = authority.issue_root(id, &scope, &SystemClock)?;
        let expiry = SystemClock
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
                expires_at_micros: expiry,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 8,
                max_depth: 3,
                token_budget: 1000,
            },
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new("test/model")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        Ok(Self {
            _state: state,
            workspace,
            kernel,
            authority,
            frame,
            token,
        })
    }

    fn service<S: MutationSafety + 'static>(
        &self,
        safety: S,
    ) -> TestResult<FrameMutationService<S>> {
        self.service_with_cancel(safety, CancellationToken::new())
    }
    fn service_with_cancel<S: MutationSafety + 'static>(
        &self,
        safety: S,
        cancellation: CancellationToken,
    ) -> TestResult<FrameMutationService<S>> {
        Ok(FrameMutationService::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            self.authority.public_key(),
            Arc::new(safety),
            FrameMutationConfig {
                verifier: VerifierConfig::default(),
                intent: "synthetic test intent".to_owned(),
                cancellation,
            },
        )?)
    }
}

#[tokio::test]
async fn create_new_file_keeps_absence_as_base_and_records_actual_bytes() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let service = fixture.service(SyntheticAllow)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".to_owned(),
                contents: "fn after() {}\n".to_owned(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await?;
    let receipt = service.apply(prepared, &fixture.token).await?;
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn after() {}\n"
    );
    assert_eq!(
        receipt.base(),
        Some(polaris_storage::cas::content_id(b"fn before() {}\n"))
    );
    assert_eq!(
        receipt.new_cid(),
        polaris_storage::cas::content_id(b"fn after() {}\n")
    );
    Ok(())
}

#[tokio::test]
async fn create_new_file_records_no_base_and_never_uses_empty_file_identity() -> TestResult {
    let fixture =
        Fixture::new_with_resource(&[Right::ProposeDiff], ResourceScope::Workspace).await?;
    let service = fixture.service(SyntheticAllow)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "created.rs".to_owned(),
                contents: "fn created() {}\n".to_owned(),
                expected_base: None,
            },
        )
        .await?;
    let receipt = service.apply(prepared, &fixture.token).await?;
    assert_eq!(receipt.base(), None);
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("created.rs"))?,
        b"fn created() {}\n"
    );
    assert_eq!(
        receipt.new_cid(),
        polaris_storage::cas::content_id(b"fn created() {}\n")
    );
    Ok(())
}

#[tokio::test]
async fn needs_approval_returns_without_changing_workspace() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let service = fixture.service(SyntheticNeedsApproval)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".to_owned(),
                contents: "fn after() {}\n".to_owned(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await?;
    let result = service.apply(prepared, &fixture.token).await;
    assert!(matches!(
        result,
        Err(polaris_kernel::FrameMutationError::NeedsApproval { .. })
    ));
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn before() {}\n"
    );
    Ok(())
}

#[tokio::test]
async fn capability_without_propose_diff_is_rejected_before_workspace_change() -> TestResult {
    let fixture = Fixture::new(&[Right::Read]).await?;
    let service = fixture.service(SyntheticAllow)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".to_owned(),
                contents: "fn after() {}\n".to_owned(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await;
    assert!(prepared.is_err());
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn before() {}\n"
    );
    Ok(())
}

#[tokio::test]
async fn changed_base_after_prepare_is_rejected_without_overwrite() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let service = fixture.service(SyntheticAllow)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".to_owned(),
                contents: "fn after() {}\n".to_owned(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await?;
    std::fs::write(
        fixture.workspace.path().join("base.rs"),
        "fn external() {}\n",
    )?;
    assert!(matches!(
        service.apply(prepared, &fixture.token).await,
        Err(polaris_kernel::FrameMutationError::StaleBase)
    ));
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn external() {}\n"
    );
    Ok(())
}

#[tokio::test]
async fn edit_and_patch_requests_produce_and_commit_exact_candidates() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let service = fixture.service(SyntheticAllow)?;
    let edit = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::EditFile {
                path: "base.rs".to_owned(),
                old_text: "before".to_owned(),
                new_text: "edited".to_owned(),
                replace_all: false,
                expected_base: polaris_storage::cas::content_id(b"fn before() {}\n"),
            },
        )
        .await?;
    let receipt = service.apply(edit, &fixture.token).await?;
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn edited() {}\n"
    );
    let patch =
        "--- a/base.rs\n+++ b/base.rs\n@@ -1,1 +1,1 @@\n-fn edited() {}\n+fn patched() {}\n";
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::ApplyPatch {
                path: "base.rs".to_owned(),
                patch: patch.to_owned(),
                expected_base: receipt.new_cid(),
            },
        )
        .await?;
    let result = service.apply(prepared, &fixture.token).await?;
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn patched() {}\n"
    );
    assert_eq!(
        result.new_cid(),
        polaris_storage::cas::content_id(b"fn patched() {}\n")
    );
    Ok(())
}

#[tokio::test]
async fn traversal_secret_and_git_targets_are_rejected() -> TestResult {
    let fixture =
        Fixture::new_with_resource(&[Right::ProposeDiff], ResourceScope::Workspace).await?;
    let service = fixture.service(SyntheticAllow)?;
    for path in ["../outside.rs", ".git/config", ".env"] {
        let result = service
            .prepare(
                &fixture.token,
                FrameMutationRequest::WriteFile {
                    path: path.to_owned(),
                    contents: "fn bad() {}\n".to_owned(),
                    expected_base: None,
                },
            )
            .await;
        assert!(result.is_err(), "target {path} was unexpectedly admitted");
    }
    Ok(())
}

#[tokio::test]
async fn mutation_diff_reconstructs_exact_missing_final_newline() -> TestResult {
    use polaris_kernel::verifier::EvidenceStore;
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let service = fixture.service(SyntheticAllow)?;
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".into(),
                contents: "fn after() {}".into(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await?;
    let receipt = service.apply(prepared, &fixture.token).await?;
    let diff = polaris_kernel::KernelEvidence::new(fixture.kernel.context().clone())
        .get(&receipt.diff())?
        .ok_or("diff missing")?;
    let rebuilt = polaris_tools::builtin::apply_unified_patch(
        b"fn before() {}\n",
        std::str::from_utf8(&diff)?,
    )?;
    assert_eq!(
        rebuilt,
        std::fs::read(fixture.workspace.path().join("base.rs"))?
    );
    assert_eq!(rebuilt, b"fn after() {}");
    Ok(())
}

#[tokio::test]
async fn oversized_source_is_rejected_before_hashing_the_candidate() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.workspace.path().join("base.rs"))?;
    file.set_len(4_194_305)?;
    drop(file);
    let service = fixture.service(SyntheticAllow)?;
    let result = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".into(),
                contents: "fn after() {}\n".into(),
                expected_base: Some(Cid::from_bytes([0; 32])),
            },
        )
        .await;
    assert!(
        matches!(result, Err(polaris_kernel::FrameMutationError::Limit)),
        "oversized source must fail at the read bound"
    );
    Ok(())
}

#[derive(Debug)]
struct SyntheticPausedSafety {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
impl MutationSafety for SyntheticPausedSafety {
    async fn assess(
        &self,
        _request: &SafetyRequest<'_>,
    ) -> Result<SafetyAssessment, VerifierError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(SafetyAssessment::Allow)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_during_verification_prevents_commit() -> TestResult {
    let fixture = Fixture::new(&[Right::ProposeDiff]).await?;
    let cancellation = CancellationToken::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let service = Arc::new(fixture.service_with_cancel(
        SyntheticPausedSafety {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        },
        cancellation.clone(),
    )?);
    let prepared = service
        .prepare(
            &fixture.token,
            FrameMutationRequest::WriteFile {
                path: "base.rs".into(),
                contents: "fn after() {}\n".into(),
                expected_base: Some(polaris_storage::cas::content_id(b"fn before() {}\n")),
            },
        )
        .await?;
    let owner = Arc::clone(&service);
    let token = fixture.token.clone();
    let applying = tokio::spawn(async move { owner.apply(prepared, &token).await });
    tokio::time::timeout(Duration::from_secs(3), entered.notified()).await?;
    cancellation.cancel();
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(3), applying).await??;
    assert!(
        matches!(result, Err(polaris_kernel::FrameMutationError::Binding)),
        "cancelled verifier must not commit"
    );
    assert_eq!(
        std::fs::read(fixture.workspace.path().join("base.rs"))?,
        b"fn before() {}\n"
    );
    Ok(())
}

#[tokio::test]
async fn readonly_frame_roles_cannot_construct_a_mutation_service() -> TestResult {
    for role in [
        AgentRole::Verifier,
        AgentRole::Watchdog,
        AgentRole::Researcher,
        AgentRole::Hacker,
    ] {
        let fixture =
            Fixture::new_with_role(&[Right::ProposeDiff], ResourceScope::Workspace, role).await?;
        assert!(
            fixture.service(SyntheticAllow).is_err(),
            "role {role:?} cannot admit mutation"
        );
        let _report = fixture.kernel.shutdown().await?;
    }
    Ok(())
}
