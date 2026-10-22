//! Immutable human reviews, signed dispatch and owned sandbox test execution.
use crate::{
    KernelCtx, KernelError, KernelEvidence,
    context::blocking,
    reaper::{Disposition, Locator, ResourceKind, Teardown, TeardownContext, TeardownError},
    storage::sync::{self, Manifest},
    verifier::LocalExecutionVerifier,
};
use polaris_runtime::{
    guards::{
        self, BlastInput, BlastLimits, FileFact, GlobPattern, ShellKind, WorkspacePath,
        WorkspaceView,
    },
    leases::{LeaseConfig, LeaseKey, LeaseManager, LeaseRequest, LeaseScope},
};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event, Timestamp, TokenBytes, Unverified},
    ids::{Cid, CommandId, FrameId, SessionId},
    registry::{
        CommandReceipt, ExecOutput, ExecRequest, SandboxBackend, SandboxHandle, SandboxSpec,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const REMOTE_ROOT: &str = "/sandbox/ws";
const MAX_REVIEW_BYTES: usize = 1024 * 1024;
const EXECUTION_ENV: &[(&str, &str)] = &[
    ("CARGO_HOME", "/sandbox/cargo"),
    ("CARGO_TARGET_DIR", "/sandbox/build"),
    ("CARGO_NET_OFFLINE", "true"),
    ("RUSTUP_AUTO_INSTALL", "0"),
    ("RUSTUP_TOOLCHAIN", "1.99.0"),
    ("PYTHONDONTWRITEBYTECODE", "1"),
];

/// Explicit test and independent falsifier commands. Programs execute as argv, without an
/// implicit shell. Environment and remote working directories are fixed by this profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    /// Ordinary test runner and arguments.
    pub argv: Vec<String>,
    /// Separate falsifier/property target, executed after the ordinary suite.
    pub independent_argv: Vec<String>,
    /// Maximum seconds for each command, between 1 and 900.
    pub timeout_secs: u64,
    /// Minimum ordinary passing tests required to prove completion.
    pub min_tests: u32,
}
impl ExecutionRequest {
    fn validate(&self) -> Result<(), KernelError> {
        for argv in [&self.argv, &self.independent_argv] {
            if argv.is_empty()
                || argv.len() > 256
                || argv
                    .iter()
                    .any(|arg| arg.len() > 8192 || arg.contains('\0'))
                || argv.iter().map(String::len).sum::<usize>() > 65_536
            {
                return Err(KernelError::Configuration("execution argv exceeds limits"));
            }
        }
        if self.argv == self.independent_argv
            || !(1..=900).contains(&self.timeout_secs)
            || self.min_tests == 0
        {
            return Err(KernelError::Configuration(
                "distinct independent runner and positive bounded limits required",
            ));
        }
        Ok(())
    }
}

/// Review displayed by the trusted approval interface. Approval must echo `digest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReview {
    /// Exact immutable CAS artifact accepted by the operator.
    pub digest: Cid,
    /// Source tree bound to the artifact.
    pub tree: Cid,
    /// Full review: commands, policy, image and conservative blast accounting.
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReview {
    version: u32,
    session: SessionId,
    frame: FrameId,
    nonce: CommandId,
    workspace: PathBuf,
    tree: Cid,
    sandbox: SandboxSpec,
    request: ExecutionRequest,
    blast: serde_json::Value,
}

/// Generic execution owner. The production root injects real OpenShell; contract tests inject
/// explicitly identified test backends. No model or ambient credential is consulted.
pub struct SandboxExecution<B> {
    backend: Arc<B>,
    spec: SandboxSpec,
    session: SessionId,
    frame: FrameId,
    authority: Arc<LocalExecutionVerifier>,
    leases: LeaseManager,
}
impl<B> std::fmt::Debug for SandboxExecution<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxExecution")
            .field("session", &self.session)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}
