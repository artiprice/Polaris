//! Durable frame-tree cancellation through real sealed runtime identities and kernel stores.
use polaris_kernel::{FrameCancellationService, Kernel, KernelEvidence, LocalKernelConfig};
use polaris_kernel_verifier::FrameCancellationVerifier;
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event},
    effort::Effort,
    ids::{Cid, CommandId, FrameId, ModelId, Seq, SessionId},
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
};
use std::{error::Error, time::Duration};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Fixture {
    _root: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    session: SessionId,
    kernel: polaris_kernel::KernelHandle,
    _authority: Authority,
    root: AgentContextFrame<Active>,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let session = SessionId::new_v7();
        let root_id = FrameId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            root_id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read, Right::SpawnSubagent]),
        };
        let token = authority.issue_root(root_id, &scope, &SystemClock)?;
        let expiry = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
        let root_frame = AgentContextFrame::new_root(
            root_id,
            AgentRole::Orchestrator,
            token,
            FrameScope {
                capability: scope,
                expires_at_micros: expiry,
            },
            quota(),
            ScratchpadRef::new(Cid::from_bytes([3; 32])),
        )
        .activate(model_selection()?, Effort::Low)?;
        Ok(Self {
            _root: root,
            _workspace: workspace,
            session,
            kernel,
            _authority: authority,
            root: root_frame,
        })
    }

    fn service(&self) -> TestResult<FrameCancellationService> {
        Ok(FrameCancellationService::new(
            self.kernel.context().clone(),
            self.session,
            self.root.tool_binding().frame_id(),
            CancellationToken::new(),
        )?)
    }

    fn child(parent: &mut AgentContextFrame<Active>) -> TestResult<AgentContextFrame<Active>> {
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(if parent.view().parent.is_some() {
                60
            } else {
                180
            }),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read, Right::SpawnSubagent]),
        };
        let mut child_quota = parent.remaining_quota();
        child_quota.cpu_millis = child_quota
            .cpu_millis
            .checked_div(2)
            .ok_or("quota division")?;
        child_quota.memory_mb = child_quota
            .memory_mb
            .checked_div(2)
            .ok_or("quota division")?;
        child_quota.token_budget = child_quota
            .token_budget
            .checked_div(2)
            .ok_or("quota division")?;
        let frame = parent.spawn_child(
            AgentRole::SubAgent,
            scope.clone(),
            child_quota,
            ScratchpadRef::new(Cid::from_bytes([4; 32])),
            &SystemClock,
        )?;
        let active = frame.activate(model_selection()?, Effort::Low)?;
        Ok(active)
    }

    fn unverified_cancel(
        &self,
        verifier: &FrameCancellationVerifier,
        target: FrameId,
        command_id: u128,
    ) -> TestResult<Command<polaris_sdk::contracts::Unverified>> {
        let token = verifier.credential(target, Duration::from_secs(60))?;
        Ok(Command::new(
            CommandId::from_u128(command_id),
            self.root.tool_binding().frame_id(),
            token,
            CommandBody::CancelFrame { frame: target },
        ))
    }

    async fn register_root_async(
        &self,
        service: &FrameCancellationService,
    ) -> TestResult<polaris_kernel::FrameCancellationRegistration> {
        Ok(service.register(self.root.tool_binding()).await?)
    }
}

fn quota() -> ResourceQuota {
    ResourceQuota {
        cpu_millis: 10_000,
        memory_mb: 128,
        network: Default::default(),
        max_steps: 8,
        max_depth: 6,
        token_budget: 1000,
    }
}

fn model_selection() -> TestResult<ModelSelection> {
    Ok(ModelSelection {
        model: ModelId::new("test/cancellation-model")?,
        maximum_effort: Effort::Low,
    })
}

