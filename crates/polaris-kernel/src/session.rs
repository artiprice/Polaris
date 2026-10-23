//! Durable, exclusively owned session typestate. Lifecycle records are authoritative; SQLite
//! is a rebuildable catalogue. Configuration and canonical directive text live in verified CAS.
//!
//! Invalid mutations are absent from the type's API:
//! ```compile_fail
//! use polaris_kernel::{Archived, Session};
//! use polaris_kernel::directives::directive::SessionDirective;
//! fn mutate(session: &mut Session<Archived>, directive: SessionDirective) {
//!     session.apply(directive);
//! }
//! ```
//! ```compile_fail
//! use polaris_kernel::{Paused, Session};
//! use polaris_kernel::directives::directive::SessionDirective;
//! fn mutate(session: &mut Session<Paused>, directive: SessionDirective) {
//!     session.apply(directive);
//! }
//! ```

use crate::{
    KernelCtx, KernelError,
    context::{SessionLease, blocking},
    directives::{
        ParseLimits, directive::SessionDirective, parse, render::render, state::DirectiveState,
    },
    storage::{
        cas::content_id,
        registry::{SessionMeta, SessionStatus},
    },
};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

const MAX_CONFIG_BYTES: usize = 64 << 10;
const CONFIG_VERSION: u16 = 1;

/// Unconfigured, unpublished session identity.
#[derive(Debug)]
pub struct Created;
/// Immutable configuration committed; session has not started.
#[derive(Debug)]
pub struct Configured;
/// Session is allowed to apply directives and execute verified work.
#[derive(Debug)]
pub struct Running;
/// Session retains its state but cannot mutate until resumed.
#[derive(Debug)]
pub struct Paused;
/// Archived session; reopen replays and returns a paused owner.
#[derive(Debug)]
pub struct Archived;

/// Trusted composition input; no credentials belong in session metadata or directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    /// Nonblank display name, at most 256 bytes.
    pub name: String,
    /// Existing workspace directory, canonicalized before committing.
    pub workspace: PathBuf,
    /// Git branch supplied by the composition root; empty means absent.
    pub branch: String,
    /// Base commit supplied by the composition root; empty means absent.
    pub base_commit: String,
    /// Session-local immutable defaults, applied in order.
    pub defaults: Vec<SessionDirective>,
}
#[derive(Debug, Serialize, Deserialize)]
struct ConfigDocument {
    version: u16,
    session: SessionId,
    name: String,
    workspace: PathBuf,
    branch: String,
    base_commit: String,
    defaults: Vec<String>,
}
#[derive(Debug, Serialize)]
struct WorkspaceDocument<'a> {
    root: &'a std::path::Path,
    branch: &'a str,
    base_commit: &'a str,
}
#[derive(Debug, Clone)]
struct SessionData {
    configuration: SessionConfig,
    workspace: Cid,
    created_at: Timestamp,
    prompt_tokens: u64,
    completion_tokens: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Configured,
    Running,
    Paused,
    Archived,
}

