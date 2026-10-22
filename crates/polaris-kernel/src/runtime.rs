//! P1 local composition: bounded command dispatch, signed controls and durable replay.
use crate::{
    Created, KernelCtx, KernelError, KernelEvidence, Running, Session, SessionConfig,
    bus::{BusError, EventStream},
    context::blocking,
    reaper::ShutdownReport,
    verifier::LocalDirectiveVerifier,
};
use polaris_sdk::{
    contracts::{Command, CommandBody, EventRecord, Unverified},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi, KernelError as ApiError},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const MAX_PENDING: usize = 1024;
const MAX_CONTROL_BYTES: usize = 65_536;
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
        }
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
        Self::start_inner(config, None).await
    }
    /// Start the P2 execution profile with an explicit live OpenShell backend and image policy.
    /// # Errors
    /// Invalid image/policy, authority, store, recovery or session startup failure.
    pub async fn start_sandbox(
        config: LocalKernelConfig,
        backend: Arc<polaris_openshell::client::OpenShellBackend>,
        spec: polaris_sdk::registry::SandboxSpec,
    ) -> Result<KernelHandle, KernelError> {
        let execution = Arc::new(crate::SandboxExecution::new(
            backend,
            spec,
            config.session,
            config.frame,
        )?);
        Self::start_inner(config, Some(execution)).await
    }
    async fn start_inner(
        config: LocalKernelConfig,
        execution: Option<
            Arc<crate::SandboxExecution<polaris_openshell::client::OpenShellBackend>>,
        >,
    ) -> Result<KernelHandle, KernelError> {
        let control = Arc::new(
            LocalDirectiveVerifier::new(config.session, config.frame)
                .map_err(|_| KernelError::Configuration("local authority unavailable"))?,
        );
        let platform = Arc::new(polaris_platform::process::ProcessPlatform::new());
        let ctx = KernelCtx::open(&config.root).await?;
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
        let commands = ctx.take_commands()?;
        let shared = Arc::new(Shared {
            ctx,
            backends,
            execution,
            frame: config.frame,
            session: config.session,
            workspace,
            pending: Mutex::new(BTreeMap::new()),
            closed: AtomicBool::new(false),
            handles: AtomicUsize::new(1),
            stop: CancellationToken::new(),
            finished: CancellationToken::new(),
            task: tokio::sync::Mutex::new(None),
            shutdown_result: Mutex::new(None),
            inspections: Arc::new(tokio::sync::RwLock::new(())),
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
            let _drained = owner.inspections.write().await;
            // This coordinator survives a dispatcher panic/early return and owns async cleanup.
            let report = owner.ctx.shutdown().await;
            match result {
                Ok(()) => report,
                Err(error) => {
                    if let Err(cleanup) = report {
                        tracing::error!(error = %cleanup, "kernel emergency cleanup failed");
                    }
                    Err(error)
                }
            }
        });
        *shared.task.lock().await = Some(task);
        Ok(KernelHandle { shared })
    }
}

#[derive(Debug)]
struct Shared {
    ctx: Arc<KernelCtx>,
    backends: LocalBackends,
    execution: Option<Arc<crate::SandboxExecution<polaris_openshell::client::OpenShellBackend>>>,
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
            self.shared.stop.cancel();
        }
    }
}
impl KernelHandle {
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
    /// Inspect the initialized local tool catalog.
    #[must_use]
    pub fn tools(&self) -> &Arc<crate::LocalTools> {
        &self.shared.backends.tools
    }
    /// Trusted local inspection entry point. Path, session and frame context cannot be overridden.
    /// # Errors
    /// Closed admission, invalid arguments, path denial, deadline, or real backend failure.
    pub async fn invoke_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<polaris_tools::builtin::ToolOutput, polaris_sdk::registry::ToolError> {
        let permit = Arc::clone(&self.shared.inspections).read_owned().await;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "kernel closed".to_owned(),
            ));
        }
        let shared = Arc::clone(&self.shared);
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let context = polaris_sdk::registry::ToolContext {
                session: shared.session,
                frame: shared.frame,
                workspace_root: &shared.workspace,
                deadline: Some(Duration::from_secs(5)),
            };
            shared
                .backends
                .tools
                .invoke_blocking(&name, &arguments, &context)
        })
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
        let control = Arc::clone(&self.shared.backends.control);
        let token =
            tokio::task::spawn_blocking(move || control.credential(Duration::from_secs(300)))
                .await
                .map_err(|_| ApiError::Internal("credential owner failed".to_owned()))?
                .map_err(|_| ApiError::Rejected("credential unavailable".to_owned()))?;
        self.submit(Command::new(
            CommandId::new_v7(),
            self.frame(),
            token,
            CommandBody::ApplyDirective { directive: cid },
        ))
        .await
    }
    /// Stop admission, drain accepted commands, pause the session and reap owned resources.
    /// # Errors
    /// Owner failure, durable pause or reaper failure. Repeated callers receive the same report.
    pub async fn shutdown(&self) -> Result<ShutdownReport, KernelError> {
        self.shared.closed.store(true, Ordering::Release);
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
    loop {
        let next = if shared.stop.is_cancelled() {
            commands.close();
            commands.recv().await
        } else {
            tokio::select! { command = commands.recv() => command, () = shared.stop.cancelled() => { commands.close(); commands.recv().await } }
        };
        let Some(command) = next else {
            break;
        };
        let id = command.id();
        if matches!(command.body(), CommandBody::ExecuteTool { .. }) {
            let result = match shared.execution.as_ref() {
                Some(execution) => execution
                    .execute(
                        Arc::clone(&shared.ctx),
                        shared.workspace.clone(),
                        command,
                        shared.stop.clone(),
                    )
                    .await
                    .map_err(api_error),
                None => Err(ApiError::Rejected(
                    "sandbox execution profile is not configured".into(),
                )),
            };
            if let Ok(mut pending) = shared.pending.lock() {
                if let Some(reply) = pending.remove(&id) {
                    let _delivered = reply.send(result);
                }
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
                if let Ok(mut pending) = shared.pending.lock() {
                    if let Some(reply) = pending.remove(&id) {
                        let _delivered = reply.send(result);
                    }
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
    let CommandBody::ApplyDirective { directive } = *command.body() else {
        return Err(ApiError::Rejected(
            "command is outside P1 local control profile".to_owned(),
        ));
    };
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
        command: command.id(),
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