#[tokio::test]
async fn frame_cancellation_requires_actual_registered_parent() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let child = Fixture::child(&mut fixture.root)?;
    assert!(service.register(child.tool_binding()).await.is_err());
    fixture.register_root_async(&service).await?;
    let parent = service.register(child.tool_binding()).await?;
    assert_eq!(parent.frame_id(), child.tool_binding().frame_id());
    assert!(!parent.cancellation_token().is_cancelled());
    assert!(service.register(child.tool_binding()).await.is_err());
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn frame_cancellation_cancels_target_descendants_only() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let root_registration = fixture.register_root_async(&service).await?;
    let mut target_frame = Fixture::child(&mut fixture.root)?;
    let target_registration = service.register(target_frame.tool_binding()).await?;
    let grandchild_frame = Fixture::child(&mut target_frame)?;
    let grandchild_registration = service.register(grandchild_frame.tool_binding()).await?;
    let sibling_frame = Fixture::child(&mut fixture.root)?;
    let sibling_registration = service.register(sibling_frame.tool_binding()).await?;
    let target = target_frame.tool_binding().frame_id();
    let verifier =
        FrameCancellationVerifier::new(fixture.session, fixture.root.tool_binding().frame_id())?;
    let command = fixture.unverified_cancel(&verifier, target, 10)?;
    let evidence = KernelEvidence::new(fixture.kernel.context().clone());
    let verified = verifier.verify(command, &evidence)?;
    let outcome = service.apply_verified(verified).await?;
    assert!(outcome.cancelled_frames > 0);
    assert!(target_registration.cancellation_token().is_cancelled());
    assert!(grandchild_registration.cancellation_token().is_cancelled());
    assert!(!sibling_registration.cancellation_token().is_cancelled());
    assert!(!root_registration.cancellation_token().is_cancelled());
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    assert!(records.iter().any(|record| matches!(
        record.event(),
        Event::FrameTreeCancellationRequested { target: actual, .. } if *actual == target
    )));
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn frame_cancellation_rejects_collisions_stale_and_invalid_authority() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    fixture.register_root_async(&service).await?;
    let child = Fixture::child(&mut fixture.root)?;
    let child_id = child.tool_binding().frame_id();
    let _registration = service.register(child.tool_binding()).await?;
    let verifier =
        FrameCancellationVerifier::new(fixture.session, fixture.root.tool_binding().frame_id())?;
    let stale = fixture.unverified_cancel(&verifier, FrameId::new_v7(), 11)?;
    let evidence = KernelEvidence::new(fixture.kernel.context().clone());
    let verified_stale = verifier.verify(stale, &evidence)?;
    assert!(service.apply_verified(verified_stale).await.is_err());
    let _ = child_id;
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn frame_cancellation_rehydrates_pending_and_terminal_states() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    fixture.register_root_async(&service).await?;
    let child = Fixture::child(&mut fixture.root)?;
    let child_id = child.tool_binding().frame_id();
    let child_registration = service.register(child.tool_binding()).await?;
    assert!(!child_registration.cancellation_token().is_cancelled());
    let rehydrated = fixture.service()?;
    assert!(!rehydrated.is_cancelled(child_id)?);
    let verifier =
        FrameCancellationVerifier::new(fixture.session, fixture.root.tool_binding().frame_id())?;
    let command = fixture.unverified_cancel(&verifier, child_id, 12)?;
    let evidence = KernelEvidence::new(fixture.kernel.context().clone());
    let verified = verifier.verify(command, &evidence)?;
    let _outcome = rehydrated.apply_verified(verified).await?;
    let terminal = fixture.service()?;
    assert!(terminal.is_cancelled(child_id)?);
    assert!(terminal.token_for(child_id)?.is_cancelled());
    let _report = fixture.kernel.shutdown().await?;
    drop(service);
    drop(rehydrated);
    drop(terminal);
    drop(evidence);
    let Fixture {
        _root,
        _workspace,
        session,
        kernel,
        root,
        ..
    } = fixture;
    let root_id = root.view().id;
    drop(kernel);
    let resumed = Kernel::start(LocalKernelConfig::new(
        _root.path(),
        _workspace.path(),
        session,
        root_id,
    ))
    .await?;
    let replay = FrameCancellationService::new(
        resumed.context().clone(),
        session,
        root_id,
        CancellationToken::new(),
    )?;
    assert!(
        replay.is_cancelled(child_id)?,
        "a fresh context must replay the persisted child cancellation"
    );
    assert!(
        !replay.is_cancelled(root_id)?,
        "replay must preserve the uncancelled parent"
    );
    resumed.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn frame_cancellation_cancel_race_is_idempotent_and_durable() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    fixture.register_root_async(&service).await?;
    let child = Fixture::child(&mut fixture.root)?;
    let target = child.tool_binding().frame_id();
    let _registration = service.register(child.tool_binding()).await?;
    let verifier =
        FrameCancellationVerifier::new(fixture.session, fixture.root.tool_binding().frame_id())?;
    let command = fixture.unverified_cancel(&verifier, target, 13)?;
    let evidence = KernelEvidence::new(fixture.kernel.context().clone());
    let duplicate = command.clone();
    let verified = verifier.verify(command, &evidence)?;
    let first = service.apply_verified(verified).await?;
    let conflicting = fixture.unverified_cancel(&verifier, target, 13)?;
    assert!(
        service
            .apply_verified(verifier.verify(conflicting, &evidence)?)
            .await
            .is_err(),
        "same command id with changed bearer identity must be rejected"
    );
    let verified_duplicate = verifier.verify(duplicate, &evidence)?;
    assert_eq!(service.apply_verified(verified_duplicate).await?, first);
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let count = records
        .iter()
        .filter(|record| matches!(record.event(), Event::FrameTreeCancellationRequested { target: actual, .. } if *actual == target))
        .count();
    assert_eq!(count, 1);
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cancelling_ancestor_after_child_replays_without_reviving_any_frame() -> TestResult {
    let mut fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let root = fixture.register_root_async(&service).await?;
    let child = Fixture::child(&mut fixture.root)?;
    let child_id = child.view().id;
    service.register(child.tool_binding()).await?;
    let verifier = FrameCancellationVerifier::new(fixture.session, fixture.root.view().id)?;
    let evidence = KernelEvidence::new(fixture.kernel.context().clone());
    service
        .apply_verified(verifier.verify(
            fixture.unverified_cancel(&verifier, child_id, 21)?,
            &evidence,
        )?)
        .await?;
    service
        .apply_verified(verifier.verify(
            fixture.unverified_cancel(&verifier, fixture.root.view().id, 22)?,
            &evidence,
        )?)
        .await?;
    assert!(root.cancellation_token().is_cancelled());
    let replay = fixture.service()?;
    assert!(replay.is_cancelled(child_id)?);
    assert!(replay.is_cancelled(fixture.root.view().id)?);
    assert!(replay.token_for(child_id)?.is_cancelled());
    assert!(
        replay.register(child.tool_binding()).await.is_err(),
        "cancellation cannot be bypassed by registering the same child again"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}
