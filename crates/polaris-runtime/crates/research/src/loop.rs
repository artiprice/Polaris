//! Bounded orchestration across authorized frame tools, an SDK sandbox, VCS, and durable CAS.
use crate::store::RunOwnerLease;
use crate::{
    Budget, CheckpointStore, Decision, ExperimentId, LedgerKind, MetricValue, Observation,
    ProgramMd, ResearchError, ResearchState, StoreError, Usage, WorkspaceError,
};
use polaris_runtime_frames::{ToolCall, ToolRunner};
use polaris_sdk::{
    error::HasKind,
    ids::{Cid, FrameId},
    registry::{ExecRequest, SandboxBackend, SandboxHandle},
};
use polaris_security_capability::{
    ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock, Token, attenuate,
};
use polaris_storage_cas::content_id;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const MAX_METRIC_OUTPUT_BYTES: usize = 65_536;
const MAX_TOOL_RESULT_BYTES: usize = 65_536;

/// One proposed tool call bound to an exact repository-relative mutation path.
#[derive(Clone, Debug)]
pub struct ScopedToolCall {
    /// Proposed call; it remains untrusted until its narrowed token is authorized.
    pub call: ToolCall,
    /// Exact path captured by VCS and used to attenuate the capability for this call.
    pub path: Box<str>,
}

/// Experiment proposal. Every write call names the exact path captured and authorized by the loop.
#[derive(Clone, Debug)]
pub struct ResearchEdit {
    /// One concise, human-reviewable experiment hypothesis.
    pub hypothesis: Box<str>,
    /// Tool calls with exact paths; the loop snapshots these and narrows each capability to them.
    pub calls: Vec<ScopedToolCall>,
    /// Signed frame token and identity used as the parent for per-path capability attenuation.
    pub tool_context: Option<ToolContext>,
    /// Actual model usage reported by the trusted frame telemetry.
    pub model_usage: ModelUsage,
}

/// Capability context for authorizing proposed frame tools.
#[derive(Clone)]
pub struct ToolContext {
    /// Signed and verified frame token.
    pub token: Token,
    /// Frame identity that owns the calls.
    pub frame: FrameId,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("token", &self.token)
            .field("frame", &self.frame)
            .finish()
    }
}

/// Model resources reported by the trusted frame telemetry for one proposal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelUsage {
    /// Prompt and completion tokens attributed to this proposal.
    pub tokens: u64,
    /// Actual cost attributed by the configured provider ledger, in micro-USD.
    pub micro_usd: u64,
}

/// Outcome of one completed or safely recovered experiment.
#[derive(Clone, Debug, PartialEq)]
pub struct StepReport {
    /// Run-local experiment ordinal.
    pub id: ExperimentId,
    /// Proposed human-readable hypothesis.
    pub hypothesis: Box<str>,
    /// Measured metric, absent when the harness failed or produced invalid output.
    pub metric: Option<MetricValue>,
    /// Keep/revert outcome when the workspace operation was acknowledged.
    pub decision: Decision,
    /// Content ID of raw metric harness output, when it was captured.
    pub evidence: Option<Cid>,
    /// Durable content IDs returned by authorized tool executions in this iteration.
    pub tool_evidence: Vec<Cid>,
    /// Content ID of the acknowledged VCS mutation or recovery receipt.
    pub mutation_receipt: Cid,
    /// Honest terminal status for this iteration.
    pub status: IterationStatus,
}

/// Terminal status for one experiment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IterationStatus {
    /// A valid measurement was settled with its requested keep/revert action.
    Measured,
    /// Execution failed or produced invalid output; baseline recovery was acknowledged.
    Recovered(FailureReason),
}

/// Why the iteration was recovered without a successful measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureReason {
    /// An authorized tool failed.
    Tool,
    /// Sandbox transport failed.
    Sandbox,
    /// Metric process returned a nonzero exit code.
    ExitStatus,
    /// Metric output was invalid or ambiguous.
    Metric,
    /// Actual output or usage exceeded a bound.
    Limit,
    /// The VCS mutation did not settle as requested, then baseline recovery succeeded.
    Workspace,
}

