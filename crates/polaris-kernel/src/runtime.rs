//! P1 local composition: bounded command dispatch, signed controls and durable replay.
use crate::{
    Created, KernelCtx, KernelError, KernelEvidence, Running, Session, SessionConfig,
    bus::{BusError, EventStream},
    context::blocking,
    reaper::ShutdownReport,
    sandbox_recovery::SandboxRecovery,
    verifier::LocalDirectiveVerifier,
};
use polaris_devenv::docker::{Docker, DockerEndpoint};
use polaris_sdk::{
    contracts::{Command, CommandBody, EventRecord, Unverified},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi, KernelError as ApiError},
};
use polaris_tools::builtin::ToolAccessPolicy;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedSemaphorePermit, Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

const MAX_PENDING: usize = 1024;
const MAX_CONTROL_BYTES: usize = 65_536;
const MAX_INSPECTION_OWNERS: usize = 64;
type Reply = oneshot::Sender<Result<CommandReceipt, ApiError>>;

/// Explicit local roots and identities; no remote endpoint or ambient credential is required.
#[derive(Debug, Clone)]
pub struct LocalKernelConfig {
    /// Durable local state root.
    pub root: PathBuf,
    /// Existing workspace directory.
    pub workspace: PathBuf,
    /// Session to create or recover.
    pub session: SessionId,
    /// Root control frame.
    pub frame: FrameId,
    /// Optional authenticated local System-One worker profile.
    pub s1: Option<crate::s1_host::LocalS1Config>,
}
impl LocalKernelConfig {
    /// Build a local-only composition configuration.
    #[must_use]
    pub fn new(
        root: impl AsRef<Path>,
        workspace: impl AsRef<Path>,
        session: SessionId,
        frame: FrameId,
    ) -> Self {
        Self {
            root: root.as_ref().to_owned(),
            workspace: workspace.as_ref().to_owned(),
            session,
            frame,
            s1: None,
        }
    }

    /// Configure a supervised local S1 worker for this kernel.
    #[must_use]
    pub fn with_s1(mut self, config: crate::s1_host::LocalS1Config) -> Self {
        self.s1 = Some(config);
        self
    }
}

/// P1 backend bundle: durable local stores, session control authority and command verifier.
/// S2, S1 workers and sandbox choices enter the execution profile in P2/P3.
#[derive(Debug)]
pub struct LocalBackends {
    control: Arc<LocalDirectiveVerifier>,
    platform: Arc<polaris_platform::process::ProcessPlatform>,
    tools: Arc<crate::LocalTools>,
}

