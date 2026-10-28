//! Durable cancellation registry for actual sealed runtime frame trees.
//!
//! Frame parentage comes from `FrameToolBinding`, never from caller-selected ids or cancellation
//! token topology. The registry owns each canonical frame token and appends cancellation intent
//! before waking only the verified target subtree.

use crate::{KernelCtx, KernelError, KernelEvidence, storage::cas::content_id};
use polaris_kernel_verifier::{
    FrameCancellationEvidence, FrameCancellationVerifierError, load_cancellation_evidence,
    validate_cancellation_evidence,
};
use polaris_runtime::frames::FrameToolBinding;
use polaris_sdk::{
    contracts::{Command, CommandBody, Event, EventRecord, Verified},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const MAX_SESSION_FRAMES: usize = 256;
const MAX_FRAME_DEPTH: usize = 32;
const MAX_CANCELLATION_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_REPLAY_RECORDS: u64 = u64::MAX;
const CANCELLATION_APPROVER: &str = "polaris-frame-cancellation-verifier";

/// Maximum number of frames accepted into one session registry.
pub const MAX_REGISTERED_FRAMES_PER_SESSION: usize = MAX_SESSION_FRAMES;

/// Maximum registered ancestor depth, including the session root.
pub const MAX_REGISTERED_FRAME_DEPTH: usize = MAX_FRAME_DEPTH;

/// Returned handle for the one canonical cancellation token owned by a registered frame.
#[derive(Clone)]
pub struct FrameCancellationRegistration {
    frame: FrameId,
    token: CancellationToken,
}

impl std::fmt::Debug for FrameCancellationRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameCancellationRegistration")
            .field("frame", &self.frame)
            .field("cancelled", &self.token.is_cancelled())
            .finish()
    }
}

impl FrameCancellationRegistration {
    /// Registered frame identity.
    #[must_use]
    pub const fn frame_id(&self) -> FrameId {
        self.frame
    }

    /// Clone the canonical frame token for the runtime runner.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

/// Durable result of one committed frame-tree cancellation intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCancellationOutcome {
    /// Exact cancellation target.
    pub target: FrameId,
    /// Number of registered frames in the committed subtree snapshot.
    pub cancelled_frames: u16,
    /// Event-log position of the durable intent.
    pub seq: Seq,
    /// CAS content id for the complete cancellation receipt.
    pub receipt: Cid,
}