/// Live projection for research status.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchStatus {
    /// Number of settled or recovered iterations.
    pub completed: u32,
    /// Current best acknowledged metric.
    pub best: Option<MetricValue>,
    /// Consecutive non-improving iterations.
    pub plateau: u32,
    /// Accounted resource use, including in-flight reservation.
    pub spent: Usage,
    /// True after a resource overrun.
    pub halted: bool,
    /// In-flight iteration requiring workspace recovery, if any.
    pub recovery_required: Option<ExperimentId>,
}

/// Bounded loop stop rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopCondition {
    /// Stop after at most this many additional plans.
    pub max_iterations: u32,
    /// Stop after this many non-improving iterations; `None` disables plateau stopping.
    pub plateau_patience: Option<u32>,
}

/// Summary of the plans consumed by run_until.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSummary {
    /// Number of plans that completed or recovered.
    pub iterations: u32,
    /// Why the run stopped.
    pub stop: RunStop,
}

/// Bounded terminal condition for a research run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStop {
    /// The configured maximum was reached.
    MaxIterations,
    /// The provided plan stream ended.
    PlansEnded,
    /// No more bounded run resources were available.
    BudgetExhausted,
    /// The plateau patience bound was reached.
    Plateau,
}

/// One research run wired directly to authorized tools, an SDK sandbox, VCS, and durable storage.
pub struct ResearchLoop<W, B, T> {
    program: ProgramMd,
    state: ResearchState,
    workspace: W,
    sandbox: B,
    sandbox_handle: SandboxHandle,
    tools: T,
    store: CheckpointStore,
    _run_owner: RunOwnerLease,
}