/// Local kernel composition root. Owns one mutable session in a single command consumer.
#[derive(Debug)]
pub struct Kernel;
impl Kernel {
    /// Start storage/recovery, configure or replay the session, then expose command admission.
    /// # Errors
    /// Store, journal, replay, configuration or authority failure. No handle escapes failed startup.
    pub async fn start(config: LocalKernelConfig) -> Result<KernelHandle, KernelError> {
        Self::start_inner(config, None, None, None).await
    }
    /// Start with an explicit Docker endpoint, recovering Docker pool orphans before session admission.
    ///
    /// # Errors
    /// Docker connection or cleanup failure, unresolved non-Docker resources, store, replay,
    /// configuration, or authority failure. No handle escapes failed startup.
    pub async fn start_with_docker(
        config: LocalKernelConfig,
        endpoint: DockerEndpoint,
    ) -> Result<KernelHandle, KernelError> {
        let docker = Docker::connect(&endpoint).await?;
        Self::start_inner(config, None, Some(docker), None).await
    }
    /// Start the P2 execution profile with an explicit live OpenShell backend and image policy.
    /// # Errors
    /// Invalid image/policy, authority, store, recovery or session startup failure.
    pub async fn start_sandbox(
        config: LocalKernelConfig,
        backend: Arc<polaris_openshell::client::OpenShellBackend>,
        spec: polaris_sdk::registry::SandboxSpec,
    ) -> Result<KernelHandle, KernelError> {
        let recovery_backend = Arc::clone(&backend);
        let recovery: Arc<dyn SandboxRecovery> = recovery_backend;
        let execution = Arc::new(crate::SandboxExecution::new(
            backend,
            spec,
            config.session,
            config.frame,
        )?);
        Self::start_inner(config, Some(execution), None, Some(recovery)).await
    }
    async fn start_inner(
        config: LocalKernelConfig,
        execution: Option<
            Arc<crate::SandboxExecution<polaris_openshell::client::OpenShellBackend>>,
        >,
        docker: Option<Docker>,
        sandbox_recovery: Option<Arc<dyn SandboxRecovery>>,
    ) -> Result<KernelHandle, KernelError> {
        let s1_config = config.s1.clone();
        if let Some(profile) = s1_config.as_ref() {
            profile.validate()?;
        }
        let control = Arc::new(
            LocalDirectiveVerifier::new(config.session, config.frame)
                .map_err(|_| KernelError::Configuration("local authority unavailable"))?,
        );
        let platform = Arc::new(polaris_platform::process::ProcessPlatform::new());
        let ctx = match (docker.as_ref(), sandbox_recovery.as_deref()) {
            (Some(docker), _) => KernelCtx::open_with_docker(&config.root, docker).await?,
            (None, Some(backend)) => KernelCtx::open_with_sandbox(&config.root, backend).await?,
            (None, None) => KernelCtx::open(&config.root).await?,
        };
        let existing = {
            let ctx = Arc::clone(&ctx);
            let id = config.session;
            blocking(move || {
                for record in ctx.bus.store().replay_iter(Seq::new(u64::MAX))? {
                    let record = record?;
                    if record.session() == id {
                        return Ok(true);
                    }
                }
                Ok(false)
            })
            .await?
        };
        let session = if existing {
            Session::<crate::Archived>::reopen(config.session, Arc::clone(&ctx))
                .await?
                .resume()
                .await?
        } else {
            let context = Arc::clone(&ctx);
            let id = config.session;
            let workspace = config.workspace;
            blocking(move || {
                Session::<Created>::create(id, context)?.configure(SessionConfig {
                    name: "Local session".to_owned(),
                    workspace,
                    branch: String::new(),
                    base_commit: String::new(),
                    defaults: Vec::new(),
                })
            })
            .await?
            .start()
            .await?
        };
        let workspace = session
            .configuration()
            .ok_or(KernelError::Integrity(
                "active session has no configuration",
            ))?
            .workspace
            .clone();
        let backends = LocalBackends {
            control,
            platform,
            tools: Arc::new(
                crate::tools::build(Arc::clone(&ctx), config.session)
                    .map_err(|_| KernelError::Configuration("local tool registry unavailable"))?,
            ),
        };
        let human_verifier = crate::HumanRequestVerifier::new(config.session, config.frame)
            .map_err(|_| KernelError::Configuration("human answer authority unavailable"))?;
        let human_context = Arc::clone(&ctx);
        let human_session = config.session;
        let human_frame = config.frame;
        let human_requests = Arc::new(
            blocking(move || {
                crate::HumanRequestService::new(human_context, human_session, human_frame)
                    .map_err(|_| KernelError::Integrity("human request replay failed"))
            })
            .await?,
        );
        let commands = ctx.take_commands()?;
        let workspace_leases = match execution.as_ref() {
            Some(execution) => execution.workspace_leases(),
            None => polaris_runtime::leases::LeaseManager::new(
                polaris_runtime::leases::LeaseConfig::default(),
            ),
        };
        let stop = CancellationToken::new();
        let cancellation_context = Arc::clone(&ctx);
        let cancellation_session = config.session;
        let cancellation_root = config.frame;
        let cancellation_stop = stop.clone();
        let frame_cancellations = Arc::new(
            blocking(move || {
                crate::FrameCancellationService::new(
                    cancellation_context,
                    cancellation_session,
                    cancellation_root,
                    cancellation_stop,
                )
                .map_err(|_| KernelError::Integrity("frame cancellation replay failed"))
            })
            .await?,
        );
        // A crash can leave a durable cancellation intent before request interruption.
        for request in human_requests
            .list()
            .await
            .map_err(|_| KernelError::Integrity("human request replay snapshot failed"))?
        {
            if request.outcome == crate::HumanRequestOutcome::Pending
                && frame_cancelled_or_unregistered(&frame_cancellations, request.ticket.frame)?
            {
                let _interrupted = human_requests
                    .interrupt_frame(request.ticket.frame)
                    .await
                    .map_err(|_| KernelError::Integrity("cancelled request recovery failed"))?;
            }
        }
        let cancellation_verifier = Arc::new(
            crate::verifier::FrameCancellationVerifier::new(config.session, config.frame).map_err(
                |_| KernelError::Configuration("frame cancellation authority unavailable"),
            )?,
        );
        let s1_host = if let Some(profile) = s1_config {
            match crate::s1_host::LocalS1Host::start(
                profile,
                Arc::clone(&backends.platform),
                Arc::clone(&ctx.bus),
                config.session,
                stop.clone(),
            )
            .await
            {
                Ok(host) => Some(Arc::new(host)),
                Err(error) => {
                    if let Err(cleanup) = ctx.shutdown().await {
                        tracing::error!(
                            error = %cleanup,
                            "kernel cleanup failed after S1 host startup error"
                        );
                        return Err(KernelError::Runtime(
                            "S1 worker startup and kernel cleanup both failed".to_owned(),
                        ));
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let shared = Arc::new(Shared {
            ctx,
            backends,
            workspace_leases: WorkspaceLeases(workspace_leases),
            human_requests,
            frame_cancellations,
            cancellation_verifier,
            human_verifier: Arc::new(human_verifier),
            execution,
            s1_host,
            frame: config.frame,
            session: config.session,
            workspace,
            pending: Mutex::new(BTreeMap::new()),
            closed: AtomicBool::new(false),
            handles: AtomicUsize::new(1),
            stop,
            finished: CancellationToken::new(),
            task: tokio::sync::Mutex::new(None),
            shutdown_result: Mutex::new(None),
            inspections: Arc::new(tokio::sync::RwLock::new(())),
            inspection_admission: Arc::new(Semaphore::new(MAX_INSPECTION_OWNERS)),
            frame_operation_admission: Arc::new(Semaphore::new(64)),
            frame_operations: Arc::new(tokio::sync::RwLock::new(())),
        });
        let owner = Arc::clone(&shared);
        let task = tokio::spawn(async move {
            let _completion = OwnerCompletion(Arc::clone(&owner));
            let dispatcher = Arc::clone(&owner);
            let inner = tokio::spawn(async move { run(dispatcher, commands, session).await });
            let result = match inner.await {
                Ok(result) => result,
                Err(_) => Err(KernelError::Runtime(
                    "kernel command owner panicked or stopped".to_owned(),
                )),
            };
            owner.inspection_admission.close();
            let _operations = owner.frame_operations.write().await;
            let _drained = owner.inspections.write().await;
            let s1_cleanup = match owner.s1_host.as_ref() {
                Some(host) => host.shutdown().await.map(|_| ()),
                None => Ok(()),
            };
            // This coordinator survives a dispatcher panic/early return and owns async cleanup.
            let report = owner.ctx.shutdown().await;
            match result {
                Ok(()) => match s1_cleanup {
                    Ok(()) => report,
                    Err(error) => {
                        if let Err(cleanup) = report {
                            tracing::error!(error = %cleanup, "kernel emergency cleanup failed");
                        }
                        Err(error)
                    }
                },
                Err(error) => match (s1_cleanup, report) {
                    (Ok(()), Ok(_)) => Err(error),
                    (s1_result, kernel_result) => {
                        if let Err(cleanup) = s1_result {
                            tracing::error!(
                                execution_error = %error,
                                cleanup_error = %cleanup,
                                "kernel operation and S1 worker cleanup failed"
                            );
                        }
                        if let Err(cleanup) = kernel_result {
                            tracing::error!(
                                execution_error = %error,
                                cleanup_error = %cleanup,
                                "kernel operation and emergency cleanup failed"
                            );
                        }
                        Err(KernelError::Runtime(
                            "kernel operation and cleanup both failed".to_owned(),
                        ))
                    }
                },
            }
        });
        *shared.task.lock().await = Some(task);
        Ok(KernelHandle { shared })
    }
}

struct WorkspaceLeases(polaris_runtime::leases::LeaseManager);
impl std::fmt::Debug for WorkspaceLeases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkspaceLeases(<kernel-owned>)")
    }
}

#[derive(Debug)]
struct Shared {
    ctx: Arc<KernelCtx>,
    backends: LocalBackends,
    workspace_leases: WorkspaceLeases,
    human_requests: Arc<crate::HumanRequestService>,
    frame_cancellations: Arc<crate::FrameCancellationService>,
    cancellation_verifier: Arc<crate::verifier::FrameCancellationVerifier>,
    human_verifier: Arc<crate::HumanRequestVerifier>,
    execution: Option<Arc<crate::SandboxExecution<polaris_openshell::client::OpenShellBackend>>>,
    s1_host: Option<Arc<crate::s1_host::LocalS1Host>>,
    frame: FrameId,
    session: SessionId,
    workspace: PathBuf,
    pending: Mutex<BTreeMap<CommandId, Reply>>,
    closed: AtomicBool,
    handles: AtomicUsize,
    stop: CancellationToken,
    finished: CancellationToken,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<ShutdownReport, KernelError>>>>,
    shutdown_result: Mutex<Option<Result<ShutdownReport, Arc<KernelError>>>>,
    inspections: Arc<tokio::sync::RwLock<()>>,
    inspection_admission: Arc<Semaphore>,
    frame_operation_admission: Arc<Semaphore>,
    frame_operations: Arc<tokio::sync::RwLock<()>>,
}

/// Bounded owner for one immutable tool inspection against this kernel's actual session stores.
/// Moving this value into a blocking closure keeps shutdown fencing and the global admission slot
/// alive until that blocking work has actually returned, even if its async caller is cancelled.
pub(crate) struct InspectionOwner {
    shared: Arc<Shared>,
    _inspection: OwnedRwLockReadGuard<()>,
    _admission: OwnedSemaphorePermit,
}

impl InspectionOwner {
    pub(crate) fn context(&self) -> &Arc<KernelCtx> {
        &self.shared.ctx
    }
    pub(crate) fn workspace_root(&self) -> &Path {
        &self.shared.workspace
    }

    pub(crate) fn session(&self) -> SessionId {
        self.shared.session
    }

    pub(crate) fn cas(&self) -> &crate::storage::cas::Cas {
        &self.shared.ctx.cas
    }

    pub(crate) fn ensure_open(&self) -> Result<(), polaris_sdk::registry::ToolError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "kernel closed".to_owned(),
            ));
        }
        self.shared
            .ctx
            .ensure_open()
            .map_err(|_| polaris_sdk::registry::ToolError::Io("kernel closed".to_owned()))
    }

    /// Invoke after the caller has verified an opaque frame binding against the active kernel.
    /// The supplied frame identifies that bound child; this owner supplies the actual workspace,
    /// session, deadline and tool backend and keeps shutdown/admission guards alive.
    pub(crate) fn invoke_authorized(
        &self,
        name: &str,
        args: &Value,
        frame: FrameId,
        cap: u64,
        policy: &impl ToolAccessPolicy,
    ) -> Result<polaris_tools::builtin::ToolOutput, polaris_sdk::registry::ToolError> {
        self.ensure_open()?;
        let context = polaris_sdk::registry::ToolContext {
            session: self.shared.session,
            frame,
            workspace_root: &self.shared.workspace,
            deadline: Some(Duration::from_secs(5)),
        };
        self.shared
            .backends
            .tools
            .invoke_authorized_bounded(name, args, &context, cap, policy)
    }

    fn invoke_trusted(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<polaris_tools::builtin::ToolOutput, polaris_sdk::registry::ToolError> {
        self.ensure_open()?;
        let context = polaris_sdk::registry::ToolContext {
            session: self.shared.session,
            frame: self.shared.frame,
            workspace_root: &self.shared.workspace,
            deadline: Some(Duration::from_secs(5)),
        };
        self.shared
            .backends
            .tools
            .invoke_blocking(name, arguments, &context)
    }
}