#[derive(Debug)]
struct RegisteredFrame {
    parent: Option<FrameId>,
    root: FrameId,
    role: polaris_sdk::ids::AgentRole,
    binding: Cid,
    token: CancellationToken,
    cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandReceipt {
    target: FrameId,
    receipt: Cid,
    seq: Seq,
}

#[derive(Debug)]
struct RegistryState {
    frames: BTreeMap<FrameId, RegisteredFrame>,
    commands: BTreeMap<CommandId, CommandReceipt>,
    outcomes: BTreeMap<FrameId, FrameCancellationOutcome>,
}

// The context owns registry state without a back-reference to the context, preventing Arc cycles.
#[derive(Debug)]
pub(crate) struct FrameRegistryCache {
    root: FrameId,
    root_token: CancellationToken,
    state: Arc<Mutex<RegistryState>>,
}

/// Trusted service for registering sealed frame identities and applying verified tree cancels.
#[derive(Debug, Clone)]
pub struct FrameCancellationService {
    context: Arc<KernelCtx>,
    session: SessionId,
    root: FrameId,
    root_token: CancellationToken,
    state: Arc<Mutex<RegistryState>>,
}

impl FrameCancellationService {
    /// Share the context-owned session registry, replaying and validating it on first construction.
    ///
    /// # Errors
    /// Rejects closed/unknown sessions, malformed history or corrupt registration/cancellation
    /// receipts. Canonical frame tokens are reconstructed beneath the current session stop source.
    pub fn new(
        context: Arc<KernelCtx>,
        session: SessionId,
        root: FrameId,
        session_stop: CancellationToken,
    ) -> Result<Self, FrameCancellationError> {
        context.ensure_open()?;
        let mut registries = context
            .frame_registries
            .lock()
            .map_err(|_| FrameCancellationError::Poisoned)?;
        if let Some(existing) = registries.get(&session) {
            if existing.root != root {
                return Err(FrameCancellationError::Rejected(
                    "session registry root mismatch",
                ));
            }
            return Ok(Self {
                context: Arc::clone(&context),
                session,
                root,
                root_token: existing.root_token.clone(),
                state: Arc::clone(&existing.state),
            });
        }
        if registries.len() >= 64 {
            return Err(FrameCancellationError::Rejected(
                "context session registry bound exceeded",
            ));
        }

        let _writer = context.lock_writer()?;
        let records = context
            .bus
            .store()
            .replay_iter(Seq::new(MAX_REPLAY_RECORDS))
            .map_err(KernelError::from)?;
        let evidence = KernelEvidence::new(Arc::clone(&context));
        let root_token = session_stop.child_token();
        let mut state = RegistryState {
            frames: BTreeMap::new(),
            commands: BTreeMap::new(),
            outcomes: BTreeMap::new(),
        };
        let mut seen_session = false;
        for record in records {
            let record = record.map_err(KernelError::from)?;
            if record.session() != session {
                continue;
            }
            seen_session = true;
            restore_record(
                &context,
                &evidence,
                &mut state,
                record,
                session,
                root,
                &root_token,
            )?;
        }
        if !seen_session {
            return Err(KernelError::UnknownSession(session).into());
        }

        let state = Arc::new(Mutex::new(state));
        registries.insert(
            session,
            FrameRegistryCache {
                root,
                root_token: root_token.clone(),
                state: Arc::clone(&state),
            },
        );
        drop(_writer);
        drop(registries);
        Ok(Self {
            context,
            session,
            root,
            root_token,
            state,
        })
    }

    /// Register the actual sealed binding and return its canonical private cancellation token.
    ///
    /// The root's canonical token is already scoped as a child of the session stop token supplied
    /// to `new`. A descendant's parent identity comes from the binding and its canonical token is
    /// derived internally from the already-registered actual parent's token.
    ///
    /// # Errors
    /// Rejects binding/session/root mismatch, absent or cancelled actual parent, wrong external
    /// token placement, duplicate identities, frame/depth limits and durable write failures.
    pub async fn register(
        &self,
        binding: FrameToolBinding,
    ) -> Result<FrameCancellationRegistration, FrameCancellationError> {
        let service = self.clone();
        run_blocking(move || service.bind_owned(binding, false)).await
    }

