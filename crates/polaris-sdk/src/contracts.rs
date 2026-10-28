//! CQRS contracts: capability token bytes, the `Command<Unverified | Verified>` typestate, and the
//! versioned, fixed-size `Event` envelope.
//!
//! Events carry only ids, scalars and [`Cid`] references; text and blobs live in CAS. That keeps
//! `Event` `Copy` and small enough for the zero-allocation bus (PLAN ADR-003).

use core::fmt;
use core::marker::PhantomData;
use std::borrow::Cow;

use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use crate::ids::CommandId;
pub use jiff::Timestamp;

use crate::effort::Effort;
use crate::error::ParseError;
use crate::hex;
use crate::human::HumanRequestId;
use crate::ids::{AgentRole, Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};

/// Current [`EventRecord`] schema version.
pub const EVENT_SCHEMA_VERSION: u16 = 7;

/// Opaque capability token (a serialized Biscuit). The sdk never interprets it and never prints it:
/// `Debug` shows only the length.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenBytes(Bytes);

impl TokenBytes {
    /// Wraps a static token (fixtures).
    #[must_use]
    pub const fn from_static(bytes: &'static [u8]) -> Self {
        Self(Bytes::from_static(bytes))
    }

    /// The raw token bytes (for the capability verifier only).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Token length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` for an empty token.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for TokenBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(Bytes::from(value))
    }
}

impl From<Bytes> for TokenBytes {
    fn from(value: Bytes) -> Self {
        Self(value)
    }
}

impl fmt::Debug for TokenBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TokenBytes(<redacted, {} bytes>)", self.0.len())
    }
}

impl Serialize for TokenBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(&self.0))
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for TokenBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let text = Cow::<str>::deserialize(deserializer)?;
            hex::decode_vec(&text)
                .map(Self::from)
                .map_err(serde::de::Error::custom)
        } else {
            Bytes::deserialize(deserializer).map(Self)
        }
    }
}

/// Typestate: a command as submitted, not yet checked by the verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverified {}

/// Typestate: a command approved by the verifier; only these reach kernel handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Unverified {}
    impl Sealed for super::Verified {}
}

/// The verification state of a [`Command`] and the proof it carries. Sealed: only
/// [`Unverified`] (no proof) and [`Verified`] (an [`ApprovalWitness`]) exist.
pub trait VerificationState: sealed::Sealed {
    /// What a command in this state carries as proof of verification.
    type Proof: Clone + PartialEq + fmt::Debug + Serialize;
}

impl VerificationState for Unverified {
    type Proof = ();
}

impl VerificationState for Verified {
    type Proof = ApprovalWitness;
}

/// Name of the component that approved a command (e.g. `polaris-kernel-verifier`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApproverId(Box<str>);

impl ApproverId {
    /// Longest accepted approver id in bytes.
    pub const MAX_LEN: usize = 64;