/// Independent bounded async owner: human waits do not occupy immutable inspection slots.
/// It holds shutdown fencing without counting as an external KernelHandle or keeping it alive.
pub(crate) struct FrameOperationOwner {
    shared: Arc<Shared>,
    _inspection: OwnedRwLockReadGuard<()>,
    _admission: OwnedSemaphorePermit,
}
impl FrameOperationOwner {
    pub(crate) fn workspace_root(&self) -> &Path {
        &self.shared.workspace
    }
    pub(crate) fn context(&self) -> &Arc<KernelCtx> {
        &self.shared.ctx
    }
    pub(crate) fn session(&self) -> SessionId {
        self.shared.session
    }
    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shared.stop.clone()
    }
    pub(crate) async fn prepare_frame_tests(
        &self,
        request: crate::ExecutionRequest,
    ) -> Result<crate::ExecutionReview, KernelError> {
        self.shared
            .execution
            .as_ref()
            .ok_or(KernelError::Configuration(
                "sandbox execution profile is not configured",
            ))?
            .prepare(
                Arc::clone(&self.shared.ctx),
                self.shared.workspace.clone(),
                request,
            )
            .await
    }
}

/// Cheap local API handle. Dropping the final handle requests orderly background shutdown.
#[derive(Debug)]
pub struct KernelHandle {
    shared: Arc<Shared>,
}
impl Clone for KernelHandle {
    fn clone(&self) -> Self {
        self.shared.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}
impl Drop for KernelHandle {
    fn drop(&mut self) {
        if self.shared.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.closed.store(true, Ordering::Release);
            self.shared.inspection_admission.close();
            self.shared.stop.cancel();
        }
    }
}
impl KernelHandle {
    /// Trusted host entry point for an actual registered subtree. No cancellation credential is exposed to tools.
    /// # Errors
    /// Unknown target, closed admission, authorization, durable cancellation or interruption failure.
    pub async fn cancel_frame(&self, target: FrameId) -> Result<CommandReceipt, ApiError> {
        let token = self
            .shared
            .cancellation_verifier
            .credential(target, Duration::from_secs(60))
            .map_err(|_| ApiError::Rejected("frame cancellation credential rejected".into()))?;
        self.submit(Command::new(
            CommandId::new_v7(),
            self.frame(),
            token,
            CommandBody::CancelFrame { frame: target },
        ))
        .await
    }
    pub(crate) fn frame_cancellations(&self) -> &Arc<crate::FrameCancellationService> {
        &self.shared.frame_cancellations
    }
    pub(crate) fn validate_frame_test_request(
        &self,
        frame: FrameId,
        request: &crate::ExecutionRequest,
    ) -> Result<(), KernelError> {
        if frame != self.shared.frame
            || self.shared.execution.is_none()
            || self.shared.closed.load(Ordering::Acquire)
        {
            return Err(KernelError::Configuration(
                "frame sandbox execution profile is not configured",
            ));
        }
        request.validate()
    }

    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shared.stop.clone()
    }
    pub(crate) fn workspace_leases(&self) -> polaris_runtime::leases::LeaseManager {
        self.shared.workspace_leases.0.clone()
    }
    pub(crate) fn session(&self) -> SessionId {
        self.shared.session
    }
    pub(crate) async fn frame_operation_owner(
        &self,
    ) -> Result<FrameOperationOwner, polaris_sdk::registry::ToolError> {
        let unavailable =
            || polaris_sdk::registry::ToolError::Forbidden("kernel closed".to_owned());
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        let admission = tokio::select! {
            biased;
            () = self.shared.stop.cancelled() => return Err(unavailable()),
            result = Arc::clone(&self.shared.frame_operation_admission).acquire_owned() => result.map_err(|_| unavailable())?,
        };
        let guard = Arc::clone(&self.shared.frame_operations).read_owned().await;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        Ok(FrameOperationOwner {
            shared: Arc::clone(&self.shared),
            _inspection: guard,
            _admission: admission,
        })
    }

    /// Inspect the one durable human-request owner used by this kernel's command dispatcher.
    /// This trusted host API exposes no execution or mutation approval authority.
    #[must_use]
    pub fn human_requests(&self) -> &Arc<crate::HumanRequestService> {
        &self.shared.human_requests
    }

    /// Commit a complete answer batch supplied by the trusted host through the signed command bus.
    /// This is a human input interface; do not register it as an agent tool or execution approval.
    /// # Errors
    /// Closed kernel, invalid/stale answer, queue admission, credential or durable commit failure.
    pub async fn answer_human_request(
        &self,
        request_id: polaris_sdk::human::HumanRequestId,
        revision: Cid,
        answers: Vec<polaris_sdk::human::HumanQuestionAnswer>,
    ) -> Result<CommandReceipt, ApiError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(ApiError::Closed);
        }
        let answer = self
            .shared
            .human_requests
            .stage_answer(request_id, revision, answers)
            .await
            .map_err(|_| ApiError::Rejected("human answer validation failed".to_owned()))?;
        let token = self
            .shared
            .human_verifier
            .credential(Duration::from_secs(60))
            .map_err(|_| ApiError::Rejected("human answer authority unavailable".to_owned()))?;
        self.submit(Command::new(
            CommandId::new_v7(),
            self.shared.frame,
            token,
            CommandBody::AnswerHumanRequest {
                request_id,
                revision,
                answer,
            },
        ))
        .await
    }

    /// Prepare an immutable P2 review from actual workspace, policy and image facts.
    /// # Errors
    /// Execution is unconfigured, kernel closed, or review preparation failed.
    pub async fn prepare_execution(
        &self,
        request: crate::ExecutionRequest,
    ) -> Result<crate::ExecutionReview, KernelError> {
        let _permit = self.shared.inspections.read().await;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(KernelError::Closed);
        }
        let execution = self
            .shared
            .execution
            .as_ref()
            .ok_or(KernelError::Configuration(
                "sandbox execution profile is not configured",
            ))?;
        execution
            .prepare(
                Arc::clone(&self.shared.ctx),
                self.shared.workspace.clone(),
                request,
            )
            .await
    }
    /// Submit an exact operator-approved review through the bounded signed command bus.
    /// This is a trusted approval-interface method; agent tool adapters must not expose it.
    /// # Errors
    /// Approval mismatch, closed/busy queue, stale or consumed review, execution/proof failure.
    pub async fn approve_execution(
        &self,
        review: Cid,
        accepted_digest: Cid,
    ) -> Result<CommandReceipt, ApiError> {
        let execution = self.shared.execution.as_ref().ok_or_else(|| {
            ApiError::Rejected("sandbox execution profile is not configured".into())
        })?;
        let token = execution
            .credential(review, accepted_digest)
            .map_err(api_error)?;
        self.submit(Command::new(
            CommandId::new_v7(),
            self.shared.frame,
            token,
            CommandBody::ExecuteTool { call: review },
        ))
        .await
    }

    /// Load the exact immutable execution review for the trusted non-model approval interface.
    /// Its digest, session, frame, workspace, image and execution policy are checked before return.
    /// Approval still performs a fresh source-tree check and consumes its one-use credential.
    /// # Errors
    /// Closed admission, missing backend, foreign/corrupt review or durable storage failure.
    pub async fn execution_review(
        &self,
        digest: Cid,
    ) -> Result<crate::ExecutionReview, KernelError> {
        let _permit = self.shared.inspections.read().await;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(KernelError::Closed);
        }
        let execution = self
            .shared
            .execution
            .as_ref()
            .ok_or(KernelError::Configuration(
                "sandbox execution profile is not configured",
            ))?;
        execution
            .read_review(
                Arc::clone(&self.shared.ctx),
                self.shared.workspace.clone(),
                digest,
            )
            .await
    }

    /// List bounded, durably published reviews not yet consumed by a sandbox execution.
    /// Returned digests are host review references, never automatic execution permission.
    /// # Errors
    /// Missing execution profile, closed admission, replay failure or bounded capacity exhaustion.
    pub async fn pending_execution_reviews(&self) -> Result<Vec<Cid>, KernelError> {
        let _permit = self.shared.inspections.read().await;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(KernelError::Closed);
        }
        let execution = self
            .shared
            .execution
            .as_ref()
            .ok_or(KernelError::Configuration(
                "sandbox execution profile is not configured",
            ))?;
        execution
            .pending_reviews(Arc::clone(&self.shared.ctx))
            .await
    }
    /// Durable local storage owner, for trusted embedding code only.
    #[must_use]
    pub fn context(&self) -> &Arc<KernelCtx> {
        &self.shared.ctx
    }
    /// Native tree owner for trusted composition code. Agent tools execute through sandbox APIs.
    #[must_use]
    pub fn platform(&self) -> &Arc<polaris_platform::process::ProcessPlatform> {
        &self.shared.backends.platform
    }
    /// Access the real supervised S1 host configured for this kernel, if present.
    #[must_use]
    pub fn s1_host(&self) -> Option<&Arc<crate::s1_host::LocalS1Host>> {
        self.shared.s1_host.as_ref()
    }

    /// Explicit trusted-host recovery for an abandoned System-One heavy-work pause.
    /// A live lease rejects recovery; this API must never be registered as a model tool.
    /// # Errors
    /// Missing host, active heavy operation, worker/control failure or closed kernel admission.
    pub async fn recover_s1_model_admission(&self) -> Result<(), KernelError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(KernelError::Closed);
        }
        let host = self
            .shared
            .s1_host
            .as_ref()
            .ok_or(KernelError::Configuration(
                "System One host is not configured",
            ))?;
        let key = polaris_runtime::leases::LeaseKey::from_resolved_relative("execution/workspace")
            .map_err(|_| KernelError::Integrity("execution lease key"))?;
        let _lease = tokio::time::timeout(
            Duration::from_secs(120),
            self.shared
                .workspace_leases
                .0
                .acquire(polaris_runtime::leases::LeaseRequest {
                    agent: self.frame(),
                    key,
                    scope: polaris_runtime::leases::LeaseScope::EntireFile,
                    base: None,
                }),
        )
        .await
        .map_err(|_| {
            KernelError::Runtime(
                "model admission recovery waited too long for sandbox execution".to_owned(),
            )
        })?
        .map_err(|_| {
            KernelError::Runtime(
                "model admission recovery could not own workspace execution".to_owned(),
            )
        })?;
        if self
            .shared
            .ctx
            .reaper
            .leak_report()
            .entries
            .iter()
            .any(|entry| entry.kind == crate::reaper::ResourceKind::Sandbox)
        {
            return Err(KernelError::Configuration(
                "sandbox teardown ownership remains; model admission cannot resume",
            ));
        }
        host.recover_model_admission().await
    }
    /// Inspect the initialized local tool catalog.
    #[must_use]
    pub fn tools(&self) -> &Arc<crate::LocalTools> {
        &self.shared.backends.tools
    }
    /// Admit a bounded immutable inspection tied to the active session and workspace.
    /// # Errors
    /// Closed kernel or closed inspection admission.
    pub(crate) async fn inspection_owner(
        &self,
    ) -> Result<InspectionOwner, polaris_sdk::registry::ToolError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "kernel closed".to_owned(),
            ));
        }
        let admission = Arc::clone(&self.shared.inspection_admission)
            .acquire_owned()
            .await
            .map_err(|_| polaris_sdk::registry::ToolError::Forbidden("kernel closed".to_owned()))?;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "kernel closed".to_owned(),
            ));
        }
        let inspection = Arc::clone(&self.shared.inspections).read_owned().await;
        let owner = InspectionOwner {
            shared: Arc::clone(&self.shared),
            _inspection: inspection,
            _admission: admission,
        };
        owner.ensure_open()?;
        Ok(owner)
    }
    /// Trusted local inspection entry point. Path, session and frame context cannot be overridden.
    /// # Errors
    /// Closed admission, invalid arguments, path denial, deadline, or real backend failure.
    pub async fn invoke_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<polaris_tools::builtin::ToolOutput, polaris_sdk::registry::ToolError> {
        let owner = self.inspection_owner().await?;
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || owner.invoke_trusted(&name, &arguments))
            .await
            .map_err(|_| polaris_sdk::registry::ToolError::Io("tool owner failed".to_owned()))?
    }
    /// Bound control frame identity.
    #[must_use]
    pub fn frame(&self) -> FrameId {
        self.shared.frame
    }
    /// Persist bounded UTF-8 control input without blocking a runtime worker.
    /// # Errors
    /// Closed admissions, size limit, or CAS failure.
    pub async fn store_text(&self, text: &str) -> Result<Cid, KernelError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(KernelError::Closed);
        }
        if text.len() > MAX_CONTROL_BYTES {
            return Err(KernelError::Configuration("local control exceeds 64 KiB"));
        }
        let text = text.to_owned();
        let ctx = Arc::clone(&self.shared.ctx);
        blocking(move || {
            ctx.ensure_open()?;
            Ok(ctx.cas.put(text.as_bytes())?)
        })
        .await
    }
    /// Submit one directive with a newly issued session-local signed credential.
    /// # Errors
    /// Invalid grammar, closed/full admissions, authorization, storage or projection failure.
    pub async fn apply_directive(&self, text: &str) -> Result<CommandReceipt, ApiError> {
        let cid = self.store_text(text).await.map_err(api_error)?;
        self.submit_control(CommandBody::ApplyDirective { directive: cid })
            .await
    }
    /// Record one typed user message verbatim (no directive parsing) with a newly issued signed
    /// session-local credential. The message is only recorded: no model execution is implied.
    /// # Errors
    /// Closed/full admissions, size limit, blank or non-UTF-8 text, authorization, storage or
    /// projection failure.
    pub async fn submit_user_input(&self, text: &str) -> Result<CommandReceipt, ApiError> {
        let cid = self.store_text(text).await.map_err(api_error)?;
        self.submit_control(CommandBody::SubmitUserInput { text: cid })
            .await
    }
    async fn submit_control(&self, body: CommandBody) -> Result<CommandReceipt, ApiError> {
        let control = Arc::clone(&self.shared.backends.control);
        let token =
            tokio::task::spawn_blocking(move || control.credential(Duration::from_secs(300)))
                .await
                .map_err(|_| ApiError::Internal("credential owner failed".to_owned()))?
                .map_err(|_| ApiError::Rejected("credential unavailable".to_owned()))?;
        self.submit(Command::new(CommandId::new_v7(), self.frame(), token, body))
            .await
    }
    /// Stop admission, drain accepted commands, pause the session and reap owned resources.
    /// # Errors
    /// Owner failure, durable pause or reaper failure. Repeated callers receive the same report.
    pub async fn shutdown(&self) -> Result<ShutdownReport, KernelError> {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.inspection_admission.close();
        self.shared.stop.cancel();
        let mut owner = self.shared.task.lock().await;
        let Some(task) = owner.as_mut() else {
            return self
                .shared
                .shutdown_result
                .lock()
                .map_err(|_| KernelError::Poisoned)?
                .clone()
                .ok_or(KernelError::Closed)?
                .map_err(KernelError::Shutdown);
        };
        let result = match task.await {
            Ok(result) => result,
            Err(_) => Err(KernelError::Runtime(
                "kernel shutdown owner failed".to_owned(),
            )),
        }
        .map_err(Arc::new);
        *self
            .shared
            .shutdown_result
            .lock()
            .map_err(|_| KernelError::Poisoned)? = Some(result.clone());
        let _finished = owner.take();
        result.map_err(KernelError::Shutdown)
    }
}