    /// Blocking cold composition boundary; exact repeated bindings reuse the canonical identity.
    pub(crate) fn bind_blocking(
        &self,
        binding: FrameToolBinding,
    ) -> Result<FrameCancellationRegistration, FrameCancellationError> {
        let mut lineage = Vec::new();
        let mut current = Some(&binding);
        while let Some(frame) = current {
            if lineage.len() >= MAX_FRAME_DEPTH {
                return Err(FrameCancellationError::Rejected(
                    "sealed ancestry exceeds bound",
                ));
            }
            if frame.parent_frame_id() != frame.parent_binding().map(FrameToolBinding::frame_id) {
                return Err(FrameCancellationError::Integrity(
                    "sealed ancestry parent mismatch",
                ));
            }
            lineage.push(frame.clone());
            current = frame.parent_binding();
        }
        let mut registered = None;
        for frame in lineage.into_iter().rev() {
            registered = Some(self.bind_owned(frame, true)?);
        }
        registered.ok_or(FrameCancellationError::UnknownFrame)
    }
    fn bind_owned(
        &self,
        binding: FrameToolBinding,
        reuse: bool,
    ) -> Result<FrameCancellationRegistration, FrameCancellationError> {
        let binding_id = binding.frame_id();
        let parent_id = binding.parent_frame_id();
        let root_id = binding.root_frame_id();
        let role = binding.role();
        let binding_digest = binding.identity_digest();
        let context = &self.context;
        let state = &self.state;
        let session = self.session;
        let root = self.root;
        let root_token = self.root_token.clone();
        context.ensure_open()?;
        let _writer = context.lock_writer()?;
        let mut state = state.lock().map_err(|_| FrameCancellationError::Poisoned)?;
        if binding_id == root {
            if parent_id.is_some() || root_id != binding_id {
                return Err(FrameCancellationError::Rejected(
                    "invalid root frame identity",
                ));
            }
        } else if root_id != root || parent_id.is_none() {
            return Err(FrameCancellationError::Rejected(
                "invalid child frame identity",
            ));
        }
        if let Some(existing) = state.frames.get(&binding_id) {
            if reuse
                && existing.parent == parent_id
                && existing.root == root_id
                && existing.role == role
                && existing.binding == binding_digest
            {
                return Ok(FrameCancellationRegistration {
                    frame: binding_id,
                    token: existing.token.clone(),
                });
            }
            return Err(FrameCancellationError::Rejected(
                "frame identity already registered",
            ));
        }
        if state.frames.len() >= MAX_SESSION_FRAMES {
            return Err(FrameCancellationError::Rejected(
                "session frame limit reached",
            ));
        }
        let (token, _depth) = match parent_id {
            Some(parent) => {
                let parent_state =
                    state
                        .frames
                        .get(&parent)
                        .ok_or(FrameCancellationError::Rejected(
                            "actual parent is not registered",
                        ))?;
                if parent_state.root != root_id || parent_state.cancelled {
                    return Err(FrameCancellationError::Rejected(
                        "actual parent is cancelled or belongs to another root",
                    ));
                }
                let depth = frame_depth(&state.frames, parent)?
                    .checked_add(1)
                    .ok_or(FrameCancellationError::Integrity("frame depth overflow"))?;
                if depth > MAX_FRAME_DEPTH {
                    return Err(FrameCancellationError::Rejected(
                        "frame depth limit reached",
                    ));
                }
                (parent_state.token.child_token(), depth)
            }
            None => (root_token, 1),
        };
        let record = context.publish(
            session,
            Event::FrameRegistered {
                frame: binding_id,
                parent: parent_id,
                root: root_id,
                role,
                binding: binding_digest,
            },
        )?;
        let _record = record;
        state.frames.insert(
            binding_id,
            RegisteredFrame {
                parent: parent_id,
                root: root_id,
                role,
                binding: binding_digest,
                token: token.clone(),
                cancelled: false,
            },
        );
        Ok(FrameCancellationRegistration {
            frame: binding_id,
            token,
        })
    }