    /// Validates an approver id: non-empty, at most [`Self::MAX_LEN`] bytes, ASCII alphanumerics
    /// and `-_.:` only.
    ///
    /// # Errors
    /// [`ParseError::InvalidValue`] when a rule is violated.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        let what = "approver id";
        if value.is_empty() {
            return Err(ParseError::InvalidValue {
                what,
                reason: "empty",
            });
        }
        if value.len() > Self::MAX_LEN {
            return Err(ParseError::InvalidValue {
                what,
                reason: "too long",
            });
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        {
            return Err(ParseError::InvalidValue {
                what,
                reason: "contains a disallowed character",
            });
        }
        Ok(Self(value.into()))
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ApproverId {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ApproverId> for String {
    fn from(value: ApproverId) -> Self {
        value.0.into_string()
    }
}

/// Evidence that a command was approved: who approved it, the verification report (in CAS) and when.
///
/// Rust cannot stop another workspace crate from calling a public constructor; the typestate makes
/// approval explicit and greppable, and `cargo xtask check-witness` fails when
/// `ApprovalWitness::new` is called outside `polaris-sdk` and `polaris-kernel-verifier`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalWitness {
    approver: ApproverId,
    evidence: Cid,
    at: Timestamp,
}

impl ApprovalWitness {
    /// Records an approval.
    #[must_use]
    pub const fn new(approver: ApproverId, evidence: Cid, at: Timestamp) -> Self {
        Self {
            approver,
            evidence,
            at,
        }
    }

    /// Who approved.
    #[must_use]
    pub const fn approver(&self) -> &ApproverId {
        &self.approver
    }

    /// The verification report in CAS.
    #[must_use]
    pub const fn evidence(&self) -> Cid {
        self.evidence
    }

    /// When the approval happened.
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.at
    }
}

/// What a command asks the kernel to do. Payload text lives in CAS and is referenced by [`Cid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandBody {
    /// User input for the frame (text in CAS).
    SubmitUserInput {
        /// The input text.
        text: Cid,
    },
    /// A proposed workspace diff (unified diff in CAS).
    ProposeDiff {
        /// The diff.
        diff: Cid,
    },
    /// Lease an AST scope (file + symbol) before editing it.
    AcquireAstScope {
        /// Workspace-relative path (in CAS).
        path: Cid,
        /// Symbol path inside the file (in CAS).
        symbol: Cid,
    },
    /// Release a previously granted lease.
    ReleaseAstScope {
        /// Lease number from `FileLeaseGranted`.
        lease: u64,
    },
    /// Start a child frame.
    SpawnSubagent {
        /// Role of the child frame.
        role: AgentRole,
        /// Effort level of the child's model.
        effort: Effort,
        /// The child's brief (in CAS).
        brief: Cid,
    },
    /// Execute a tool call (call record JSON in CAS).
    ExecuteTool {
        /// The call record.
        call: Cid,
    },
    /// Roll the session back to an event sequence number.
    RollbackTo {
        /// Target sequence number.
        seq: Seq,
    },
    /// Cancel a frame and its children.
    CancelFrame {
        /// The frame to cancel.
        frame: FrameId,
    },
    /// Apply a `$directive` (directive text in CAS).
    ApplyDirective {
        /// The directive text.
        directive: Cid,
    },
    /// Atomically answer one immutable human request using a complete answer batch in CAS.
    AnswerHumanRequest {
        /// Stable human request identity.
        request_id: HumanRequestId,
        /// Immutable request digest used as a compare-and-swap revision.
        revision: Cid,
        /// Complete answer batch in CAS.
        answer: Cid,
    },
}

/// A command in verification state `S`.
///
/// Kernel handlers take `Command<Verified>`; the only way to get one is [`Command::verify`]:
///
/// ```
/// use polaris_sdk::contracts::*;
/// use polaris_sdk::ids::{Cid, FrameId};
/// fn execute(_cmd: Command<Verified>) {}
/// let cmd = Command::new(CommandId::from_u128(1), FrameId::from_u128(2), TokenBytes::from(vec![1]),
///     CommandBody::ProposeDiff { diff: Cid::from_bytes([0; 32]) });
/// let witness = ApprovalWitness::new(ApproverId::new("polaris-kernel-verifier")?, Cid::from_bytes([1; 32]),
///     "2026-10-20T12:00:00Z".parse()?);
/// execute(cmd.verify(witness));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// An unverified command does not type-check as a verified one (`handler_requires_verified_command`):
///
/// ```compile_fail
/// use polaris_sdk::contracts::*;
/// use polaris_sdk::ids::{Cid, FrameId};
/// fn execute(_cmd: Command<Verified>) {}
/// let cmd = Command::new(CommandId::from_u128(1), FrameId::from_u128(2), TokenBytes::from(vec![1]),
///     CommandBody::ProposeDiff { diff: Cid::from_bytes([0; 32]) });
/// execute(cmd);
/// ```
///
/// Only unverified commands can be deserialized (`verified_command_cannot_be_deserialized`):
///
/// ```compile_fail
/// use polaris_sdk::contracts::{Command, Verified};
/// fn needs_deserialize<T: for<'de> serde::Deserialize<'de>>() {}
/// needs_deserialize::<Command<Verified>>();
/// ```
///
/// ```
/// use polaris_sdk::contracts::{Command, Unverified};
/// fn needs_deserialize<T: for<'de> serde::Deserialize<'de>>() {}
/// needs_deserialize::<Command<Unverified>>();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(bound = "")]
pub struct Command<S: VerificationState> {
    id: CommandId,
    frame: FrameId,
    token: TokenBytes,
    body: CommandBody,
    #[serde(rename = "witness")]
    proof: S::Proof,
    #[serde(skip)]
    state: PhantomData<S>,
}