impl KernelApi for KernelHandle {
    type Events = KernelEvents;
    async fn submit(&self, command: Command<Unverified>) -> Result<CommandReceipt, ApiError> {
        let (tx, rx) = oneshot::channel();
        let id = command.id();
        {
            let mut pending = self
                .shared
                .pending
                .lock()
                .map_err(|_| ApiError::Internal("receipt owner poisoned".to_owned()))?;
            if self.shared.closed.load(Ordering::Acquire) {
                return Err(ApiError::Closed);
            }
            if pending.len() >= MAX_PENDING || pending.contains_key(&id) {
                return Err(ApiError::Busy);
            }
            pending.insert(id, tx);
            if let Err(error) = self.shared.ctx.bus.try_send(command) {
                pending.remove(&id);
                return Err(match error {
                    crate::bus::TrySendError::Full(_) => ApiError::Busy,
                    crate::bus::TrySendError::Closed(_) => ApiError::Closed,
                });
            }
        }
        rx.await.map_err(|_| ApiError::Closed)?
    }
    async fn subscribe(&self, from: Seq) -> Result<KernelEvents, ApiError> {
        // Subscribe first, then replay by cursor; overlap is deduplicated below.
        Ok(KernelEvents {
            ctx: Arc::clone(&self.shared.ctx),
            live: self.shared.ctx.subscribe(),
            cursor: from.get().max(1),
            replay: std::collections::VecDeque::new(),
            finished: self.shared.finished.clone(),
            exhausted: false,
        })
    }
}