    /// Apply a host-root verified `CancelFrame` after validating its dedicated L4 evidence.
    ///
    /// The exact registered subtree snapshot is calculated while holding the kernel writer and
    /// registry lock, then persisted in CAS before the event is appended and tokens are cancelled.
    /// Frame registration uses the same locks, so a new child cannot race past the receipt.
    ///
    /// # Errors
    /// Rejects non-cancel commands, wrong root actor, missing verifier evidence, unregistered or
    /// already-cancelled targets, conflicting command ids, over-limit receipts or I/O failures.
    pub async fn apply_verified(
        &self,
        command: Command<Verified>,
    ) -> Result<FrameCancellationOutcome, FrameCancellationError> {
        let CommandBody::CancelFrame { frame: target } = *command.body() else {
            return Err(FrameCancellationError::Rejected(
                "not a CancelFrame command",
            ));
        };
        let actor = command.frame();
        let command_id = command.id();
        let approval = command.witness().evidence();
        if actor != self.root || command.witness().approver().as_str() != CANCELLATION_APPROVER {
            return Err(FrameCancellationError::Rejected(
                "wrong cancellation authority",
            ));
        }
        let evidence = KernelEvidence::new(Arc::clone(&self.context));
        let token_bytes = command.token().clone();
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        let session = self.session;
        let root = self.root;
        let evidence_context = evidence.clone();
        run_blocking(move || {
            context.ensure_open()?;
            validate_cancellation_evidence(
                &evidence_context,
                approval,
                session,
                actor,
                target,
                command_id,
                &token_bytes,
            )?;
            let _writer = context.lock_writer()?;
            let mut state = state.lock().map_err(|_| FrameCancellationError::Poisoned)?;
            if let Some(receipt) = state.commands.get(&command_id).copied() {
                if receipt.target == target && receipt.receipt == approval {
                    let outcome = state
                        .outcomes
                        .get(&target)
                        .copied()
                        .ok_or(FrameCancellationError::Integrity("missing command outcome"))?;
                    if outcome.seq == receipt.seq {
                        return Ok(outcome);
                    }
                    return Err(FrameCancellationError::Integrity(
                        "command sequence mismatch",
                    ));
                }
                return Err(FrameCancellationError::Rejected("conflicting command id"));
            }
            if let Some(existing) = state.outcomes.get(&target).copied() {
                return Ok(existing);
            }
            let target_frame =
                state
                    .frames
                    .get(&target)
                    .ok_or(FrameCancellationError::Rejected(
                        "target frame is not registered",
                    ))?;
            if target_frame.cancelled {
                for (cancelled_target, outcome) in &state.outcomes {
                    if is_descendant_or_self(&state.frames, target, *cancelled_target)? {
                        return Ok(*outcome);
                    }
                }
                return Err(FrameCancellationError::Integrity(
                    "cancelled target has no outcome",
                ));
            }
            let descendants = subtree_records(&state.frames, target)?;
            let receipt = CancellationReceipt {
                schema: 1,
                session,
                root,
                actor,
                target,
                command: command_id,
                approval,
                token_digest: *content_id(token_bytes.as_bytes()).as_bytes(),
                frames: descendants.clone(),
            };
            let receipt_bytes =
                serde_json::to_vec(&receipt).map_err(|_| FrameCancellationError::Encoding)?;
            if u64::try_from(receipt_bytes.len()).map_err(|_| FrameCancellationError::Encoding)?
                > MAX_CANCELLATION_RECEIPT_BYTES
            {
                return Err(FrameCancellationError::Rejected(
                    "cancellation receipt too large",
                ));
            }
            let receipt_id = context.cas.put(&receipt_bytes)?;
            if receipt_id != content_id(&receipt_bytes)
                || context
                    .cas
                    .get_bounded(&receipt_id, MAX_CANCELLATION_RECEIPT_BYTES)?
                    .as_ref()
                    != receipt_bytes.as_slice()
            {
                return Err(FrameCancellationError::Integrity(
                    "cancellation receipt CID mismatch",
                ));
            }
            let record = context.publish(
                session,
                Event::FrameTreeCancellationRequested {
                    target,
                    receipt: receipt_id,
                    command: command_id,
                },
            )?;
            let cancelled_frames = u16::try_from(descendants.len())
                .map_err(|_| FrameCancellationError::Integrity("subtree size overflow"))?;
            let outcome = FrameCancellationOutcome {
                target,
                cancelled_frames,
                seq: record.seq(),
                receipt: receipt_id,
            };
            for member in descendants {
                let frame = state.frames.get_mut(&member.frame).ok_or(
                    FrameCancellationError::Integrity("subtree member disappeared"),
                )?;
                frame.cancelled = true;
                frame.token.cancel();
            }
            state.commands.insert(
                command_id,
                CommandReceipt {
                    target,
                    receipt: approval,
                    seq: record.seq(),
                },
            );
            state.outcomes.insert(target, outcome);
            Ok(outcome)
        })
        .await
    }

    /// Whether a registered frame has a durable cancellation intent.
    /// # Errors
    /// Returns a typed error for an unknown frame or poisoned registry.
    pub fn is_cancelled(&self, frame: FrameId) -> Result<bool, FrameCancellationError> {
        let state = self
            .state
            .lock()
            .map_err(|_| FrameCancellationError::Poisoned)?;
        state
            .frames
            .get(&frame)
            .map(|entry| entry.cancelled)
            .ok_or(FrameCancellationError::UnknownFrame)
    }