impl<S: VerificationState> Command<S> {
    /// The command id.
    #[must_use]
    pub const fn id(&self) -> CommandId {
        self.id
    }

    /// The frame the command belongs to.
    #[must_use]
    pub const fn frame(&self) -> FrameId {
        self.frame
    }

    /// The capability token presented with the command.
    #[must_use]
    pub const fn token(&self) -> &TokenBytes {
        &self.token
    }

    /// What the command asks for.
    #[must_use]
    pub const fn body(&self) -> &CommandBody {
        &self.body
    }
}

impl Command<Unverified> {
    /// A new, unverified command.
    #[must_use]
    pub const fn new(id: CommandId, frame: FrameId, token: TokenBytes, body: CommandBody) -> Self {
        Self {
            id,
            frame,
            token,
            body,
            proof: (),
            state: PhantomData,
        }
    }

    /// Promotes the command after the verifier approved it.
    #[must_use]
    pub fn verify(self, witness: ApprovalWitness) -> Command<Verified> {
        Command {
            id: self.id,
            frame: self.frame,
            token: self.token,
            body: self.body,
            proof: witness,
            state: PhantomData,
        }
    }
}

impl Command<Verified> {
    /// The approval this command carries.
    #[must_use]
    pub const fn witness(&self) -> &ApprovalWitness {
        &self.proof
    }
}

impl Eq for Command<Unverified> {}
impl Eq for Command<Verified> {}

#[derive(Deserialize)]
struct CommandWire {
    id: CommandId,
    frame: FrameId,
    token: TokenBytes,
    body: CommandBody,
}

/// Commands arrive over the wire unverified; any `witness` field in the input is ignored.
impl<'de> Deserialize<'de> for Command<Unverified> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = CommandWire::deserialize(deserializer)?;
        Ok(Self::new(wire.id, wire.frame, wire.token, wire.body))
    }
}

/// Supervised worker processes (one OS process per mode, PLAN ADR-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    /// System-One classifier worker.
    S1,
    /// Document ingest / OCR worker.
    Ingest,
    /// Needle (tier 0) worker.
    Nano,
}

/// Final disposition of one model/tool/verifier step, persisted with its CAS transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameStepOutcome {
    /// Independent verifier accepted the step.
    Accepted,
    /// The model claimed completion; proof-of-done remains a separate gate.
    ClaimedDone,
    /// Independent verifier rejected the step.
    Rejected,
    /// Cooperative cancellation stopped the step.
    Cancelled,
    /// A backend, provider, or protocol error stopped the step.
    Failed,
}