/// Cursor-based replay followed by live events; lag causes bounded durable replay.
#[derive(Debug)]
pub struct KernelEvents {
    ctx: Arc<KernelCtx>,
    live: EventStream,
    cursor: u64,
    replay: std::collections::VecDeque<EventRecord>,
    finished: CancellationToken,
    exhausted: bool,
}
impl EventSource for KernelEvents {
    async fn next_event(&mut self) -> Option<EventRecord> {
        loop {
            if self.exhausted {
                return None;
            }
            if let Some(record) = self.replay.pop_front() {
                if record.seq().get() < self.cursor {
                    continue;
                }
                match record.seq().get().checked_add(1) {
                    Some(next) => self.cursor = next,
                    None => self.exhausted = true,
                }
                return Some(record);
            }
            let ctx = Arc::clone(&self.ctx);
            let from = Seq::new(self.cursor);
            let to = Seq::new(self.cursor.saturating_add(255));
            match blocking(move || ctx.records(from, to)).await {
                Ok(records) if !records.is_empty() => {
                    self.replay.extend(records);
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(error = %error, "event replay failed; subscription closed");
                    return None;
                }
            }
            if self.finished.is_cancelled() {
                return None;
            }
            tokio::select! {
                result = self.live.next() => match result {
                    Ok(record) => { self.replay.push_back(record); }
                    Err(BusError::Lagged { .. }) => continue,
                    Err(_) => return None,
                },
                () = self.finished.cancelled() => continue,
            }
        }
    }
}