/// One mutable session owner. Dropping a running owner leaves durable state resumable; it never
/// manufactures an archive event. Archiving releases the live-owner lease immediately.
#[derive(Debug)]
pub struct Session<S> {
    id: SessionId,
    ctx: Arc<KernelCtx>,
    directives: DirectiveState,
    data: Option<SessionData>,
    last_seq: Seq,
    last_at: Timestamp,
    lease: Option<SessionLease>,
    state: PhantomData<S>,
}
impl<S> Session<S> {
    /// Stable session identity.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }
    /// Last lifecycle/directive event folded by this owner.
    #[must_use]
    pub const fn last_seq(&self) -> Seq {
        self.last_seq
    }
    /// Session-local typed settings after defaults and last-wins overrides.
    #[must_use]
    pub const fn directives(&self) -> &DirectiveState {
        &self.directives
    }
    /// Immutable configuration, absent only in the Created state.
    #[must_use]
    pub fn configuration(&self) -> Option<&SessionConfig> {
        self.data.as_ref().map(|data| &data.configuration)
    }
    fn transition<T>(self) -> Session<T> {
        Session {
            id: self.id,
            ctx: self.ctx,
            directives: self.directives,
            data: self.data,
            last_seq: self.last_seq,
            last_at: self.last_at,
            lease: self.lease,
            state: PhantomData,
        }
    }
    fn data(&self) -> Result<&SessionData, KernelError> {
        self.data.as_ref().ok_or(KernelError::Integrity(
            "configured session has no configuration",
        ))
    }
    fn project(&self, phase: Phase) -> Result<(), KernelError> {
        let data = self.data()?;
        let status = match phase {
            Phase::Running => SessionStatus::Active,
            Phase::Paused => SessionStatus::Paused,
            Phase::Archived => SessionStatus::Archived,
            Phase::Configured => return Ok(()),
        };
        let meta = SessionMeta {
            id: self.id,
            name: data.configuration.name.clone(),
            workspace: data.configuration.workspace.clone(),
            branch: data.configuration.branch.clone(),
            base_commit: data.configuration.base_commit.clone(),
            prompt_tokens: data.prompt_tokens,
            completion_tokens: data.completion_tokens,
            sandbox_checkpoint: None,
            status,
            created_at: data.created_at,
            updated_at: self.last_at,
        };
        self.ctx
            .registry
            .upsert_session(&meta)
            .map_err(|source| KernelError::ProjectionBehind {
                seq: self.last_seq,
                source,
            })
    }
    fn commit_transition(&mut self, event: Event, phase: Phase) -> Result<(), KernelError> {
        let ctx = Arc::clone(&self.ctx);
        let _writer = ctx.lock_writer()?;
        let record = ctx.publish(self.id, event)?;
        self.last_seq = record.seq();
        self.last_at = record.at();
        self.project(phase)
    }
}
fn parse_one(text: &str) -> Result<SessionDirective, KernelError> {
    let mut parsed = parse(text, &ParseLimits::default())?;
    if parsed.command.is_some()
        || !parsed.mentions.is_empty()
        || !parsed.prose.is_empty()
        || parsed.directives.len() != 1
    {
        return Err(KernelError::Integrity(
            "persisted directive is not exactly one directive",
        ));
    }
    parsed
        .directives
        .pop()
        .map(|item| item.value)
        .ok_or(KernelError::Integrity("missing persisted directive"))
}
fn validate(mut cfg: SessionConfig) -> Result<SessionConfig, KernelError> {
    if cfg.name.trim().is_empty() || cfg.name.len() > 256 || cfg.name.chars().any(char::is_control)
    {
        return Err(KernelError::Configuration("invalid display name"));
    }
    if cfg.branch.len() > 1024 || cfg.base_commit.len() > 1024 {
        return Err(KernelError::Configuration("Git metadata is too long"));
    }
    cfg.workspace = std::fs::canonicalize(&cfg.workspace)?;
    if !cfg.workspace.is_dir() || cfg.workspace.to_str().is_none() {
        return Err(KernelError::Configuration(
            "workspace must be a UTF-8 directory",
        ));
    }
    for directive in &cfg.defaults {
        if parse_one(&render(directive))? != *directive {
            return Err(KernelError::Configuration(
                "directive default does not roundtrip",
            ));
        }
    }
    Ok(cfg)
}
fn workspace_bytes(cfg: &SessionConfig) -> Result<Vec<u8>, KernelError> {
    Ok(serde_json::to_vec(&WorkspaceDocument {
        root: &cfg.workspace,
        branch: &cfg.branch,
        base_commit: &cfg.base_commit,
    })?)
}
impl Session<Created> {
    /// Claim one unpublished session id in the local context. No event is written yet.
    ///
    /// # Errors
    /// Closed context or another mutable owner for the id.
    pub fn create(id: SessionId, ctx: Arc<KernelCtx>) -> Result<Self, KernelError> {
        let lease = ctx.claim(id)?;
        Ok(Self {
            id,
            ctx,
            directives: DirectiveState::new(),
            data: None,
            last_seq: Seq::ZERO,
            last_at: Timestamp::UNIX_EPOCH,
            lease: Some(lease),
            state: PhantomData,
        })
    }
    /// Validate and durably configure. Blocking API: callers run this on their command owner
    /// thread, not a Tokio worker. The entire default set is one content-addressed document.
    ///
    /// # Errors
    /// Invalid configuration, reused session id, or CAS/event-log failure. Invalid input commits nothing.
    pub fn configure(mut self, cfg: SessionConfig) -> Result<Session<Configured>, KernelError> {
        let cfg = validate(cfg)?;
        let ctx = Arc::clone(&self.ctx);
        let _writer = ctx.lock_writer()?;
        ctx.ensure_open()?;
        if ctx
            .records(Seq::ZERO, Seq::new(u64::MAX))?
            .iter()
            .any(|record| record.session() == self.id)
        {
            return Err(KernelError::SessionExists(self.id));
        }
        let document = ConfigDocument {
            version: CONFIG_VERSION,
            session: self.id,
            name: cfg.name.clone(),
            workspace: cfg.workspace.clone(),
            branch: cfg.branch.clone(),
            base_commit: cfg.base_commit.clone(),
            defaults: cfg.defaults.iter().map(render).collect(),
        };
        let bytes = serde_json::to_vec(&document)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(KernelError::Configuration(
                "session configuration exceeds 64 KiB",
            ));
        }
        let configuration = ctx.cas.put(&bytes)?;
        let workspace = ctx.cas.put(&workspace_bytes(&cfg)?)?;
        let record = ctx.publish(self.id, Event::SessionConfigured { configuration })?;
        self.last_seq = record.seq();
        self.last_at = record.at();
        self.directives = DirectiveState::with_defaults(cfg.defaults.iter().cloned());
        self.data = Some(SessionData {
            configuration: cfg,
            workspace,
            created_at: record.at(),
            prompt_tokens: 0,
            completion_tokens: 0,
        });
        Ok(self.transition())
    }
}
impl Session<Configured> {
    /// Commit startup and create the registry row. Disk I/O runs on a blocking worker.
    ///
    /// # Errors
    /// Append failures, closed context, or `ProjectionBehind` after a successful durable commit.
    pub async fn start(mut self) -> Result<Session<Running>, KernelError> {
        blocking(move || {
            let workspace = self.data()?.workspace;
            self.commit_transition(Event::SessionStarted { workspace }, Phase::Running)?;
            Ok(self.transition())
        })
        .await
    }
}
impl Session<Running> {
    /// Commit one trusted, already-verified directive before updating memory. Blocking API;
    /// the eventual command dispatcher owns its verification boundary and dedicated I/O thread.
    ///
    /// # Errors
    /// Invalid canonical directive, append failure, closed context, or committed projection failure.
    pub fn apply(&mut self, directive: SessionDirective) -> Result<Seq, KernelError> {
        let text = render(&directive);
        if parse_one(&text)? != directive {
            return Err(KernelError::Configuration("directive does not roundtrip"));
        }
        let ctx = Arc::clone(&self.ctx);
        let _writer = ctx.lock_writer()?;
        ctx.ensure_open()?;
        let cid = ctx.cas.put(text.as_bytes())?;
        let record = ctx.publish(self.id, Event::DirectiveApplied { directive: cid })?;
        self.last_seq = record.seq();
        self.last_at = record.at();
        let _previous = self.directives.apply(directive);
        self.project(Phase::Running)?;
        Ok(self.last_seq)
    }
    /// Persist pause before returning a non-mutating owner.
    ///
    /// # Errors
    /// Durable append or registry projection failed.
    pub async fn pause(mut self) -> Result<Session<Paused>, KernelError> {
        blocking(move || {
            self.commit_transition(Event::SessionPaused, Phase::Paused)?;
            Ok(self.transition())
        })
        .await
    }
    /// Archive durably, then release the live-owner lease so the archive can be reopened.
    ///
    /// # Errors
    /// Durable append or registry projection failed.
    pub async fn archive(mut self) -> Result<Session<Archived>, KernelError> {
        blocking(move || {
            self.commit_transition(Event::SessionClosed, Phase::Archived)?;
            let _lease = self.lease.take();
            Ok(self.transition())
        })
        .await
    }
}
impl Session<Paused> {
    /// Resume only after confirming the original workspace still exists.
    ///
    /// # Errors
    /// Missing workspace, closed context, append or projection failure.
    pub async fn resume(mut self) -> Result<Session<Running>, KernelError> {
        blocking(move || {
            if !self.data()?.configuration.workspace.is_dir() {
                return Err(KernelError::Configuration("workspace is unavailable"));
            }
            self.commit_transition(Event::SessionResumed, Phase::Running)?;
            Ok(self.transition())
        })
        .await
    }
}
impl Session<Archived> {
    /// Rebuild a session from verified CAS documents and ordered events, then return a paused
    /// owner. Running/archived records acquire a durable pause before recovery is exposed.
    ///
    /// # Errors
    /// Unknown/incomplete session, competing owner, invalid lifecycle order, missing/corrupt CAS,
    /// closed context or registry rebuild failure. Nothing is silently skipped.
    pub async fn reopen(
        id: SessionId,
        ctx: Arc<KernelCtx>,
    ) -> Result<Session<Paused>, KernelError> {
        blocking(move || {
            let lease = ctx.claim(id)?;
            let _writer = ctx.lock_writer()?;
            ctx.ensure_open()?;
            let (data, directives, phase, last_seq, last_at) = replay(&ctx, id)?;
            if phase == Phase::Configured {
                return Err(KernelError::NotStarted(id));
            }
            let mut session = Session::<Paused> {
                id,
                ctx: Arc::clone(&ctx),
                directives,
                data: Some(data),
                last_seq,
                last_at,
                lease: Some(lease),
                state: PhantomData,
            };
            if phase != Phase::Paused {
                let record = ctx.publish(id, Event::SessionPaused)?;
                session.last_seq = record.seq();
                session.last_at = record.at();
            }
            session.project(Phase::Paused)?;
            Ok(session)
        })
        .await
    }
}