impl<B: SandboxBackend + 'static> SandboxExecution<B> {
    /// Bind a backend, explicit policy and immutable image reference to one session/frame.
    /// # Errors
    /// Missing policy/image, mutable image reference, or authority construction failure.
    pub fn new(
        backend: Arc<B>,
        spec: SandboxSpec,
        session: SessionId,
        frame: FrameId,
    ) -> Result<Self, KernelError> {
        let Some(image) = spec.image.as_deref() else {
            return Err(KernelError::Configuration(
                "explicit immutable sandbox image required",
            ));
        };
        let digest = match image
            .strip_prefix("sha256:")
            .or_else(|| image.split_once("@sha256:").map(|(_, digest)| digest))
        {
            Some(digest) => digest,
            None => {
                return Err(KernelError::Configuration(
                    "sandbox image must use a content digest",
                ));
            }
        };
        if digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || spec.policy.is_none()
        {
            return Err(KernelError::Configuration(
                "valid image digest and explicit policy required",
            ));
        }
        Ok(Self {
            backend,
            spec,
            session,
            frame,
            authority: Arc::new(
                LocalExecutionVerifier::new(session, frame)
                    .map_err(|_| KernelError::Configuration("execution authority unavailable"))?,
            ),
            leases: LeaseManager::new(LeaseConfig::default()),
        })
    }

    /// Snapshot workspace facts and persist the review before any sandbox side effect.
    /// # Errors
    /// Invalid inputs, ambiguous workspace, scan, blast analysis or CAS failure.
    pub async fn prepare(
        &self,
        ctx: Arc<KernelCtx>,
        workspace: PathBuf,
        request: ExecutionRequest,
    ) -> Result<ExecutionReview, KernelError> {
        request.validate()?;
        let spec = self.spec.clone();
        let session = self.session;
        let frame = self.frame;
        blocking(move || {
            ctx.ensure_open()?;
            let workspace = std::fs::canonicalize(workspace)?;
            let snapshot = snapshot(&workspace)?;
            let view = SnapshotView::new(&workspace, &snapshot)?;
            let cwd = WorkspacePath::root();
            let limits = BlastLimits::new(10, 100, 1024 * 1024, 16 * 1024 * 1024, true).map_err(|_| KernelError::Configuration("blast limits invalid"))?;
            let mut reports = Vec::new();
            for argv in [&request.argv, &request.independent_argv] {
                let command = argv.join(" ");
                let report = guards::blast_radius(&view, &BlastInput {command:&command,cwd:&cwd,shell:ShellKind::Posix,limits:&limits,cargo_target_dir:None}).map_err(|_|KernelError::Configuration("blast facts unavailable"))?;
                reports.push(serde_json::json!({"severity":format!("{:?}",report.severity),"confidence":format!("{:?}",report.confidence),"deleted":report.deleted.iter().map(WorkspacePath::as_str).collect::<Vec<_>>(),"overwritten":report.overwritten.iter().map(WorkspacePath::as_str).collect::<Vec<_>>(),"uncommitted_hit":report.uncommitted_hit.iter().map(WorkspacePath::as_str).collect::<Vec<_>>(),"total_bytes":report.total_bytes,"reasons":report.reasons}));
            }
            let tree = snapshot.content_id();
            let environment: std::collections::BTreeMap<_, _> = EXECUTION_ENV.iter().copied().collect();
            let stored = StoredReview {version:1,session,frame,nonce:CommandId::new_v7(),workspace,tree,sandbox:spec,request,blast:serde_json::json!({"commands":reports,"approval":"explicit_digest_required","source":"paranoid_scan_and_git_status","execution_environment":environment,"remote_workdir":REMOTE_ROOT})};
            let bytes = serde_json::to_vec(&stored)?;
            if bytes.len() > MAX_REVIEW_BYTES { return Err(KernelError::Configuration("review exceeds size bound")); }
            let digest = ctx.cas.put(&bytes)?;
            Ok(ExecutionReview {digest,tree,details:serde_json::to_value(stored)?})
        }).await
    }

    /// Trusted approval interface entry point. Never expose this issuer to agent tools.
    /// # Errors
    /// Accepted digest differs from the review or capability issuance failed.
    pub fn credential(&self, review: Cid, accepted_digest: Cid) -> Result<TokenBytes, KernelError> {
        self.authority
            .credential(review, accepted_digest, Duration::from_secs(60))
            .map_err(|_| KernelError::Configuration("execution approval rejected"))
    }

    /// Dispatch signed execution. The caller retains this future in an owned worker until it
    /// completes; cancellation is injected through `stop`, never by dropping the owner future.
    /// # Errors
    /// Invalid/replayed/stale review, backend failure, proof rejection or cleanup failure.
    pub async fn execute(
        &self,
        ctx: Arc<KernelCtx>,
        workspace: PathBuf,
        command: Command<Unverified>,
        stop: CancellationToken,
    ) -> Result<CommandReceipt, KernelError> {
        let authority = Arc::clone(&self.authority);
        let context = Arc::clone(&ctx);
        let verified = blocking(move || {
            authority
                .verify(command, &KernelEvidence::new(context))
                .map_err(|_| KernelError::Configuration("signed execution rejected"))
        })
        .await?;
        let CommandBody::ExecuteTool { call } = *verified.body() else {
            return Err(KernelError::Integrity("execution command kind changed"));
        };
        let owner = verified.frame();
        let command_id = verified.id();
        let context = Arc::clone(&ctx);
        let expected_spec = self.spec.clone();
        let session = self.session;
        let (review, manifest, accepted_at) = blocking(move || {
            let bytes = context.cas.get(&call)?;
            if bytes.len() > MAX_REVIEW_BYTES { return Err(KernelError::Configuration("review exceeds limit")); }
            let review: StoredReview = serde_json::from_slice(&bytes)?;
            review.request.validate()?;
            let workspace = std::fs::canonicalize(workspace)?;
            if review.version != 1 || review.session != session || review.frame != owner || review.workspace != workspace || review.sandbox != expected_spec { return Err(KernelError::Configuration("review context mismatch")); }
            let manifest = snapshot(&workspace)?;
            if manifest.content_id() != review.tree { return Err(KernelError::Configuration("reviewed source tree changed")); }
            let _writer = context.lock_writer()?;
            for record in context.bus.store().replay_iter(polaris_sdk::ids::Seq::new(u64::MAX))? {
                let record = record?;
                if record.session() == session && matches!(record.event(), Event::ToolExecutionStarted {call: used,..} if *used == call) { return Err(KernelError::Configuration("execution approval was already consumed")); }
            }
            let accepted = context.publish(session,Event::ToolExecutionStarted {frame:owner,call})?.seq();
            Ok((review,manifest,accepted))
        }).await?;
        let scope_key = LeaseKey::from_resolved_relative("execution/workspace")
            .map_err(|_| KernelError::Integrity("execution lease key"))?;
        let _lease = self
            .leases
            .acquire(LeaseRequest {
                agent: self.frame,
                key: scope_key,
                scope: LeaseScope::EntireFile,
                base: Some(review.tree),
            })
            .await
            .map_err(|_| KernelError::Configuration("execution lease unavailable"))?;
        // OpenShell v0.1.4 permits 19 characters. The UUID's final 16 hex digits retain
        // 62 random bits, avoiding the timestamp-heavy prefix of UUIDv7.
        // Generate the identity after approval. A public review must not let another
        // caller reserve the journaled name before this owner reaches create.
        let suffix: String = CommandId::new_v7()
            .as_uuid()
            .simple()
            .to_string()
            .chars()
            .rev()
            .take(16)
            .collect();
        let sandbox = SandboxHandle::new(&format!("p2-{suffix}"))
            .map_err(|_| KernelError::Integrity("sandbox name invalid"))?;
        let context = Arc::clone(&ctx);
        let backend = Arc::clone(&self.backend);
        let registered = sandbox.clone();
        let registration = blocking(move || {
            Ok(context.reaper.register_journaled(
                ResourceKind::Sandbox,
                Locator::Label(registered.as_str().to_owned()),
                OwnedSandbox {
                    backend,
                    handle: registered,
                },
            )?)
        })
        .await?;
        let mut spec = self.spec.clone();
        spec.name = Some(sandbox.as_str().to_owned());
        let started = Instant::now();
        let operation = async {
            if stop.is_cancelled() {
                return Err(KernelError::Closed);
            }
            let created = self.backend.create(&spec).await.map_err(backend_error)?;
            if created != sandbox {
                // Preserve the actual returned identity even if immediate destruction fails.
                let context = Arc::clone(&ctx);
                let backend = Arc::clone(&self.backend);
                let actual = created.clone();
                let unexpected_registration = blocking(move || {
                    Ok(context.reaper.register_journaled(
                        ResourceKind::Sandbox,
                        Locator::Label(actual.as_str().to_owned()),
                        OwnedSandbox {
                            backend,
                            handle: actual,
                        },
                    )?)
                })
                .await?;
                self.backend
                    .destroy(&created)
                    .await
                    .map_err(backend_error)?;
                let context = Arc::clone(&ctx);
                blocking(move || Ok(context.reaper.release(unexpected_registration)?)).await?;
                return Err(KernelError::Integrity(
                    "backend changed journaled sandbox identity",
                ));
            }
            self.run_in_sandbox(
                Arc::clone(&ctx),
                &sandbox,
                &review,
                &manifest,
                command_id,
                &stop,
            )
            .await
        }
        .await;
        let failure_report = if let Err(error) = &operation {
            let context = Arc::clone(&ctx);
            let reason = error.to_string();
            let frame = self.frame;
            let sandbox_name = sandbox.as_str().to_owned();
            blocking(move || {
                let report=context.cas.put(&serde_json::to_vec(&serde_json::json!({"stage":"sandbox_execution","review":call,"sandbox":sandbox_name,"command":command_id,"reason":reason}))?)?;
                context.publish(session,Event::ProofOfDoneBlocked {frame,report})?;Ok(())
            }).await
        } else {
            Ok(())
        };
        // A journaled intent precedes create, so cancellation during readiness cannot lose the
        // cleanup identity. Destruction failure retains recovery metadata for reaper retry.
        let cleanup = if matches!(
            &operation,
            Err(KernelError::Sandbox(
                polaris_sdk::registry::SandboxError::AlreadyExists(_)
            ))
        ) {
            // The backend explicitly refused ownership. Removing this intent must never
            // issue a delete against the resource belonging to another creator.
            Ok(())
        } else {
            tokio::time::timeout(Duration::from_secs(30), self.backend.destroy(&sandbox))
                .await
                .map_err(|_| KernelError::Runtime("sandbox cleanup timed out".into()))
                .and_then(|result| result.map_err(backend_error))
        };
        if cleanup.is_ok() {
            let context = Arc::clone(&ctx);
            blocking(move || Ok(context.reaper.release(registration)?)).await?;
        }
        let context = Arc::clone(&ctx);
        let ok = operation.is_ok() && cleanup.is_ok();
        let frame = self.frame;
        let latency_us = u64::try_from(started.elapsed().as_micros())
            .map_err(|_| KernelError::Integrity("execution duration overflow"))?;
        blocking(move || {
            context.publish(
                session,
                Event::ToolExecutionCompleted {
                    frame,
                    call,
                    ok,
                    latency_us,
                },
            )?;
            Ok(())
        })
        .await?;
        cleanup?;
        failure_report?;
        operation?;
        Ok(CommandReceipt {
            command: command_id,
            accepted_at,
        })
    }

    async fn run_in_sandbox(
        &self,
        ctx: Arc<KernelCtx>,
        sandbox: &SandboxHandle,
        review: &StoredReview,
        manifest: &Manifest,
        command: CommandId,
        stop: &CancellationToken,
    ) -> Result<(), KernelError> {
        let init = ExecRequest {
            argv: vec![
                "mkdir".into(),
                "-p".into(),
                REMOTE_ROOT.into(),
                "/sandbox/build".into(),
                "/sandbox/cargo".into(),
            ],
            timeout: Some(Duration::from_secs(10)),
            ..ExecRequest::default()
        };
        if self
            .backend
            .exec(sandbox, &init)
            .await
            .map_err(backend_error)?
            .exit_code
            != 0
        {
            return Err(KernelError::Configuration(
                "sandbox workspace initialization failed",
            ));
        }
        let handshake = ExecRequest {
            argv: vec!["/usr/bin/polaris-guest".into(), "--protocol-version".into()],
            timeout: Some(Duration::from_secs(10)),
            ..ExecRequest::default()
        };
        let guest = self
            .backend
            .exec(sandbox, &handshake)
            .await
            .map_err(backend_error)?;
        if guest.exit_code != 0 || guest.stdout.as_ref() != b"1\n" {
            return Err(KernelError::Configuration("guest protocol mismatch"));
        }
        let plan = sync::plan(manifest, &Manifest::default());
        let report = sync::apply(
            self.backend.as_ref(),
            sandbox,
            &review.workspace,
            manifest,
            &plan,
            &sync::ApplyOpts {
                remote_root: REMOTE_ROOT.into(),
                concurrency: 8,
                cancellation: None,
            },
        )
        .await
        .map_err(|_| KernelError::Runtime("workspace sync failed".into()))?;
        if !report.failed.is_empty()
            || !report.changed.is_empty()
            || !report.conflicts.is_empty()
            || !report.path_conflicts.is_empty()
            || !report.cancelled.is_empty()
        {
            return Err(KernelError::Configuration(
                "workspace sync did not preserve reviewed snapshot",
            ));
        }
        self.verify_remote(sandbox, manifest).await?;
        let output = self
            .exec(
                sandbox,
                &review.request.argv,
                review.request.timeout_secs,
                stop,
            )
            .await?;
        let context = Arc::clone(&ctx);
        let captured = output.clone();
        let session = self.session;
        let frame = self.frame;
        let sandbox_name = sandbox.as_str().to_owned();
        let (stdout,stderr,exec_record) = blocking(move || {
            let stdout=context.cas.put(&captured.stdout)?;let stderr=context.cas.put(&captured.stderr)?;
            let latency_us=u64::try_from(captured.duration.as_micros()).map_err(|_|KernelError::Integrity("execution duration overflow"))?;
            let record=context.cas.put(&serde_json::to_vec(&serde_json::json!({"sandbox":sandbox_name,"command":command,"stdout":stdout,"stderr":stderr,"exit_code":captured.exit_code,"duration_us":latency_us}))?)?;
            context.publish(session,Event::SandboxExecutionCompleted {frame,exec:record,exit_code:captured.exit_code,latency_us})?;
            Ok((stdout,stderr,record))
        }).await?;
        self.verify_remote(sandbox, manifest).await?;
        let independent = self
            .exec(
                sandbox,
                &review.request.independent_argv,
                review.request.timeout_secs,
                stop,
            )
            .await?;
        self.verify_remote(sandbox, manifest).await?;
        let independent_summary = guards::TestSummary::from_output(
            &review.request.independent_argv.join(" "),
            &independent,
        );
        let independent_ok = independent.exit_code == 0
            && independent_summary.parse_status() == guards::ParseStatus::Parsed
            && independent_summary.passed() > 0
            && independent_summary.failed() == 0;
        let context = Arc::clone(&ctx);
        let captured = independent.clone();
        let independent_report=blocking(move || {
            let out=context.cas.put(&captured.stdout)?;let err=context.cas.put(&captured.stderr)?;
            Ok(context.cas.put(&serde_json::to_vec(&serde_json::json!({"kind":"independent_falsifier_run","stdout":out,"stderr":err,"exit_code":captured.exit_code,"ordinary_execution":exec_record}))?)?)
        }).await?;
        let current = {
            let root = review.workspace.clone();
            blocking(move || snapshot(&root)).await?
        };
        let current_tree = current.content_id();
        let context = Arc::clone(&ctx);
        let output_for_rtk = output.clone();
        let issues = blocking(move || {
            let length = output_for_rtk
                .stdout
                .len()
                .checked_add(output_for_rtk.stderr.len())
                .ok_or(KernelError::Integrity("output size overflow"))?;
            if length > 16 * 1024 * 1024 {
                return Err(KernelError::Configuration("execution output exceeds limit"));
            }
            let mut raw = Vec::with_capacity(length.saturating_add(1));
            raw.extend_from_slice(&output_for_rtk.stdout);
            raw.push(b'\n');
            raw.extend_from_slice(&output_for_rtk.stderr);
            let path = context
                .root()
                .join("execution-ledgers")
                .join(format!("{command}.md"));
            let ledger = polaris_tools::rtk::LedgerFile::new(path);
            polaris_tools::rtk::run(
                polaris_tools::rtk::RtkMode::Issues,
                &raw,
                &ledger,
                Timestamp::now(),
                &context.cas,
            )
            .map_err(|_| KernelError::Runtime("execution RTK reconciliation failed".into()))?
            .summary
            .ok_or(KernelError::Integrity("RTK issues result missing"))
        })
        .await?;
        // Remote source equals the reviewed manifest before and after both runners. This P2
        // immutable-test profile refuses every source mutation, including assertion tampering.
        let independent_proof = guards::IndependentProof::falsifier(
            current_tree,
            stdout,
            independent_report,
            independent_summary.passed(),
            if independent_ok { 0 } else { 1 },
        );
        let run = guards::RunEvidence::from_sandbox(
            review.request.argv.join(" "),
            &output,
            review.tree,
            stdout,
            stderr,
            &issues,
            guards::TamperReport::default(),
            independent_proof,
            None,
        )
        .map_err(|_| KernelError::Integrity("run artifact binding failed"))?;
        let policy = guards::ProofPolicy::new(review.request.min_tests, false)
            .map_err(|_| KernelError::Configuration("proof policy invalid"))?;
        let verdict = guards::proof_of_done(
            guards::Completion::pending(command, review.tree),
            &run,
            current_tree,
            &policy,
        );
        let context = Arc::clone(&ctx);
        let passed = verdict.is_ok();
        let reason = verdict.as_ref().err().map(ToString::to_string);
        let session = self.session;
        let frame = self.frame;
        blocking(move || {
            let report=context.cas.put(&serde_json::to_vec(&serde_json::json!({"command":command,"tree":current_tree,"stdout":stdout,"stderr":stderr,"independent_report":independent_report,"passed":passed,"reason":reason}))?)?;
            context.publish(session,if passed {Event::ProofOfDonePassed{frame,report}}else{Event::ProofOfDoneBlocked{frame,report}})?;
            Ok(())
        }).await?;
        verdict.map(|_| ()).map_err(|_| {
            KernelError::Configuration("Proof-of-Done blocked; inspect durable report and stderr")
        })
    }

    async fn exec(
        &self,
        sandbox: &SandboxHandle,
        argv: &[String],
        timeout_secs: u64,
        stop: &CancellationToken,
    ) -> Result<ExecOutput, KernelError> {
        let request = ExecRequest {
            argv: argv.to_vec(),
            workdir: Some(REMOTE_ROOT.into()),
            env: EXECUTION_ENV
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            timeout: Some(Duration::from_secs(timeout_secs)),
            ..ExecRequest::default()
        };
        tokio::select! {
            result=self.backend.exec(sandbox,&request)=>result.map_err(backend_error),
            ()=stop.cancelled()=>Err(KernelError::Closed),
        }
    }
    async fn verify_remote(
        &self,
        sandbox: &SandboxHandle,
        expected: &Manifest,
    ) -> Result<(), KernelError> {
        let remote = sync::SandboxSyncBackend::new(self.backend.as_ref(), sandbox, REMOTE_ROOT, 8)
            .map_err(|_| KernelError::Configuration("remote snapshot configuration invalid"))?
            .snapshot()
            .await
            .map_err(|_| KernelError::Runtime("remote snapshot failed".into()))?;
        if &remote != expected {
            return Err(KernelError::Configuration(
                "sandbox source changed; proof refused",
            ));
        }
        Ok(())
    }
}

