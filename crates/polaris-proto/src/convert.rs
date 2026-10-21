//! Conversions between the sdk vocabulary and the generated protobuf messages.
//!
//! sdk -> pb is infallible (`From<&T>`); pb -> sdk is fallible (`TryFrom`) because the wire is
//! untrusted: required fields can be missing, enum numbers unknown, digests the wrong length and
//! strings invalid. No conversion panics. Every `match` over a protobuf `oneof` or an sdk enum is
//! exhaustive (no wildcard arm), so a new variant fails to compile until it is converted.
//!
//! Blobs never appear here: the sdk already holds them as `Cid` references.

use polaris_sdk::contracts::{
    Command, CommandBody, CommandId, EVENT_SCHEMA_VERSION, Event, EventRecord, Timestamp,
    TokenBytes, Unverified, VerificationState, WorkerKind,
};
use polaris_sdk::effort::Effort;
use polaris_sdk::error::ParseError;
use polaris_sdk::ids::{AgentRole, Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
    QuestionId, QuestionKind,
};

use crate::error::ProtoError;
use crate::pb;

/// Required length of a [`pb::Cid`] digest in bytes.
const CID_LEN: usize = 32;
const NANOS_PER_SECOND: i32 = 1_000_000_000;

fn require<T>(field: &'static str, value: Option<T>) -> Result<T, ProtoError> {
    value.ok_or(ProtoError::MissingField(field))
}

// ---------------------------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------------------------