async fn run(
    shared: Arc<Shared>,
    mut commands: crate::bus::CommandReceiver,
    mut session: Session<Running>,
) -> Result<(), KernelError> {
    let mut deferred = std::collections::VecDeque::new();
    loop {
        let next = if let Some(command) = deferred.pop_front() {
            Some(command)
        } else if shared.stop.is_cancelled() {
            commands.close();
            commands.recv().await
        } else {
            tokio::select! { command = commands.recv() => command, () = shared.stop.cancelled() => { commands.close(); commands.recv().await } }
        };
        let Some(command) = next else {
            break;
        };
        let id = command.id();
        if matches!(command.body(), CommandBody::CancelFrame { .. }) {
            let result = commit_frame_cancel(&shared, command).await;
            if let Ok(mut pending) = shared.pending.lock()
                && let Some(reply) = pending.remove(&id)
            {
                let _delivered = reply.send(result);
            }
            continue;
        }
        if matches!(command.body(), CommandBody::AnswerHumanRequest { .. }) {
            let verifier = Arc::clone(&shared.human_verifier);
            let evidence = KernelEvidence::new(Arc::clone(&shared.ctx));
            let verified =
                tokio::task::spawn_blocking(move || verifier.verify(command, &evidence)).await;
            let result = match verified {
                Ok(Ok(verified)) => match shared.human_requests.answer(verified).await {
                    Ok(crate::HumanRequestOutcome::Answered { seq, .. }) => Ok(CommandReceipt {
                        command: id,
                        accepted_at: seq,
                    }),
                    Ok(
                        crate::HumanRequestOutcome::Interrupted { .. }
                        | crate::HumanRequestOutcome::Pending,
                    ) => Err(ApiError::Rejected(
                        "human request was interrupted".to_owned(),
                    )),
                    Err(_) => Err(ApiError::Rejected(
                        "human answer commit rejected".to_owned(),
                    )),
                },
                Ok(Err(_)) => Err(ApiError::Rejected(
                    "human answer authorization failed".to_owned(),
                )),
                Err(_) => Err(ApiError::Rejected(
                    "human answer verifier owner failed".to_owned(),
                )),
            };
            if let Ok(mut pending) = shared.pending.lock()
                && let Some(reply) = pending.remove(&id)
            {
                let _delivered = reply.send(result);
            }
            continue;
        }
        if matches!(command.body(), CommandBody::ExecuteTool { .. }) {
            let result = match shared.execution.as_ref() {
                Some(execution) => {
                    let cancellation = match shared.frame_cancellations.token_for(shared.frame) {
                        Ok(token) => token,
                        Err(crate::FrameCancellationError::UnknownFrame) => {
                            shared.stop.child_token()
                        }
                        Err(_) => {
                            return Err(KernelError::Integrity(
                                "execution frame registry unavailable",
                            ));
                        }
                    };
                    let operation = execution.execute_with_model_pause(
                        Arc::clone(&shared.ctx),
                        shared.workspace.clone(),
                        command,
                        cancellation,
                        shared.s1_host.clone(),
                    );
                    tokio::pin!(operation);
                    let mut commands_open = true;
                    loop {
                        tokio::select! {
                            result=&mut operation=>break result.map_err(api_error),
                            next=commands.recv(),if commands_open=>match next {
                                Some(control) if matches!(control.body(),CommandBody::CancelFrame {..})=> {
                                    let control_id=control.id();
                                    let result=commit_frame_cancel(&shared,control).await;
                                    if let Ok(mut pending)=shared.pending.lock() && let Some(reply)=pending.remove(&control_id) {let _delivered=reply.send(result);}
                                }
                                Some(other)=>deferred.push_back(other),
                                None=>commands_open=false,
                            },
                        }
                    }
                }
                None => Err(ApiError::Rejected(
                    "sandbox execution profile is not configured".into(),
                )),
            };
            if let Ok(mut pending) = shared.pending.lock()
                && let Some(reply) = pending.remove(&id)
            {
                let _delivered = reply.send(result);
            }
            continue;
        }
        let owner = Arc::clone(&shared);
        let outcome = tokio::task::spawn_blocking(move || {
            let result = execute(&owner, &mut session, command);
            (session, result)
        })
        .await;
        match outcome {
            Ok((returned, result)) => {
                session = returned;
                if let Ok(mut pending) = shared.pending.lock()
                    && let Some(reply) = pending.remove(&id)
                {
                    let _delivered = reply.send(result);
                }
            }
            Err(_) => {
                shared.closed.store(true, Ordering::Release);
                return Err(KernelError::Runtime(
                    "kernel command operation panicked or stopped".to_owned(),
                ));
            }
        }
    }
    let paused = session.pause().await;
    {
        let mut pending = shared.pending.lock().map_err(|_| KernelError::Poisoned)?;
        for (_, reply) in std::mem::take(&mut *pending) {
            let _delivered = reply.send(Err(ApiError::Closed));
        }
    }
    let _paused = paused?;
    Ok(())
}