/// A domain event (schema v7, backwards-readable v1-v6). Fixed-size and `Copy`: text and blobs
/// are [`Cid`] references. Schema 2 adds configuration/pause/resume, schema 3 adds blast alerts,
/// schema 4 adds durable frame-step records, schema 5 adds the human request lifecycle, schema 6
/// adds the frame registry/tool-result records, and schema 7 adds typed user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Immutable session configuration and directive defaults committed before startup.
    SessionConfigured {
        /// Versioned configuration document in CAS.
        configuration: Cid,
    },
    /// Session was paused, including an explicitly reopened archived session.
    SessionPaused,
    /// A paused session resumed execution.
    SessionResumed,
    /// A session started on a workspace (workspace descriptor in CAS).
    SessionStarted {
        /// Workspace descriptor.
        workspace: Cid,
    },
    /// A `$directive` took effect (text in CAS).
    DirectiveApplied {
        /// Directive text.
        directive: Cid,
    },
    /// A user message for the session, text in CAS; recorded offline, no model execution implied
    /// (schema 7).
    UserInputSubmitted {
        /// The message text.
        text: Cid,
    },
    /// System One answered a decision schema.
    DecisionMade {
        /// Frame that asked.
        frame: FrameId,
        /// The decision record.
        decision: Cid,
        /// Engine latency in microseconds.
        latency_us: u64,
    },
    /// A model request finished.
    ModelRequestCompleted {
        /// Requesting frame.
        frame: FrameId,
        /// Request/response record.
        request: Cid,
        /// Prompt tokens billed.
        prompt_tokens: TokenCount,
        /// Completion tokens billed.
        completion_tokens: TokenCount,
        /// Prompt tokens served from the provider cache.
        cached_tokens: TokenCount,
        /// Cost.
        cost: MicroUsd,
        /// End-to-end latency in microseconds.
        latency_us: u64,
    },
    /// A tool call started.
    ToolExecutionStarted {
        /// Calling frame.
        frame: FrameId,
        /// Call record.
        call: Cid,
    },
    /// A tool call finished.
    ToolExecutionCompleted {
        /// Calling frame.
        frame: FrameId,
        /// Call record (with output).
        call: Cid,
        /// `true` when the tool succeeded.
        ok: bool,
        /// Latency in microseconds.
        latency_us: u64,
    },
    /// The verifier approved a diff.
    DiffApproved {
        /// Proposing frame.
        frame: FrameId,
        /// The diff.
        diff: Cid,
    },
    /// The verifier rejected a diff.
    DiffRejected {
        /// Proposing frame.
        frame: FrameId,
        /// The diff.
        diff: Cid,
        /// Rejection report.
        reason: Cid,
    },
    /// The verifier could not establish safety without a human decision.
    DiffNeedsApproval {
        /// Proposing frame.
        frame: FrameId,
        /// The diff.
        diff: Cid,
        /// Durable verifier report describing why review is required.
        report: Cid,
    },
    /// A file/AST lease was granted.
    FileLeaseGranted {
        /// Holder frame.
        frame: FrameId,
        /// Leased path.
        path: Cid,
        /// Lease number.
        lease: u64,
    },
    /// A lease was revoked or released.
    FileLeaseRevoked {
        /// Former holder.
        frame: FrameId,
        /// Lease number.
        lease: u64,
    },
    /// A sandbox snapshot branch was created (Contree `directory_state_id` branching).
    SandboxSnapshotBranchCreated {
        /// Owning frame.
        frame: FrameId,
        /// Parent snapshot.
        parent: Cid,
        /// New branch snapshot.
        branch: Cid,
    },
    /// A sandboxed command finished.
    SandboxExecutionCompleted {
        /// Owning frame.
        frame: FrameId,
        /// Execution record.
        exec: Cid,
        /// Process exit code.
        exit_code: i32,
        /// Latency in microseconds.
        latency_us: u64,
    },
    /// Proof-of-Done checks passed.
    ProofOfDonePassed {
        /// Claiming frame.
        frame: FrameId,
        /// Check report.
        report: Cid,
    },
    /// Proof-of-Done blocked a completion claim.
    ProofOfDoneBlocked {
        /// Claiming frame.
        frame: FrameId,
        /// Check report.
        report: Cid,
    },
    /// A supervised worker process was restarted.
    WorkerRestarted {
        /// Which worker.
        worker: WorkerKind,
        /// Restarts within the current intensity window.
        restarts: u32,
    },
    /// A circuit breaker opened (breaker name in CAS).
    CircuitTripped {
        /// Breaker name.
        breaker: Cid,
    },
    /// A message went to the dead-letter queue.
    DeadLettered {
        /// The message.
        message: Cid,
    },
    /// The session closed.
    SessionClosed,
    /// A conservative blast review awaits explicit operator approval (schema 3).
    BlastRadiusAlert {
        /// Frame requesting execution.
        frame: FrameId,
        /// Exact immutable review artifact, including blast facts, commands and policy.
        report: Cid,
    },
    /// A complete or interrupted frame-step transcript was stored in CAS before this event.
    FrameStepRecorded {
        /// Owning frame.
        frame: FrameId,
        /// One-based frame step.
        step: u32,
        /// Durable transcript with prompt, prior messages, result, and verified references.
        transcript: Cid,
        /// Final disposition of the attempt.
        outcome: FrameStepOutcome,
    },
    /// A validated immutable human request was durably created.
    HumanRequestCreated {
        /// Stable human request identity.
        request_id: HumanRequestId,
        /// Requesting frame.
        frame: FrameId,
        /// Request origin step/turn.
        origin_step: u32,
        /// Complete request payload; its digest is also the answer compare-and-swap revision.
        request: Cid,
    },
    /// A complete answer batch was durably committed.
    HumanRequestAnswered {
        /// Stable human request identity.
        request_id: HumanRequestId,
        /// Requesting frame.
        frame: FrameId,
        /// CAS audit receipt binding revision, answer, command, actor and approval evidence.
        receipt: Cid,
    },
    /// A pending human request was interrupted by its requesting frame.
    HumanRequestInterrupted {
        /// Stable human request identity.
        request_id: HumanRequestId,
        /// Requesting frame.
        frame: FrameId,
        /// Request revision interrupted.
        revision: Cid,
    },
    /// Exact result evidence persisted by the executing frame tool owner.
    FrameToolResultRecorded {
        /// Actual requesting frame.
        frame: FrameId,
        /// Exact stored model tool call.
        call: Cid,
        /// Call-bound CAS result envelope.
        result: Cid,
    },
    /// A frame's actual sealed runtime identity and direct parent were durably registered.
    FrameRegistered {
        /// Registered frame identity.
        frame: FrameId,
        /// Actual direct parent; `None` is reserved for the session root.
        parent: Option<FrameId>,
        /// Root identity supplied by the runtime frame constructor.
        root: FrameId,
        /// Trusted frame role captured by the sealed binding.
        role: AgentRole,
        /// Digest of frame, parent, root, role and bound capability token.
        binding: Cid,
    },
    /// A verified exact-target subtree cancellation intent was durably committed.
    FrameTreeCancellationRequested {
        /// Exact target frame; its registered descendants are included by the receipt.
        target: FrameId,
        /// CAS receipt with the command, approval evidence and exact descendant snapshot.
        receipt: Cid,
        /// Idempotency identity of the verified command.
        command: CommandId,
    },
}