fn replay(
    ctx: &KernelCtx,
    id: SessionId,
) -> Result<(SessionData, DirectiveState, Phase, Seq, Timestamp), KernelError> {
    let mut data: Option<SessionData> = None;
    let mut directives = DirectiveState::new();
    let mut phase = None;
    let mut last_seq = Seq::ZERO;
    let mut last_at = Timestamp::UNIX_EPOCH;
    let head = ctx.bus.store().head()?;
    let mut from = Seq::new(1);
    // Bound replay working memory independently of the lifetime of the global event journal.
    while from <= head {
        let end = Seq::new(from.get().saturating_add(255).min(head.get()));
        let records = ctx.records(from, end)?;
        let expected = end.get().saturating_sub(from.get()).saturating_add(1);
        if u64::try_from(records.len()).ok() != Some(expected) {
            return Err(KernelError::Integrity("gap in durable global log"));
        }
        for (offset, record) in (0u64..).zip(records) {
            if record.seq().get() != from.get().saturating_add(offset) {
                return Err(KernelError::Integrity(
                    "event key/envelope sequence mismatch",
                ));
            }
            if record.session() != id {
                continue;
            }
            fold(ctx, id, &record, &mut data, &mut directives, &mut phase)?;
            last_seq = record.seq();
            last_at = record.at();
        }
        let Some(next) = end.next() else {
            break;
        };
        from = next;
    }
    Ok((
        data.ok_or(KernelError::UnknownSession(id))?,
        directives,
        phase.ok_or(KernelError::UnknownSession(id))?,
        last_seq,
        last_at,
    ))
}
fn fold(
    ctx: &KernelCtx,
    id: SessionId,
    record: &EventRecord,
    data: &mut Option<SessionData>,
    directives: &mut DirectiveState,
    phase: &mut Option<Phase>,
) -> Result<(), KernelError> {
    match *record.event() {
        Event::SessionConfigured { configuration } => {
            if phase.is_some() {
                return Err(KernelError::Integrity("duplicate session configuration"));
            }
            let bytes = ctx.cas.get(&configuration)?;
            if bytes.len() > MAX_CONFIG_BYTES {
                return Err(KernelError::Integrity("oversize configuration document"));
            }
            let document: ConfigDocument = serde_json::from_slice(&bytes)?;
            if document.version != CONFIG_VERSION || document.session != id {
                return Err(KernelError::Integrity(
                    "configuration document version/identity mismatch",
                ));
            }
            let defaults = document
                .defaults
                .iter()
                .map(|text| parse_one(text))
                .collect::<Result<Vec<_>, _>>()?;
            let cfg = SessionConfig {
                name: document.name,
                workspace: document.workspace,
                branch: document.branch,
                base_commit: document.base_commit,
                defaults,
            };
            let workspace = content_id(&workspace_bytes(&cfg)?);
            *directives = DirectiveState::with_defaults(cfg.defaults.iter().cloned());
            *data = Some(SessionData {
                configuration: cfg,
                workspace,
                created_at: record.at(),
                prompt_tokens: 0,
                completion_tokens: 0,
            });
            *phase = Some(Phase::Configured);
        }
        Event::SessionStarted { workspace } => {
            if *phase != Some(Phase::Configured) {
                return Err(KernelError::Integrity("startup without one configuration"));
            }
            if data.as_ref().is_none_or(|data| data.workspace != workspace) {
                return Err(KernelError::Integrity(
                    "workspace descriptor disagrees with configuration",
                ));
            }
            let _verified = ctx.cas.get(&workspace)?;
            *phase = Some(Phase::Running);
        }
        Event::SessionPaused => {
            if !matches!(phase, Some(Phase::Running | Phase::Archived)) {
                return Err(KernelError::Integrity("invalid pause/reopen transition"));
            }
            *phase = Some(Phase::Paused);
        }
        Event::SessionResumed => {
            if *phase != Some(Phase::Paused) {
                return Err(KernelError::Integrity("resume without pause"));
            }
            *phase = Some(Phase::Running);
        }
        Event::SessionClosed => {
            if *phase != Some(Phase::Running) {
                return Err(KernelError::Integrity("archive without running session"));
            }
            *phase = Some(Phase::Archived);
        }
        Event::DirectiveApplied { directive } => {
            if *phase != Some(Phase::Running) {
                return Err(KernelError::Integrity("directive outside running session"));
            }
            let bytes = ctx.cas.get(&directive)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| KernelError::Integrity("directive CAS document is not UTF-8"))?;
            let _previous = directives.apply(parse_one(text)?);
        }
        Event::ModelRequestCompleted {
            prompt_tokens,
            completion_tokens,
            ..
        } => {
            let Some(data) = data.as_mut() else {
                return Err(KernelError::Integrity(
                    "request precedes session configuration",
                ));
            };
            data.prompt_tokens = data
                .prompt_tokens
                .saturating_add(u64::from(prompt_tokens.get()));
            data.completion_tokens = data
                .completion_tokens
                .saturating_add(u64::from(completion_tokens.get()));
        }
        Event::DecisionMade { .. }
        | Event::ToolExecutionStarted { .. }
        | Event::ToolExecutionCompleted { .. }
        | Event::DiffApproved { .. }
        | Event::DiffRejected { .. }
        | Event::DiffNeedsApproval { .. }
        | Event::FileLeaseGranted { .. }
        | Event::FileLeaseRevoked { .. }
        | Event::SandboxSnapshotBranchCreated { .. }
        | Event::SandboxExecutionCompleted { .. }
        | Event::ProofOfDonePassed { .. }
        | Event::ProofOfDoneBlocked { .. }
        | Event::WorkerRestarted { .. }
        | Event::CircuitTripped { .. }
        | Event::DeadLettered { .. }
        | Event::BlastRadiusAlert { .. }
        | Event::FrameStepRecorded { .. } => {}
    }
    Ok(())
}