fn frame_cancelled_or_unregistered(
    service: &crate::FrameCancellationService,
    frame: FrameId,
) -> Result<bool, KernelError> {
    match service.is_cancelled(frame) {
        Ok(cancelled) => Ok(cancelled),
        Err(crate::FrameCancellationError::UnknownFrame) => Ok(false),
        Err(_) => Err(KernelError::Integrity(
            "frame cancellation registry unavailable",
        )),
    }
}

async fn commit_frame_cancel(
    shared: &Shared,
    command: Command<Unverified>,
) -> Result<CommandReceipt, ApiError> {
    let id = command.id();
    let verifier = Arc::clone(&shared.cancellation_verifier);
    let evidence = KernelEvidence::new(Arc::clone(&shared.ctx));
    let verified = tokio::task::spawn_blocking(move || verifier.verify(command, &evidence)).await;
    match verified {
        Ok(Ok(verified)) => match shared.frame_cancellations.apply_verified(verified).await {
            Ok(outcome) => {
                async {
                    let requests =
                        shared.human_requests.list().await.map_err(|_| {
                            ApiError::Internal("human request snapshot failed".into())
                        })?;
                    for request in requests {
                        if request.outcome == crate::HumanRequestOutcome::Pending
                            && frame_cancelled_or_unregistered(
                                &shared.frame_cancellations,
                                request.ticket.frame,
                            )
                            .map_err(api_error)?
                        {
                            let _interrupted = shared
                                .human_requests
                                .interrupt_frame(request.ticket.frame)
                                .await
                                .map_err(|_| {
                                    ApiError::Internal(
                                        "pending question interruption failed".into(),
                                    )
                                })?;
                        }
                    }
                    Ok(CommandReceipt {
                        command: id,
                        accepted_at: outcome.seq,
                    })
                }
                .await
            }
            Err(_) => Err(ApiError::Rejected(
                "frame cancellation commit rejected".into(),
            )),
        },
        Ok(Err(_)) => Err(ApiError::Rejected(
            "frame cancellation authorization rejected".into(),
        )),
        Err(_) => Err(ApiError::Internal(
            "frame cancellation verifier failed".into(),
        )),
    }
}

fn execute(
    shared: &Shared,
    session: &mut Session<Running>,
    command: Command<Unverified>,
) -> Result<CommandReceipt, ApiError> {
    let command = shared
        .backends
        .control
        .verify(command, &KernelEvidence::new(Arc::clone(&shared.ctx)))
        .map_err(|_| ApiError::Rejected("local control authorization failed".to_owned()))?;
    match *command.body() {
        CommandBody::ApplyDirective { directive } => {
            execute_directive(shared, session, command.id(), directive)
        }
        CommandBody::SubmitUserInput { text } => {
            execute_user_input(shared, session, command.id(), text)
        }
        CommandBody::ProposeDiff { .. }
        | CommandBody::AcquireAstScope { .. }
        | CommandBody::ReleaseAstScope { .. }
        | CommandBody::SpawnSubagent { .. }
        | CommandBody::ExecuteTool { .. }
        | CommandBody::RollbackTo { .. }
        | CommandBody::CancelFrame { .. }
        | CommandBody::AnswerHumanRequest { .. } => Err(ApiError::Rejected(
            "command is outside P1 local control profile".to_owned(),
        )),
    }
}
fn execute_user_input(
    shared: &Shared,
    session: &mut Session<Running>,
    command: CommandId,
    text: Cid,
) -> Result<CommandReceipt, ApiError> {
    let bytes = shared
        .ctx
        .cas
        .get(&text)
        .map_err(|error| api_error(error.into()))?;
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(ApiError::Rejected("user input exceeds limit".to_owned()));
    }
    let message = std::str::from_utf8(&bytes)
        .map_err(|_| ApiError::Rejected("user input is not UTF-8".to_owned()))?;
    if message.trim().is_empty() {
        return Err(ApiError::Rejected("user input is empty".to_owned()));
    }
    let seq = session.record_user_input(text).map_err(api_error)?;
    Ok(CommandReceipt {
        command,
        accepted_at: seq,
    })
}
fn execute_directive(
    shared: &Shared,
    session: &mut Session<Running>,
    command: CommandId,
    directive: Cid,
) -> Result<CommandReceipt, ApiError> {
    let bytes = shared
        .ctx
        .cas
        .get(&directive)
        .map_err(|error| api_error(error.into()))?;
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(ApiError::Rejected("directive exceeds limit".to_owned()));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ApiError::Rejected("directive is not UTF-8".to_owned()))?;
    let parsed = crate::directives::parse(text, &crate::directives::ParseLimits::default())
        .map_err(|_| ApiError::Rejected("invalid directive grammar".to_owned()))?;
    if parsed.directives.len() != 1
        || parsed.command.is_some()
        || !parsed.mentions.is_empty()
        || !parsed.prose.trim().is_empty()
    {
        return Err(ApiError::Rejected(
            "expected exactly one directive".to_owned(),
        ));
    }
    let directive = parsed
        .directives
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::Rejected("missing directive".to_owned()))?;
    let seq = session.apply(directive.value).map_err(api_error)?;
    Ok(CommandReceipt {
        command,
        accepted_at: seq,
    })
}
fn api_error(error: KernelError) -> ApiError {
    match error {
        KernelError::Closed => ApiError::Closed,
        KernelError::Directive(_) | KernelError::Configuration(_) => {
            ApiError::Rejected(error.to_string())
        }
        other => ApiError::Internal(other.to_string()),
    }
}