/// Versioned envelope appended to the event store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EventRecordWire")]
pub struct EventRecord {
    seq: Seq,
    at: Timestamp,
    schema: u16,
    session: SessionId,
    event: Event,
}

impl EventRecord {
    /// Construct an explicitly versioned envelope without silently upgrading older records.
    ///
    /// # Errors
    /// Rejects zero/future schemas and lifecycle events mislabeled below their minimum schema.
    pub fn from_versioned(
        schema: u16,
        seq: Seq,
        at: Timestamp,
        session: SessionId,
        event: Event,
    ) -> Result<Self, ParseError> {
        let minimum = match event {
            Event::BlastRadiusAlert { .. } => 3,
            Event::DiffNeedsApproval { .. } | Event::FrameStepRecorded { .. } => 4,
            Event::HumanRequestCreated { .. }
            | Event::HumanRequestAnswered { .. }
            | Event::HumanRequestInterrupted { .. } => 5,
            Event::FrameRegistered { .. }
            | Event::FrameTreeCancellationRequested { .. }
            | Event::FrameToolResultRecorded { .. } => 6,
            Event::UserInputSubmitted { .. } => 7,
            Event::SessionConfigured { .. } | Event::SessionPaused | Event::SessionResumed => 2,
            Event::SessionStarted { .. }
            | Event::DirectiveApplied { .. }
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
            | Event::SessionClosed => 1,
        };
        if schema < minimum || schema > EVENT_SCHEMA_VERSION {
            return Err(ParseError::UnsupportedSchema(schema));
        }
        Ok(Self {
            seq,
            at,
            schema,
            session,
            event,
        })
    }
    /// A record at the current schema version.
    #[must_use]
    pub const fn new(seq: Seq, at: Timestamp, session: SessionId, event: Event) -> Self {
        Self {
            seq,
            at,
            schema: EVENT_SCHEMA_VERSION,
            session,
            event,
        }
    }

    /// Sequence number in the log.
    #[must_use]
    pub const fn seq(&self) -> Seq {
        self.seq
    }

    /// When the event happened.
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.at
    }

    /// Schema version of the record.
    #[must_use]
    pub const fn schema(&self) -> u16 {
        self.schema
    }

    /// Owning session.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// The event.
    #[must_use]
    pub const fn event(&self) -> &Event {
        &self.event
    }

    /// The same record at another sequence number (the store assigns sequence numbers).
    #[must_use]
    pub const fn with_seq(self, seq: Seq) -> Self {
        Self { seq, ..self }
    }
}

#[derive(Deserialize)]
struct EventRecordWire {
    seq: Seq,
    at: Timestamp,
    schema: u16,
    session: SessionId,
    event: Event,
}

impl TryFrom<EventRecordWire> for EventRecord {
    type Error = ParseError;

    fn try_from(wire: EventRecordWire) -> Result<Self, Self::Error> {
        Self::from_versioned(wire.schema, wire.seq, wire.at, wire.session, wire.event)
    }
}