impl<W, B, T> ResearchLoop<W, B, T>
where
    W: crate::ExperimentWorkspace,
    B: SandboxBackend,
    T: ToolRunner,
    T::Error: HasKind + Send,
{
    /// Create a fresh run and durably persist its initial state.
    ///
    /// # Errors
    /// Returns an error when another loop owns the run, a checkpoint already exists, or initial
    /// persistence fails.
    pub fn new(
        program: ProgramMd,
        workspace: W,
        sandbox: B,
        sandbox_handle: SandboxHandle,
        tools: T,
        store: CheckpointStore,
    ) -> Result<Self, LoopError> {
        let run_owner = store.acquire_run_owner()?;
        let state = ResearchState::new(
            program.cid(),
            program.metric().direction(),
            program.budget(),
        )?;
        match store.initialize(&state) {
            Ok(_) => {}
            Err(StoreError::CheckpointExists) => return Err(LoopError::ExistingCheckpoint),
            Err(error) => return Err(LoopError::Store(error)),
        }
        Ok(Self {
            program,
            state,
            workspace,
            sandbox,
            sandbox_handle,
            tools,
            store,
            _run_owner: run_owner,
        })
    }

    /// Reopen a durable run. Any unresolved workspace state is recovered before new work.
    ///
    /// # Errors
    /// Returns an error when no checkpoint exists or safe baseline restoration cannot be proven.
    pub fn resume(
        program: ProgramMd,
        workspace: W,
        sandbox: B,
        sandbox_handle: SandboxHandle,
        tools: T,
        store: CheckpointStore,
    ) -> Result<Self, LoopError> {
        let run_owner = store.acquire_run_owner()?;
        let state = store
            .load()?
            .ok_or(LoopError::MissingCheckpoint)?
            .into_state();
        if state.program() != program.cid() {
            return Err(LoopError::Research(ResearchError::ProgramChanged));
        }
        Ok(Self {
            program,
            state,
            workspace,
            sandbox,
            sandbox_handle,
            tools,
            store,
            _run_owner: run_owner,
        })
    }

    /// Resolve persisted in-flight workspace state before continuing after a crash.
    ///
    /// # Errors
    /// Returns an error when no recovery is pending or VCS cannot prove restoration.
    pub fn recover_pending(&mut self) -> Result<Cid, LoopError> {
        let id = self
            .state
            .recovery_required()
            .ok_or(LoopError::NoRecovery)?;
        let receipt = self.workspace.recover(id)?;
        let receipt_cid = self.persist_workspace_receipt(id, LedgerKind::Recovery, &receipt)?;
        self.state.recovered(id, receipt_cid)?;
        self.store.save(&self.state)?;
        Ok(receipt_cid)
    }

    /// Run one proposed experiment with actual SDK tool authorization and sandbox execution.
    ///
    /// The workspace paths are snapshotted before tool dispatch. The metric command is read from
    /// the immutable program, the backend enforces timeout/output capture limits, and VCS must
    /// return a separate mutation receipt before the state can settle.
    ///
    /// # Errors
    /// Returns an error if persistence or workspace safety is uncertain. Failed tools, transport,
    /// exit status, and metric parsing produce a `Recovered` report after actual VCS restoration.
    pub async fn step(&mut self, edit: ResearchEdit) -> Result<StepReport, LoopError> {
        if edit.hypothesis.trim().is_empty() || edit.hypothesis.len() > 4_096 {
            return Err(LoopError::Edit);
        }
        if edit.calls.is_empty()
            || edit
                .calls
                .iter()
                .any(|call| call.path.trim().is_empty() || !self.program.allows_edit(&call.path))
        {
            return Err(LoopError::Edit);
        }
        if edit.tool_context.is_none() {
            return Err(LoopError::Edit);
        }
        let reservation = per_experiment(self.program.budget())?;
        let started = Instant::now();
        let id = self.state.begin(reservation)?;
        self.store.save(&self.state)?;
        let paths = edit
            .calls
            .iter()
            .map(|call| call.path.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        self.workspace.begin(id, &paths)?;
        let tool_evidence = match self.execute_tools(id, &edit).await {
            Ok(evidence) => evidence,
            Err(LoopError::Store(error)) => {
                self.recover_pending_state(id)?;
                return Err(LoopError::Store(error));
            }
            Err(_) => return self.recover_failed(id, edit.hypothesis, FailureReason::Tool),
        };
        let request = metric_request(&self.program, reservation);
        let output = match self.sandbox.exec(&self.sandbox_handle, &request).await {
            Ok(output) => output,
            Err(_) => {
                return self.recover_failed_with_tools(
                    id,
                    edit.hypothesis,
                    tool_evidence,
                    FailureReason::Sandbox,
                );
            }
        };
        let combined = match output.stdout.len().checked_add(output.stderr.len()) {
            Some(size) if size <= MAX_METRIC_OUTPUT_BYTES => size,
            _ => {
                return self.recover_failed_with_tools(
                    id,
                    edit.hypothesis,
                    tool_evidence,
                    FailureReason::Limit,
                );
            }
        };
        let wall_ms = match elapsed_ms(started) {
            Ok(value) => value,
            Err(_) => {
                return self.recover_failed_with_tools(
                    id,
                    edit.hypothesis,
                    tool_evidence,
                    FailureReason::Limit,
                );
            }
        };
        let measured_usage =
            Usage::new(wall_ms, edit.model_usage.tokens, edit.model_usage.micro_usd);
        let raw = match raw_run_bytes(
            output.exit_code,
            output.duration,
            &output.stdout,
            &output.stderr,
            combined,
        ) {
            Ok(raw) => raw,
            Err(_) => {
                return self.recover_failed_with_tools(
                    id,
                    edit.hypothesis,
                    tool_evidence,
                    FailureReason::Limit,
                );
            }
        };
        let evidence = match self.store.put_evidence(&raw) {
            Ok(cid) => cid,
            Err(error) => {
                self.recover_pending_state(id)?;
                return Err(LoopError::Store(error));
            }
        };
        if let Err(error) = self
            .store
            .append_ledger(id.get(), LedgerKind::MetricRun, evidence)
        {
            self.recover_pending_state(id)?;
            return Err(LoopError::Store(error));
        }
        if output.exit_code != 0 {
            return self.recover_failed_with_evidence(
                id,
                edit.hypothesis,
                tool_evidence,
                Some(evidence),
                FailureReason::ExitStatus,
            );
        }
        let metric = match self.program.metric().parse_output(&output.stdout) {
            Ok(metric) => metric,
            Err(_) => {
                return self.recover_failed_with_evidence(
                    id,
                    edit.hypothesis,
                    tool_evidence,
                    Some(evidence),
                    FailureReason::Metric,
                );
            }
        };
        let observation = Observation::new(metric, evidence, measured_usage);
        let decision = match self.state.measure(id, observation) {
            Ok(decision) => decision,
            Err(ResearchError::Overrun) => {
                self.store.save(&self.state)?;
                let receipt = self.workspace.recover(id)?;
                let receipt_cid =
                    self.persist_workspace_receipt(id, LedgerKind::Recovery, &receipt)?;
                self.state.recovered(id, receipt_cid)?;
                self.store.save(&self.state)?;
                return Ok(StepReport {
                    id,
                    hypothesis: edit.hypothesis,
                    metric: None,
                    decision: Decision::Revert,
                    evidence: Some(evidence),
                    tool_evidence,
                    mutation_receipt: receipt_cid,
                    status: IterationStatus::Recovered(FailureReason::Limit),
                });
            }
            Err(error) => return Err(LoopError::Research(error)),
        };
        self.store.save(&self.state)?;
        let receipt = match self.workspace.settle(id, decision) {
            Ok(receipt) => receipt,
            Err(_) => {
                let receipt = self.recover_pending_state(id)?;
                return Ok(StepReport {
                    id,
                    hypothesis: edit.hypothesis,
                    metric: None,
                    decision: Decision::Revert,
                    evidence: Some(evidence),
                    tool_evidence,
                    mutation_receipt: receipt,
                    status: IterationStatus::Recovered(FailureReason::Workspace),
                });
            }
        };
        let receipt_cid = match self.persist_workspace_receipt(id, LedgerKind::Mutation, &receipt) {
            Ok(cid) => cid,
            Err(error) => {
                self.recover_pending_state(id)?;
                return Err(error);
            }
        };
        self.state.settle(id, decision, receipt_cid)?;
        self.store.save(&self.state)?;
        Ok(StepReport {
            id,
            hypothesis: edit.hypothesis,
            metric: Some(metric),
            decision,
            evidence: Some(evidence),
            tool_evidence,
            mutation_receipt: receipt_cid,
            status: IterationStatus::Measured,
        })
    }

    /// Consume proposed edits until a bound, plateau, resource limit, or plan exhaustion.
    ///
    /// # Errors
    /// Returns errors for uncertain workspace or persistence state.
    pub async fn run_until<I>(
        &mut self,
        plans: I,
        stop: StopCondition,
    ) -> Result<RunSummary, LoopError>
    where
        I: IntoIterator<Item = ResearchEdit>,
    {
        if stop.max_iterations == 0 || stop.plateau_patience == Some(0) {
            return Err(LoopError::Edit);
        }
        let mut iterations = 0_u32;
        for edit in plans {
            if iterations >= stop.max_iterations {
                return Ok(RunSummary {
                    iterations,
                    stop: RunStop::MaxIterations,
                });
            }
            if stop
                .plateau_patience
                .is_some_and(|patience| self.state.plateau() >= patience)
            {
                return Ok(RunSummary {
                    iterations,
                    stop: RunStop::Plateau,
                });
            }
            match self.step(edit).await {
                Ok(_) => {
                    iterations = iterations.checked_add(1).ok_or(LoopError::Limit)?;
                }
                Err(LoopError::Research(ResearchError::Budget)) => {
                    return Ok(RunSummary {
                        iterations,
                        stop: RunStop::BudgetExhausted,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(RunSummary {
            iterations,
            stop: RunStop::PlansEnded,
        })
    }

    /// Current bounded progress projection.
    #[must_use]
    pub const fn status(&self) -> ResearchStatus {
        ResearchStatus {
            completed: self.state.completed(),
            best: self.state.best(),
            plateau: self.state.plateau(),
            spent: self.state.spent(),
            halted: self.state.halted(),
            recovery_required: self.state.recovery_required(),
        }
    }

    /// Markdown report with state and content IDs that resolve to durable raw measurement blobs.
    #[must_use]
    pub fn report(&self) -> String {
        let mut report = String::from("# Research report\n\n## Goal\n\n");
        report.push_str(self.program.goal());
        report.push_str("\n\n## State and evidence ledger\n\n");
        report.push_str(&self.state.report());
        report.push_str("\nEvidence links (newest first):\n");
        match self.store.ledger() {
            Ok(entries) => {
                for entry in entries {
                    report.push_str(&format!(
                        "experiment={} kind={:?} evidence={}\n",
                        entry.experiment, entry.kind, entry.evidence
                    ));
                }
            }
            Err(_) => report.push_str("ledger_unavailable=true\n"),
        }
        report
    }

    async fn execute_tools(
        &self,
        id: ExperimentId,
        edit: &ResearchEdit,
    ) -> Result<Vec<Cid>, LoopError> {
        let mut evidence = Vec::with_capacity(edit.calls.len());
        for scoped in &edit.calls {
            let context = edit.tool_context.as_ref().ok_or(LoopError::Edit)?;
            let path = ResourcePath::new(&scoped.path).map_err(|_| LoopError::Edit)?;
            let token = attenuate(
                &context.token,
                &Scope {
                    resource: ResourceScope::Exact(path),
                    max_churn_lines: u32::MAX,
                    ttl: Duration::from_secs(3_600),
                    allow_unsafe: false,
                    diff_blake3: None,
                    rights: Rights::new(&[Right::ExecuteTool]),
                },
                &SystemClock,
            )
            .map_err(|_| LoopError::Tool)?;
            let permit = self
                .tools
                .authorize(&token, scoped.call.clone())
                .await
                .map_err(|_| LoopError::Tool)?;
            let result = self
                .tools
                .execute(permit, context.frame, MAX_TOOL_RESULT_BYTES)
                .await
                .map_err(|_| LoopError::Tool)?;
            if result.content.len() > MAX_TOOL_RESULT_BYTES {
                return Err(LoopError::Limit);
            }
            if content_id(result.content.as_bytes()) != result.content_cid {
                return Err(LoopError::Store(StoreError::Evidence));
            }
            let tool_blob =
                tool_evidence_bytes(result.cid, result.content_cid, result.content.as_bytes())?;
            let tool_cid = self.store.put_evidence(&tool_blob)?;
            self.store
                .append_ledger(id.get(), LedgerKind::Tool, tool_cid)?;
            evidence.push(tool_cid);
        }
        Ok(evidence)
    }

    fn recover_pending_state(&mut self, id: ExperimentId) -> Result<Cid, LoopError> {
        if self.state.recovery_required() != Some(id) {
            self.state.interrupt(id)?;
        }
        let receipt = self.workspace.recover(id)?;
        let receipt_cid = self.persist_workspace_receipt(id, LedgerKind::Recovery, &receipt)?;
        self.state.recovered(id, receipt_cid)?;
        self.store.save(&self.state)?;
        Ok(receipt_cid)
    }

    fn persist_workspace_receipt(
        &self,
        id: ExperimentId,
        kind: LedgerKind,
        receipt: &crate::WorkspaceReceipt,
    ) -> Result<Cid, LoopError> {
        if content_id(&receipt.payload) != receipt.cid {
            return Err(LoopError::Store(StoreError::Evidence));
        }
        let stored = self.store.put_evidence(&receipt.payload)?;
        if stored != receipt.cid {
            return Err(LoopError::Store(StoreError::Evidence));
        }
        self.store.append_ledger(id.get(), kind, stored)?;
        Ok(stored)
    }

    fn recover_failed(
        &mut self,
        id: ExperimentId,
        hypothesis: Box<str>,
        reason: FailureReason,
    ) -> Result<StepReport, LoopError> {
        self.recover_failed_with_tools(id, hypothesis, Vec::new(), reason)
    }

    fn recover_failed_with_tools(
        &mut self,
        id: ExperimentId,
        hypothesis: Box<str>,
        tool_evidence: Vec<Cid>,
        reason: FailureReason,
    ) -> Result<StepReport, LoopError> {
        self.recover_failed_with_evidence(id, hypothesis, tool_evidence, None, reason)
    }

    fn recover_failed_with_evidence(
        &mut self,
        id: ExperimentId,
        hypothesis: Box<str>,
        tool_evidence: Vec<Cid>,
        evidence: Option<Cid>,
        reason: FailureReason,
    ) -> Result<StepReport, LoopError> {
        let receipt = self.recover_pending_state(id)?;
        Ok(StepReport {
            id,
            hypothesis,
            metric: None,
            decision: Decision::Revert,
            evidence,
            tool_evidence,
            mutation_receipt: receipt,
            status: IterationStatus::Recovered(reason),
        })
    }
}

fn per_experiment(budget: Budget) -> Result<Usage, LoopError> {
    let count = u64::from(budget.experiments());
    let all = budget.resources();
    let reservation = Usage::new(
        all.wall_ms().div_ceil(count),
        all.tokens().div_ceil(count),
        all.micro_usd().div_ceil(count),
    );
    if reservation.wall_ms() == 0 {
        return Err(LoopError::Limit);
    }
    Ok(reservation)
}

fn metric_request(program: &ProgramMd, reservation: Usage) -> ExecRequest {
    ExecRequest {
        argv: program
            .metric()
            .command()
            .iter()
            .map(ToString::to_string)
            .collect(),
        workdir: None,
        stdin: None,
        env: Vec::new(),
        timeout: Some(Duration::from_millis(reservation.wall_ms())),
        output_limit_bytes: Some(MAX_METRIC_OUTPUT_BYTES),
    }
}

fn elapsed_ms(started: Instant) -> Result<u64, LoopError> {
    u64::try_from(started.elapsed().as_millis()).map_err(|_| LoopError::Limit)
}

fn raw_run_bytes(
    exit_code: i32,
    duration: Duration,
    stdout: &[u8],
    stderr: &[u8],
    combined_size: usize,
) -> Result<Vec<u8>, LoopError> {
    if combined_size > MAX_METRIC_OUTPUT_BYTES {
        return Err(LoopError::Limit);
    }
    let stdout_len = u64::try_from(stdout.len()).map_err(|_| LoopError::Limit)?;
    let stderr_len = u64::try_from(stderr.len()).map_err(|_| LoopError::Limit)?;
    let duration_ms = u64::try_from(duration.as_millis()).map_err(|_| LoopError::Limit)?;
    let mut bytes = Vec::with_capacity(combined_size.saturating_add(32));
    bytes.extend_from_slice(b"POLARIS-RESEARCH-RUN\x01");
    bytes.extend_from_slice(&exit_code.to_le_bytes());
    bytes.extend_from_slice(&duration_ms.to_le_bytes());
    bytes.extend_from_slice(&stdout_len.to_le_bytes());
    bytes.extend_from_slice(stdout);
    bytes.extend_from_slice(&stderr_len.to_le_bytes());
    bytes.extend_from_slice(stderr);
    Ok(bytes)
}

fn tool_evidence_bytes(result: Cid, content: Cid, bytes: &[u8]) -> Result<Vec<u8>, LoopError> {
    let content_len = u64::try_from(bytes.len()).map_err(|_| LoopError::Limit)?;
    let mut evidence = Vec::with_capacity(bytes.len().saturating_add(86));
    evidence.extend_from_slice(b"POLARIS-RESEARCH-TOOL\x01");
    evidence.extend_from_slice(result.as_bytes());
    evidence.extend_from_slice(content.as_bytes());
    evidence.extend_from_slice(&content_len.to_le_bytes());
    evidence.extend_from_slice(bytes);
    Ok(evidence)
}

/// Orchestration, persistence, authorization, or measurement failure.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// State transition failed.
    #[error("research transition failed: {0}")]
    Research(#[from] ResearchError),
    /// Durable store failed.
    #[error("research store failed: {0}")]
    Store(#[from] StoreError),
    /// Workspace could not safely acknowledge or restore a mutation.
    #[error("research workspace failed: {0}")]
    Workspace(#[from] WorkspaceError),
    /// A run must be resumed from a stored checkpoint.
    #[error("research checkpoint does not exist")]
    MissingCheckpoint,
    /// A checkpoint already exists and must be continued with `ResearchLoop::resume`.
    #[error("research checkpoint already exists; use resume")]
    ExistingCheckpoint,
    /// There is no in-flight experiment to recover.
    #[error("no research recovery is pending")]
    NoRecovery,
    /// Hypothesis, path, tool context, or stop bounds are invalid.
    #[error("invalid research edit or stop condition")]
    Edit,
    /// Tool authorization or execution failed.
    #[error("research tool call failed")]
    Tool,
    /// A bounded representation was exceeded.
    #[error("research iteration exceeded a fixed limit")]
    Limit,
}