impl From<&Cid> for pb::Cid {
    fn from(cid: &Cid) -> Self {
        Self {
            digest: cid.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<pb::Cid> for Cid {
    type Error = ProtoError;

    fn try_from(wire: pb::Cid) -> Result<Self, Self::Error> {
        let actual = wire.digest.len();
        <[u8; CID_LEN]>::try_from(wire.digest.as_slice())
            .map(Cid::from_bytes)
            .map_err(|_| ProtoError::InvalidLength {
                field: "cid.digest",
                expected: CID_LEN,
                actual,
            })
    }
}

macro_rules! uuid_conversions {
    ($($id:ty),+ $(,)?) => {$(
        impl From<&$id> for pb::Uuid {
            fn from(id: &$id) -> Self {
                let (hi, lo) = id.as_uuid().as_u64_pair();
                Self { hi, lo }
            }
        }

        impl From<pb::Uuid> for $id {
            fn from(wire: pb::Uuid) -> Self {
                <$id>::from_u128((u128::from(wire.hi) << 64) | u128::from(wire.lo))
            }
        }
    )+};
}

uuid_conversions!(SessionId, FrameId, CommandId);

fn cid_to_pb(cid: Cid) -> Option<pb::Cid> {
    Some(pb::Cid::from(&cid))
}

fn frame_to_pb(frame: FrameId) -> Option<pb::Uuid> {
    Some(pb::Uuid::from(&frame))
}

fn cid_from_pb(field: &'static str, wire: Option<pb::Cid>) -> Result<Cid, ProtoError> {
    Cid::try_from(require(field, wire)?)
}

fn frame_from_pb(field: &'static str, wire: Option<pb::Uuid>) -> Result<FrameId, ProtoError> {
    Ok(FrameId::from(require(field, wire)?))
}

// ---------------------------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------------------------

fn role_to_pb(role: AgentRole) -> i32 {
    i32::from(match role {
        AgentRole::Orchestrator => pb::AgentRole::Orchestrator,
        AgentRole::SubAgent => pb::AgentRole::SubAgent,
        AgentRole::Verifier => pb::AgentRole::Verifier,
        AgentRole::Researcher => pb::AgentRole::Researcher,
        AgentRole::Hacker => pb::AgentRole::Hacker,
        AgentRole::Watchdog => pb::AgentRole::Watchdog,
    })
}

fn role_from_pb(field: &'static str, value: i32) -> Result<AgentRole, ProtoError> {
    match pb::AgentRole::try_from(value) {
        Ok(pb::AgentRole::Orchestrator) => Ok(AgentRole::Orchestrator),
        Ok(pb::AgentRole::SubAgent) => Ok(AgentRole::SubAgent),
        Ok(pb::AgentRole::Verifier) => Ok(AgentRole::Verifier),
        Ok(pb::AgentRole::Researcher) => Ok(AgentRole::Researcher),
        Ok(pb::AgentRole::Hacker) => Ok(AgentRole::Hacker),
        Ok(pb::AgentRole::Watchdog) => Ok(AgentRole::Watchdog),
        Ok(pb::AgentRole::Unspecified) | Err(_) => Err(ProtoError::UnknownVariant { field, value }),
    }
}

fn effort_to_pb(effort: Effort) -> i32 {
    i32::from(match effort {
        Effort::Low => pb::Effort::Low,
        Effort::Medium => pb::Effort::Medium,
        Effort::High => pb::Effort::High,
        Effort::XHigh => pb::Effort::Xhigh,
        Effort::Max => pb::Effort::Max,
    })
}

fn effort_from_pb(field: &'static str, value: i32) -> Result<Effort, ProtoError> {
    match pb::Effort::try_from(value) {
        Ok(pb::Effort::Low) => Ok(Effort::Low),
        Ok(pb::Effort::Medium) => Ok(Effort::Medium),
        Ok(pb::Effort::High) => Ok(Effort::High),
        Ok(pb::Effort::Xhigh) => Ok(Effort::XHigh),
        Ok(pb::Effort::Max) => Ok(Effort::Max),
        Ok(pb::Effort::Unspecified) | Err(_) => Err(ProtoError::UnknownVariant { field, value }),
    }
}

fn worker_to_pb(worker: WorkerKind) -> i32 {
    i32::from(match worker {
        WorkerKind::S1 => pb::WorkerKind::S1,
        WorkerKind::Ingest => pb::WorkerKind::Ingest,
        WorkerKind::Nano => pb::WorkerKind::Nano,
    })
}

fn worker_from_pb(field: &'static str, value: i32) -> Result<WorkerKind, ProtoError> {
    match pb::WorkerKind::try_from(value) {
        Ok(pb::WorkerKind::S1) => Ok(WorkerKind::S1),
        Ok(pb::WorkerKind::Ingest) => Ok(WorkerKind::Ingest),
        Ok(pb::WorkerKind::Nano) => Ok(WorkerKind::Nano),
        Ok(pb::WorkerKind::Unspecified) | Err(_) => {
            Err(ProtoError::UnknownVariant { field, value })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Timestamp
// ---------------------------------------------------------------------------------------------

/// protobuf `Timestamp` convention: `nanos` is always in `0..1_000_000_000` and counts forward from
/// `seconds` (so instants before the epoch use a floor second). jiff reports a negative sub-second
/// part for such instants, which is shifted here.
fn timestamp_to_pb(at: Timestamp) -> prost_types::Timestamp {
    let seconds = at.as_second();
    let nanos = at.subsec_nanosecond();
    if nanos < 0 {
        // Cannot saturate: jiff timestamps stay far from the i64 / i32 limits.
        prost_types::Timestamp {
            seconds: seconds.saturating_sub(1),
            nanos: nanos.saturating_add(NANOS_PER_SECOND),
        }
    } else {
        prost_types::Timestamp { seconds, nanos }
    }
}

fn timestamp_from_pb(wire: prost_types::Timestamp) -> Result<Timestamp, ProtoError> {
    if !(0..NANOS_PER_SECOND).contains(&wire.nanos) {
        return Err(ProtoError::OutOfRange {
            field: "event_record.at.nanos",
        });
    }
    Timestamp::new(wire.seconds, wire.nanos).map_err(|_| ProtoError::OutOfRange {
        field: "event_record.at",
    })
}

// ---------------------------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------------------------

impl From<&Event> for pb::Event {
    fn from(event: &Event) -> Self {
        use pb::event::{self as m, Kind};

        let kind = match *event {
            Event::SessionStarted { workspace } => Kind::SessionStarted(m::SessionStarted {
                workspace: cid_to_pb(workspace),
            }),
            Event::DirectiveApplied { directive } => Kind::DirectiveApplied(m::DirectiveApplied {
                directive: cid_to_pb(directive),
            }),
            Event::DecisionMade {
                frame,
                decision,
                latency_us,
            } => Kind::DecisionMade(m::DecisionMade {
                frame: frame_to_pb(frame),
                decision: cid_to_pb(decision),
                latency_us,
            }),
            Event::ModelRequestCompleted {
                frame,
                request,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                cost,
                latency_us,
            } => Kind::ModelRequestCompleted(m::ModelRequestCompleted {
                frame: frame_to_pb(frame),
                request: cid_to_pb(request),
                prompt_tokens: prompt_tokens.get(),
                completion_tokens: completion_tokens.get(),
                cached_tokens: cached_tokens.get(),
                cost_micro_usd: cost.get(),
                latency_us,
            }),
            Event::ToolExecutionStarted { frame, call } => {
                Kind::ToolExecutionStarted(m::ToolExecutionStarted {
                    frame: frame_to_pb(frame),
                    call: cid_to_pb(call),
                })
            }
            Event::ToolExecutionCompleted {
                frame,
                call,
                ok,
                latency_us,
            } => Kind::ToolExecutionCompleted(m::ToolExecutionCompleted {
                frame: frame_to_pb(frame),
                call: cid_to_pb(call),
                ok,
                latency_us,
            }),
            Event::DiffApproved { frame, diff } => Kind::DiffApproved(m::DiffApproved {
                frame: frame_to_pb(frame),
                diff: cid_to_pb(diff),
            }),
            Event::DiffRejected {
                frame,
                diff,
                reason,
            } => Kind::DiffRejected(m::DiffRejected {
                frame: frame_to_pb(frame),
                diff: cid_to_pb(diff),
                reason: cid_to_pb(reason),
            }),
            Event::FileLeaseGranted { frame, path, lease } => {
                Kind::FileLeaseGranted(m::FileLeaseGranted {
                    frame: frame_to_pb(frame),
                    path: cid_to_pb(path),
                    lease,
                })
            }
            Event::FileLeaseRevoked { frame, lease } => {
                Kind::FileLeaseRevoked(m::FileLeaseRevoked {
                    frame: frame_to_pb(frame),
                    lease,
                })
            }
            Event::SandboxSnapshotBranchCreated {
                frame,
                parent,
                branch,
            } => Kind::SandboxSnapshotBranchCreated(m::SandboxSnapshotBranchCreated {
                frame: frame_to_pb(frame),
                parent: cid_to_pb(parent),
                branch: cid_to_pb(branch),
            }),
            Event::SandboxExecutionCompleted {
                frame,
                exec,
                exit_code,
                latency_us,
            } => Kind::SandboxExecutionCompleted(m::SandboxExecutionCompleted {
                frame: frame_to_pb(frame),
                exec: cid_to_pb(exec),
                exit_code,
                latency_us,
            }),
            Event::ProofOfDonePassed { frame, report } => {
                Kind::ProofOfDonePassed(m::ProofOfDonePassed {
                    frame: frame_to_pb(frame),
                    report: cid_to_pb(report),
                })
            }
            Event::ProofOfDoneBlocked { frame, report } => {
                Kind::ProofOfDoneBlocked(m::ProofOfDoneBlocked {
                    frame: frame_to_pb(frame),
                    report: cid_to_pb(report),
                })
            }
            Event::WorkerRestarted { worker, restarts } => {
                Kind::WorkerRestarted(m::WorkerRestarted {
                    worker: worker_to_pb(worker),
                    restarts,
                })
            }
            Event::CircuitTripped { breaker } => Kind::CircuitTripped(m::CircuitTripped {
                breaker: cid_to_pb(breaker),
            }),
            Event::DeadLettered { message } => Kind::DeadLettered(m::DeadLettered {
                message: cid_to_pb(message),
            }),
            Event::SessionClosed => Kind::SessionClosed(m::SessionClosed {}),
            Event::SessionConfigured { configuration } => {
                Kind::SessionConfigured(m::SessionConfigured {
                    configuration: cid_to_pb(configuration),
                })
            }
            Event::SessionPaused => Kind::SessionPaused(m::SessionPaused {}),
            Event::SessionResumed => Kind::SessionResumed(m::SessionResumed {}),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<pb::Event> for Event {
    type Error = ProtoError;

    fn try_from(wire: pb::Event) -> Result<Self, Self::Error> {
        use pb::event::Kind;

        let event = match require("event.kind", wire.kind)? {
            Kind::SessionStarted(m) => Self::SessionStarted {
                workspace: cid_from_pb("event.session_started.workspace", m.workspace)?,
            },
            Kind::DirectiveApplied(m) => Self::DirectiveApplied {
                directive: cid_from_pb("event.directive_applied.directive", m.directive)?,
            },
            Kind::DecisionMade(m) => Self::DecisionMade {
                frame: frame_from_pb("event.decision_made.frame", m.frame)?,
                decision: cid_from_pb("event.decision_made.decision", m.decision)?,
                latency_us: m.latency_us,
            },
            Kind::ModelRequestCompleted(m) => Self::ModelRequestCompleted {
                frame: frame_from_pb("event.model_request_completed.frame", m.frame)?,
                request: cid_from_pb("event.model_request_completed.request", m.request)?,
                prompt_tokens: TokenCount::new(m.prompt_tokens),
                completion_tokens: TokenCount::new(m.completion_tokens),
                cached_tokens: TokenCount::new(m.cached_tokens),
                cost: MicroUsd::new(m.cost_micro_usd),
                latency_us: m.latency_us,
            },
            Kind::ToolExecutionStarted(m) => Self::ToolExecutionStarted {
                frame: frame_from_pb("event.tool_execution_started.frame", m.frame)?,
                call: cid_from_pb("event.tool_execution_started.call", m.call)?,
            },
            Kind::ToolExecutionCompleted(m) => Self::ToolExecutionCompleted {
                frame: frame_from_pb("event.tool_execution_completed.frame", m.frame)?,
                call: cid_from_pb("event.tool_execution_completed.call", m.call)?,
                ok: m.ok,
                latency_us: m.latency_us,
            },
            Kind::DiffApproved(m) => Self::DiffApproved {
                frame: frame_from_pb("event.diff_approved.frame", m.frame)?,
                diff: cid_from_pb("event.diff_approved.diff", m.diff)?,
            },
            Kind::DiffRejected(m) => Self::DiffRejected {
                frame: frame_from_pb("event.diff_rejected.frame", m.frame)?,
                diff: cid_from_pb("event.diff_rejected.diff", m.diff)?,
                reason: cid_from_pb("event.diff_rejected.reason", m.reason)?,
            },
            Kind::FileLeaseGranted(m) => Self::FileLeaseGranted {
                frame: frame_from_pb("event.file_lease_granted.frame", m.frame)?,
                path: cid_from_pb("event.file_lease_granted.path", m.path)?,
                lease: m.lease,
            },
            Kind::FileLeaseRevoked(m) => Self::FileLeaseRevoked {
                frame: frame_from_pb("event.file_lease_revoked.frame", m.frame)?,
                lease: m.lease,
            },
            Kind::SandboxSnapshotBranchCreated(m) => Self::SandboxSnapshotBranchCreated {
                frame: frame_from_pb("event.sandbox_snapshot_branch_created.frame", m.frame)?,
                parent: cid_from_pb("event.sandbox_snapshot_branch_created.parent", m.parent)?,
                branch: cid_from_pb("event.sandbox_snapshot_branch_created.branch", m.branch)?,
            },
            Kind::SandboxExecutionCompleted(m) => Self::SandboxExecutionCompleted {
                frame: frame_from_pb("event.sandbox_execution_completed.frame", m.frame)?,
                exec: cid_from_pb("event.sandbox_execution_completed.exec", m.exec)?,
                exit_code: m.exit_code,
                latency_us: m.latency_us,
            },
            Kind::ProofOfDonePassed(m) => Self::ProofOfDonePassed {
                frame: frame_from_pb("event.proof_of_done_passed.frame", m.frame)?,
                report: cid_from_pb("event.proof_of_done_passed.report", m.report)?,
            },
            Kind::ProofOfDoneBlocked(m) => Self::ProofOfDoneBlocked {
                frame: frame_from_pb("event.proof_of_done_blocked.frame", m.frame)?,
                report: cid_from_pb("event.proof_of_done_blocked.report", m.report)?,
            },
            Kind::WorkerRestarted(m) => Self::WorkerRestarted {
                worker: worker_from_pb("event.worker_restarted.worker", m.worker)?,
                restarts: m.restarts,
            },
            Kind::CircuitTripped(m) => Self::CircuitTripped {
                breaker: cid_from_pb("event.circuit_tripped.breaker", m.breaker)?,
            },
            Kind::DeadLettered(m) => Self::DeadLettered {
                message: cid_from_pb("event.dead_lettered.message", m.message)?,
            },
            Kind::SessionClosed(_) => Self::SessionClosed,
            Kind::SessionConfigured(m) => Self::SessionConfigured {
                configuration: cid_from_pb(
                    "event.session_configured.configuration",
                    m.configuration,
                )?,
            },
            Kind::SessionPaused(_) => Self::SessionPaused,
            Kind::SessionResumed(_) => Self::SessionResumed,
        };
        Ok(event)
    }
}

impl From<&EventRecord> for pb::EventRecord {
    fn from(record: &EventRecord) -> Self {
        Self {
            seq: record.seq().get(),
            at: Some(timestamp_to_pb(record.at())),
            schema: u32::from(record.schema()),
            session: Some(pb::Uuid::from(&record.session())),
            event: Some(pb::Event::from(record.event())),
        }
    }
}

impl TryFrom<pb::EventRecord> for EventRecord {
    type Error = ProtoError;

    fn try_from(wire: pb::EventRecord) -> Result<Self, Self::Error> {
        let schema = u16::try_from(wire.schema).map_err(|_| ProtoError::OutOfRange {
            field: "event_record.schema",
        })?;
        if schema == 0 || schema > EVENT_SCHEMA_VERSION {
            return Err(ParseError::UnsupportedSchema(schema).into());
        }
        let at = timestamp_from_pb(require("event_record.at", wire.at)?)?;
        let session = SessionId::from(require("event_record.session", wire.session)?);
        let event = Event::try_from(require("event_record.event", wire.event)?)?;
        Ok(Self::from_versioned(
            schema,
            Seq::new(wire.seq),
            at,
            session,
            event,
        )?)
    }
}

// ---------------------------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------------------------

impl From<&CommandBody> for pb::CommandBody {
    fn from(body: &CommandBody) -> Self {
        use pb::command_body::{self as m, Kind};

        let kind = match *body {
            CommandBody::SubmitUserInput { text } => Kind::SubmitUserInput(m::SubmitUserInput {
                text: cid_to_pb(text),
            }),
            CommandBody::ProposeDiff { diff } => Kind::ProposeDiff(m::ProposeDiff {
                diff: cid_to_pb(diff),
            }),
            CommandBody::AcquireAstScope { path, symbol } => {
                Kind::AcquireAstScope(m::AcquireAstScope {
                    path: cid_to_pb(path),
                    symbol: cid_to_pb(symbol),
                })
            }
            CommandBody::ReleaseAstScope { lease } => {
                Kind::ReleaseAstScope(m::ReleaseAstScope { lease })
            }
            CommandBody::SpawnSubagent {
                role,
                effort,
                brief,
            } => Kind::SpawnSubagent(m::SpawnSubagent {
                role: role_to_pb(role),
                effort: effort_to_pb(effort),
                brief: cid_to_pb(brief),
            }),
            CommandBody::ExecuteTool { call } => Kind::ExecuteTool(m::ExecuteTool {
                call: cid_to_pb(call),
            }),
            CommandBody::RollbackTo { seq } => Kind::RollbackTo(m::RollbackTo { seq: seq.get() }),
            CommandBody::CancelFrame { frame } => Kind::CancelFrame(m::CancelFrame {
                frame: frame_to_pb(frame),
            }),
            CommandBody::ApplyDirective { directive } => Kind::ApplyDirective(m::ApplyDirective {
                directive: cid_to_pb(directive),
            }),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<pb::CommandBody> for CommandBody {
    type Error = ProtoError;

    fn try_from(wire: pb::CommandBody) -> Result<Self, Self::Error> {
        use pb::command_body::Kind;

        let body = match require("command_body.kind", wire.kind)? {
            Kind::SubmitUserInput(m) => Self::SubmitUserInput {
                text: cid_from_pb("command_body.submit_user_input.text", m.text)?,
            },
            Kind::ProposeDiff(m) => Self::ProposeDiff {
                diff: cid_from_pb("command_body.propose_diff.diff", m.diff)?,
            },
            Kind::AcquireAstScope(m) => Self::AcquireAstScope {
                path: cid_from_pb("command_body.acquire_ast_scope.path", m.path)?,
                symbol: cid_from_pb("command_body.acquire_ast_scope.symbol", m.symbol)?,
            },
            Kind::ReleaseAstScope(m) => Self::ReleaseAstScope { lease: m.lease },
            Kind::SpawnSubagent(m) => Self::SpawnSubagent {
                role: role_from_pb("command_body.spawn_subagent.role", m.role)?,
                effort: effort_from_pb("command_body.spawn_subagent.effort", m.effort)?,
                brief: cid_from_pb("command_body.spawn_subagent.brief", m.brief)?,
            },
            Kind::ExecuteTool(m) => Self::ExecuteTool {
                call: cid_from_pb("command_body.execute_tool.call", m.call)?,
            },
            Kind::RollbackTo(m) => Self::RollbackTo {
                seq: Seq::new(m.seq),
            },
            Kind::CancelFrame(m) => Self::CancelFrame {
                frame: frame_from_pb("command_body.cancel_frame.frame", m.frame)?,
            },
            Kind::ApplyDirective(m) => Self::ApplyDirective {
                directive: cid_from_pb("command_body.apply_directive.directive", m.directive)?,
            },
        };
        Ok(body)
    }
}

/// A command in any verification state serialises the same way: the approval witness of a
/// `Command<Verified>` is internal to the kernel and never crosses the wire.
impl<S: VerificationState> From<&Command<S>> for pb::Command {
    fn from(command: &Command<S>) -> Self {
        Self {
            id: Some(pb::Uuid::from(&command.id())),
            frame: frame_to_pb(command.frame()),
            token: command.token().as_bytes().to_vec(),
            body: Some(pb::CommandBody::from(command.body())),
        }
    }
}

/// Commands always arrive unverified; the kernel verifier promotes them.
impl TryFrom<pb::Command> for Command<Unverified> {
    type Error = ProtoError;

    fn try_from(wire: pb::Command) -> Result<Self, Self::Error> {
        let id = CommandId::from(require("command.id", wire.id)?);
        let frame = frame_from_pb("command.frame", wire.frame)?;
        let body = CommandBody::try_from(require("command.body", wire.body)?)?;
        Ok(Self::new(id, frame, TokenBytes::from(wire.token), body))
    }
}

// ---------------------------------------------------------------------------------------------
// System One: Decision, Question, DecisionInput
// ---------------------------------------------------------------------------------------------

fn probability_to_f32(probability: Probability) -> f32 {
    probability.get()
}

impl From<&Answer> for pb::Answer {
    fn from(answer: &Answer) -> Self {
        use pb::answer::{self as m, Kind};

        let kind = match answer {
            Answer::Choice { selected, probs } => Kind::Choice(m::Choice {
                selected: selected.as_str().to_owned(),
                probs: probs
                    .iter()
                    .map(|score| pb::LabelScore {
                        label: score.label.as_str().to_owned(),
                        probability: probability_to_f32(score.probability),
                    })
                    .collect(),
            }),
            Answer::Score { value, confidence } => Kind::Score(m::Score {
                value: *value,
                confidence: probability_to_f32(*confidence),
            }),
            Answer::Noul { yes } => Kind::Noul(m::Noul {
                yes: probability_to_f32(*yes),
            }),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<pb::Answer> for Answer {
    type Error = ProtoError;

    fn try_from(wire: pb::Answer) -> Result<Self, Self::Error> {
        use pb::answer::Kind;

        let answer = match require("answer.kind", wire.kind)? {
            Kind::Choice(m) => Self::Choice {
                selected: Label::new(&m.selected)?,
                probs: m
                    .probs
                    .iter()
                    .map(|score| {
                        Ok(LabelScore {
                            label: Label::new(&score.label)?,
                            probability: Probability::new(score.probability)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ProtoError>>()?,
            },
            Kind::Score(m) => Self::Score {
                value: m.value,
                confidence: Probability::new(m.confidence)?,
            },
            Kind::Noul(m) => Self::Noul {
                yes: Probability::new(m.yes)?,
            },
        };
        Ok(answer)
    }
}

impl From<&QuestionAnswer> for pb::QuestionAnswer {
    fn from(item: &QuestionAnswer) -> Self {
        Self {
            question: item.question.as_str().to_owned(),
            answer: Some(pb::Answer::from(&item.answer)),
        }
    }
}

impl TryFrom<pb::QuestionAnswer> for QuestionAnswer {
    type Error = ProtoError;

    fn try_from(wire: pb::QuestionAnswer) -> Result<Self, Self::Error> {
        Ok(Self {
            question: QuestionId::new(&wire.question)?,
            answer: Answer::try_from(require("question_answer.answer", wire.answer)?)?,
        })
    }
}

impl From<&Decision> for pb::Decision {
    fn from(decision: &Decision) -> Self {
        Self {
            answers: decision
                .answers
                .iter()
                .map(pb::QuestionAnswer::from)
                .collect(),
            latency_us: decision.latency_us,
        }
    }
}

impl TryFrom<pb::Decision> for Decision {
    type Error = ProtoError;

    fn try_from(wire: pb::Decision) -> Result<Self, Self::Error> {
        Ok(Self {
            answers: wire
                .answers
                .into_iter()
                .map(QuestionAnswer::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            latency_us: wire.latency_us,
        })
    }
}

impl From<&Question> for pb::Question {
    fn from(question: &Question) -> Self {
        use pb::question::{self as m, Kind};

        let kind = match &question.kind {
            QuestionKind::Choice { options } => Kind::Choice(m::Choice {
                options: options.iter().map(|o| o.as_str().to_owned()).collect(),
            }),
            QuestionKind::Score { min, max } => Kind::Score(m::Score {
                min: *min,
                max: *max,
            }),
            QuestionKind::Noul { statement } => Kind::Noul(m::Noul {
                statement: statement.clone(),
            }),
        };
        Self {
            id: question.id.as_str().to_owned(),
            kind: Some(kind),
        }
    }
}

impl TryFrom<pb::Question> for Question {
    type Error = ProtoError;

    fn try_from(wire: pb::Question) -> Result<Self, Self::Error> {
        use pb::question::Kind;

        let id = QuestionId::new(&wire.id)?;
        let kind = match require("question.kind", wire.kind)? {
            Kind::Choice(m) => QuestionKind::Choice {
                options: m
                    .options
                    .iter()
                    .map(|o| Label::new(o))
                    .collect::<Result<Vec<_>, ParseError>>()?,
            },
            Kind::Score(m) => QuestionKind::Score {
                min: m.min,
                max: m.max,
            },
            Kind::Noul(m) => QuestionKind::Noul {
                statement: m.statement,
            },
        };
        Ok(Self { id, kind })
    }
}

impl From<&DecisionInput> for pb::DecisionInput {
    fn from(input: &DecisionInput) -> Self {
        Self {
            text: input.text.clone(),
            context: input.context.clone(),
        }
    }
}

impl From<pb::DecisionInput> for DecisionInput {
    fn from(wire: pb::DecisionInput) -> Self {
        Self {
            text: wire.text,
            context: wire.context,
        }
    }
}