    /// Fetch the canonical token for a registered frame, including an already-cancelled token.
    /// This is used only during trusted runner rehydration; cancelled tokens stay cancelled.
    /// # Errors
    /// Returns a typed error for an unknown frame or poisoned registry.
    pub fn token_for(&self, frame: FrameId) -> Result<CancellationToken, FrameCancellationError> {
        let state = self
            .state
            .lock()
            .map_err(|_| FrameCancellationError::Poisoned)?;
        state
            .frames
            .get(&frame)
            .map(|entry| entry.token.clone())
            .ok_or(FrameCancellationError::UnknownFrame)
    }
}

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, FrameCancellationError> + Send + 'static,
) -> Result<T, FrameCancellationError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| FrameCancellationError::Owner)?
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameRegistrationReceipt {
    frame: FrameId,
    parent: Option<FrameId>,
    root: FrameId,
    role: polaris_sdk::ids::AgentRole,
    binding: Cid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancellationReceipt {
    schema: u16,
    session: SessionId,
    root: FrameId,
    actor: FrameId,
    target: FrameId,
    command: CommandId,
    approval: Cid,
    token_digest: [u8; 32],
    frames: Vec<FrameRegistrationReceipt>,
}

fn registration_receipt(frame: FrameId, entry: &RegisteredFrame) -> FrameRegistrationReceipt {
    FrameRegistrationReceipt {
        frame,
        parent: entry.parent,
        root: entry.root,
        role: entry.role,
        binding: entry.binding,
    }
}

fn subtree_records(
    frames: &BTreeMap<FrameId, RegisteredFrame>,
    target: FrameId,
) -> Result<Vec<FrameRegistrationReceipt>, FrameCancellationError> {
    let mut members = Vec::new();
    for (frame, entry) in frames {
        if is_descendant_or_self(frames, *frame, target)? {
            members.push(registration_receipt(*frame, entry));
        }
    }
    if members.is_empty() {
        return Err(FrameCancellationError::UnknownFrame);
    }
    Ok(members)
}

fn is_descendant_or_self(
    frames: &BTreeMap<FrameId, RegisteredFrame>,
    mut frame: FrameId,
    target: FrameId,
) -> Result<bool, FrameCancellationError> {
    for _ in 0..=MAX_FRAME_DEPTH {
        if frame == target {
            return Ok(true);
        }
        let Some(entry) = frames.get(&frame) else {
            return Err(FrameCancellationError::Integrity("frame parent is missing"));
        };
        let Some(parent) = entry.parent else {
            return Ok(false);
        };
        frame = parent;
    }
    Err(FrameCancellationError::Integrity("frame parent cycle"))
}

fn frame_depth(
    frames: &BTreeMap<FrameId, RegisteredFrame>,
    mut frame: FrameId,
) -> Result<usize, FrameCancellationError> {
    let mut depth = 1_usize;
    for _ in 0..MAX_FRAME_DEPTH {
        let entry = frames
            .get(&frame)
            .ok_or(FrameCancellationError::Integrity("frame parent is missing"))?;
        let Some(parent) = entry.parent else {
            return Ok(depth);
        };
        depth = depth
            .checked_add(1)
            .ok_or(FrameCancellationError::Integrity("frame depth overflow"))?;
        frame = parent;
    }
    Err(FrameCancellationError::Integrity("frame parent cycle"))
}

fn restore_record(
    context: &KernelCtx,
    evidence: &KernelEvidence,
    state: &mut RegistryState,
    record: EventRecord,
    session: SessionId,
    root: FrameId,
    root_token: &CancellationToken,
) -> Result<(), FrameCancellationError> {
    match *record.event() {
        Event::FrameRegistered {
            frame,
            parent,
            root: event_root,
            role,
            binding,
        } => {
            if event_root != root || state.frames.contains_key(&frame) {
                return Err(FrameCancellationError::Integrity(
                    "invalid or duplicate frame registration",
                ));
            }
            let token =
                match parent {
                    Some(parent_id) => {
                        let parent_entry = state.frames.get(&parent_id).ok_or(
                            FrameCancellationError::Integrity("registration parent missing"),
                        )?;
                        if parent_entry.root != root || parent_entry.cancelled {
                            return Err(FrameCancellationError::Integrity(
                                "registration follows cancelled or foreign parent",
                            ));
                        }
                        if frame_depth(&state.frames, parent_id)? >= MAX_FRAME_DEPTH {
                            return Err(FrameCancellationError::Integrity(
                                "registered depth exceeded",
                            ));
                        }
                        parent_entry.token.child_token()
                    }
                    None if frame == root => root_token.clone(),
                    None => {
                        return Err(FrameCancellationError::Integrity(
                            "non-root registration has no parent",
                        ));
                    }
                };
            if parent.is_none() && frame != root {
                return Err(FrameCancellationError::Integrity(
                    "foreign root registration",
                ));
            }
            if state.frames.len() >= MAX_SESSION_FRAMES {
                return Err(FrameCancellationError::Integrity(
                    "session frame limit exceeded",
                ));
            }
            state.frames.insert(
                frame,
                RegisteredFrame {
                    parent,
                    root,
                    role,
                    binding,
                    token,
                    cancelled: false,
                },
            );
        }
        Event::FrameTreeCancellationRequested {
            target,
            receipt,
            command,
        } => {
            let bytes = context
                .cas
                .get_bounded(&receipt, MAX_CANCELLATION_RECEIPT_BYTES)?;
            if content_id(&bytes) != receipt {
                return Err(FrameCancellationError::Integrity(
                    "cancel receipt CID mismatch",
                ));
            }
            let saved: CancellationReceipt = serde_json::from_slice(&bytes)
                .map_err(|_| FrameCancellationError::Integrity("malformed cancel receipt"))?;
            if saved.schema != 1
                || saved.session != session
                || saved.root != root
                || saved.actor != root
                || saved.target != target
                || saved.command != command
            {
                return Err(FrameCancellationError::Integrity(
                    "cancel receipt identity mismatch",
                ));
            }
            let _proof: FrameCancellationEvidence = load_cancellation_evidence(
                evidence,
                saved.approval,
                session,
                root,
                target,
                command,
            )?;
            let expected = subtree_records(&state.frames, target)?;
            if expected != saved.frames
                || saved.token_digest != _proof.token_digest
                || state.outcomes.contains_key(&target)
            {
                return Err(FrameCancellationError::Integrity(
                    "cancel receipt subtree mismatch",
                ));
            }
            let count = u16::try_from(saved.frames.len())
                .map_err(|_| FrameCancellationError::Integrity("cancel subtree size overflow"))?;
            for member in &saved.frames {
                let frame = state.frames.get_mut(&member.frame).ok_or(
                    FrameCancellationError::Integrity("cancel subtree member missing"),
                )?;
                frame.cancelled = true;
                frame.token.cancel();
            }
            let outcome = FrameCancellationOutcome {
                target,
                cancelled_frames: count,
                seq: record.seq(),
                receipt,
            };
            if state
                .commands
                .insert(
                    command,
                    CommandReceipt {
                        target,
                        receipt: saved.approval,
                        seq: record.seq(),
                    },
                )
                .is_some()
            {
                return Err(FrameCancellationError::Integrity(
                    "cancel command id reused",
                ));
            }
            state.outcomes.insert(target, outcome);
        }
        Event::SessionStarted { .. }
        | Event::DirectiveApplied { .. }
        | Event::UserInputSubmitted { .. }
        | Event::DecisionMade { .. }
        | Event::ModelRequestCompleted { .. }
        | Event::ToolExecutionStarted { .. }
        | Event::ToolExecutionCompleted { .. }
        | Event::DiffApproved { .. }
        | Event::DiffRejected { .. }
        | Event::FileLeaseGranted { .. }
        | Event::FileLeaseRevoked { .. }
        | Event::SandboxSnapshotBranchCreated { .. }
        | Event::SandboxExecutionCompleted { .. }
        | Event::ProofOfDonePassed { .. }
        | Event::ProofOfDoneBlocked { .. }
        | Event::WorkerRestarted { .. }
        | Event::CircuitTripped { .. }
        | Event::DeadLettered { .. }
        | Event::SessionClosed
        | Event::SessionConfigured { .. }
        | Event::SessionPaused
        | Event::SessionResumed
        | Event::BlastRadiusAlert { .. }
        | Event::FrameStepRecorded { .. }
        | Event::DiffNeedsApproval { .. }
        | Event::HumanRequestCreated { .. }
        | Event::HumanRequestAnswered { .. }
        | Event::HumanRequestInterrupted { .. }
        | Event::FrameToolResultRecorded { .. } => {}
    }
    Ok(())
}

/// Fail-closed errors from the durable actual frame-tree cancellation service.
#[derive(Debug, thiserror::Error)]
pub enum FrameCancellationError {
    /// Frame identity, actor, parentage or command was rejected by policy.
    #[error("frame cancellation rejected: {0}")]
    Rejected(&'static str),
    /// A requested frame is absent from this session's registry.
    #[error("unknown frame")]
    UnknownFrame,
    /// A poisoned process-local state lock was detected.
    #[error("frame cancellation registry poisoned")]
    Poisoned,
    /// Persisted registrations, receipts or event history are inconsistent or corrupt.
    #[error("frame cancellation replay integrity failure: {0}")]
    Integrity(&'static str),
    /// Receipt serialization or bounded representation failed.
    #[error("frame cancellation receipt encoding failed")]
    Encoding,
    /// Kernel event/CAS storage failed.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// CAS payload read or write failed.
    #[error(transparent)]
    Cas(#[from] crate::storage::cas::CasError),
    /// Cancellation verifier rejected or could not validate approval evidence.
    #[error(transparent)]
    Verifier(#[from] FrameCancellationVerifierError),
    /// Blocking owner task failed.
    #[error("frame cancellation storage owner failed")]
    Owner,
}

// Caller holds KernelCtx's writer: a new request/answer is ordered against the durable intent.
pub(crate) fn durable_frame_cancelled(
    context: &Arc<KernelCtx>,
    session: SessionId,
    root: FrameId,
    frame: FrameId,
) -> Result<bool, FrameCancellationError> {
    let evidence = KernelEvidence::new(Arc::clone(context));
    for record in context
        .bus
        .store()
        .replay_iter(Seq::new(u64::MAX))
        .map_err(KernelError::from)?
    {
        let record = record.map_err(KernelError::from)?;
        if record.session() != session {
            continue;
        }
        if let Event::FrameTreeCancellationRequested {
            target,
            receipt,
            command,
        } = *record.event()
        {
            let bytes = context
                .cas
                .get_bounded(&receipt, MAX_CANCELLATION_RECEIPT_BYTES)?;
            if content_id(&bytes) != receipt {
                return Err(FrameCancellationError::Integrity(
                    "cancel intent digest mismatch",
                ));
            }
            let saved: CancellationReceipt =
                serde_json::from_slice(&bytes).map_err(|_| FrameCancellationError::Encoding)?;
            if saved.schema != 1
                || saved.session != session
                || saved.root != root
                || saved.actor != root
                || saved.target != target
                || saved.command != command
                || saved.frames.len() > MAX_SESSION_FRAMES
            {
                return Err(FrameCancellationError::Integrity(
                    "cancel intent identity mismatch",
                ));
            }
            let approval = load_cancellation_evidence(
                &evidence,
                saved.approval,
                session,
                root,
                target,
                command,
            )?;
            if approval.token_digest != saved.token_digest {
                return Err(FrameCancellationError::Integrity(
                    "cancel intent approval mismatch",
                ));
            }
            if saved.frames.iter().any(|member| member.frame == frame) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