fn backend_error(error: polaris_sdk::registry::SandboxError) -> KernelError {
    KernelError::Sandbox(error)
}

fn snapshot(root: &Path) -> Result<Manifest, KernelError> {
    let rules = sync::IgnoreRules::from_workspace(root)
        .map_err(|_| KernelError::Configuration("workspace ignore rules invalid"))?;
    let scan = sync::scan(root, &rules, None, sync::HashMode::Paranoid)
        .map_err(|_| KernelError::Runtime("workspace snapshot failed".into()))?;
    if !scan.escaped.is_empty()
        || !scan.unrepresentable.is_empty()
        || scan.manifest.entries.len() > 100_000
        || !sync::detect_conflicts(&scan.manifest).is_empty()
    {
        return Err(KernelError::Configuration(
            "workspace snapshot contains unsafe or excessive paths",
        ));
    }
    Ok(scan.manifest)
}
struct SnapshotView {
    files: Vec<FileFact>,
    dirty: BTreeSet<WorkspacePath>,
}
impl SnapshotView {
    fn new(root: &Path, manifest: &Manifest) -> Result<Self, KernelError> {
        let mut dirty = BTreeSet::new();
        if root.join(".git").exists() {
            let repo = polaris_vcs::Repo::open(root)
                .map_err(|_| KernelError::Configuration("repository facts unavailable"))?;
            for entry in repo
                .status()
                .map_err(|_| KernelError::Configuration("git status unavailable"))?
                .entries
            {
                dirty.insert(
                    WorkspacePath::new(entry.path.as_str())
                        .map_err(|_| KernelError::Configuration("dirty path invalid"))?,
                );
            }
        } else {
            // Without a repository baseline, conservatively treat every source as uncommitted.
            for path in manifest.entries.keys() {
                dirty.insert(
                    WorkspacePath::new(path.as_str())
                        .map_err(|_| KernelError::Configuration("source path invalid"))?,
                );
            }
        }
        let mut files = Vec::new();
        for (path, entry) in &manifest.entries {
            let path = WorkspacePath::new(path.as_str())
                .map_err(|_| KernelError::Configuration("source path invalid"))?;
            files.push(FileFact {
                uncommitted: dirty.contains(&path),
                path,
                bytes: entry.size,
            });
        }
        Ok(Self { files, dirty })
    }
}
impl WorkspaceView for SnapshotView {
    fn list_matching(&self, pattern: &GlobPattern) -> Result<Vec<FileFact>, guards::ViewError> {
        Ok(self
            .files
            .iter()
            .filter(|file| pattern.matches(&file.path))
            .cloned()
            .collect())
    }
    fn dirty_paths(&self) -> Result<BTreeSet<WorkspacePath>, guards::ViewError> {
        Ok(self.dirty.clone())
    }
}
struct OwnedSandbox<B> {
    backend: Arc<B>,
    handle: SandboxHandle,
}
impl<B: SandboxBackend> Teardown for OwnedSandbox<B> {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|_| TeardownError::Other("sandbox destruction failed".into()))?;
        Ok(Disposition::Clean)
    }
    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|_| TeardownError::Other("sandbox destruction failed".into()))
    }
}