// Finish every receipt even if the owner exits through a panic or early error.
struct OwnerCompletion(Arc<Shared>);
impl Drop for OwnerCompletion {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        self.0.finished.cancel();
        let mut pending = match self.0.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (_, reply) in std::mem::take(&mut *pending) {
            let _delivered = reply.send(Err(ApiError::Closed));
        }
    }
}

#[cfg(test)]
mod inspection_owner_tests {
    use super::{Kernel, LocalKernelConfig};
    use polaris_sdk::ids::{FrameId, SessionId};
    use std::{io, sync::mpsc, time::Duration};

    #[tokio::test]
    async fn inspection_slots_bound_admission_and_shutdown_releases_waiters()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            SessionId::new_v7(),
            FrameId::new_v7(),
        ))
        .await?;
        let mut owners = Vec::new();
        for _ in 0..super::MAX_INSPECTION_OWNERS {
            owners.push(kernel.inspection_owner().await?);
        }
        assert_eq!(kernel.shared.inspection_admission.available_permits(), 0);
        let admission_kernel = kernel.clone();
        let mut admission = tokio::spawn(async move { admission_kernel.inspection_owner().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut admission)
                .await
                .is_err(),
            "a sixty-fifth owner must wait"
        );
        let shutdown_kernel = kernel.clone();
        let shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
        let refused = tokio::time::timeout(Duration::from_secs(5), &mut admission).await??;
        assert!(refused.is_err(), "shutdown closes blocked admission");
        drop(owners);
        let _report = tokio::time::timeout(Duration::from_secs(5), shutdown).await???;
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_drains_a_blocking_inspection_owner() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            SessionId::new_v7(),
            FrameId::new_v7(),
        ))
        .await?;
        let owner = kernel
            .inspection_owner()
            .await
            .map_err(|error| io::Error::other(format!("inspection admission failed: {error}")))?;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let inspection = tokio::task::spawn_blocking(move || {
            started_tx
                .send(())
                .map_err(|error| io::Error::other(error.to_string()))?;
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| io::Error::other(error.to_string()))?;
            drop(owner);
            Ok::<(), io::Error>(())
        });
        started_rx.recv_timeout(Duration::from_secs(5))?;

        let shutdown_kernel = kernel.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
        let completed_early = tokio::time::timeout(Duration::from_millis(25), &mut shutdown)
            .await
            .is_ok();
        release_tx
            .send(())
            .map_err(|error| io::Error::other(error.to_string()))?;
        inspection.await??;
        if completed_early {
            return Err(io::Error::other("shutdown did not wait for inspection owner").into());
        }
        let _report = shutdown.await??;
        Ok(())
    }
}

#[cfg(test)]
mod user_input_cas_tests {
    use super::{Kernel, KernelHandle, LocalKernelConfig, MAX_CONTROL_BYTES};
    use polaris_sdk::{
        contracts::{Command, CommandBody, Event},
        ids::{Cid, CommandId, FrameId, Seq, SessionId},
        registry::{KernelApi, KernelError as ApiError},
    };
    use std::time::Duration;

    async fn submit_raw_cas(
        handle: &KernelHandle,
        bytes: &[u8],
    ) -> Result<Result<polaris_sdk::registry::CommandReceipt, ApiError>, Box<dyn std::error::Error>>
    {
        let cid: Cid = handle.shared.ctx.cas.put(bytes)?;
        let token = handle
            .shared
            .backends
            .control
            .credential(Duration::from_secs(30))?;
        Ok(handle
            .submit(Command::new(
                CommandId::new_v7(),
                handle.frame(),
                token,
                CommandBody::SubmitUserInput { text: cid },
            ))
            .await)
    }

    async fn started()
    -> Result<(KernelHandle, tempfile::TempDir, tempfile::TempDir), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let handle = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            SessionId::new_v7(),
            FrameId::new_v7(),
        ))
        .await?;
        Ok((handle, root, workspace))
    }

    fn user_input_count(handle: &KernelHandle) -> Result<usize, Box<dyn std::error::Error>> {
        Ok(handle
            .context()
            .records(Seq::ZERO, Seq::new(u64::MAX))?
            .iter()
            .filter(|record| matches!(record.event(), Event::UserInputSubmitted { .. }))
            .count())
    }

    #[tokio::test]
    async fn non_utf8_cas_user_input_is_rejected_by_execute()
    -> Result<(), Box<dyn std::error::Error>> {
        let (handle, _root, _workspace) = started().await?;
        let outcome = submit_raw_cas(&handle, &[0xff, 0xfe, 0xfd]).await?;
        assert!(
            matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input is not UTF-8"),
            "unexpected outcome {outcome:?}"
        );
        assert_eq!(user_input_count(&handle)?, 0, "nothing was appended");
        let _report = handle.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn oversize_cas_user_input_is_rejected_by_execute()
    -> Result<(), Box<dyn std::error::Error>> {
        let (handle, _root, _workspace) = started().await?;
        let outcome = submit_raw_cas(&handle, &vec![b'a'; MAX_CONTROL_BYTES + 1]).await?;
        assert!(
            matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input exceeds limit"),
            "unexpected outcome {outcome:?}"
        );
        assert_eq!(user_input_count(&handle)?, 0, "nothing was appended");
        let _report = handle.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn blank_cas_user_input_is_rejected_by_execute() -> Result<(), Box<dyn std::error::Error>>
    {
        let (handle, _root, _workspace) = started().await?;
        let outcome = submit_raw_cas(&handle, b" \n\t").await?;
        assert!(
            matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input is empty"),
            "unexpected outcome {outcome:?}"
        );
        let _report = handle.shutdown().await?;
        Ok(())
    }
}
