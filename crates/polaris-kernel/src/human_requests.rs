//! Durable P3 human request lifecycle over the real kernel CAS and event journal.
//!
//! `HumanRequestService::new` belongs to trusted kernel composition code. Ask admission requires
//! the runtime's sealed frame binding, that frame's actual Biscuit token and its trusted issuer.
//! Answer admission accepts only a command promoted by the separate human-request verifier.

use crate::{KernelCtx, KernelError};
use polaris_runtime::frames::FrameToolBinding;
use polaris_sdk::{
    contracts::{Command, CommandBody, Event, EventRecord, Verified},
    human::{
        HumanAnswerPayload, HumanContractError, HumanQuestion, HumanRequestId, HumanRequestPayload,
    },
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
};
use polaris_security::capability::{
    Authorization, Facts, PublicKeyBytes, ResourcePath, Right, SystemClock, Token, authorize,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const MAX_RECEIPT_BYTES: u64 = 2048;
const MAX_SESSION_REQUESTS: usize = 32;
const MAX_REPLAY_RECORDS: u64 = u64::MAX;

/// A durable, immutable request reference returned to the requesting frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanRequestTicket {
    /// Stable request identity.
    pub request_id: HumanRequestId,
    /// The immutable request digest, used as the answer compare-and-swap revision.
    pub revision: Cid,
    /// The CAS payload containing the full request document.
    pub request: Cid,
    /// The requesting frame.
    pub frame: FrameId,
}

/// Durable terminal or pending outcome for a human request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanRequestOutcome {
    /// The request is durably pending.
    Pending,
    /// A complete answer batch was durably committed.
    Answered {
        /// CAS digest of the committed answer document.
        answer: Cid,
        /// Event-log position of the answer transition.
        seq: Seq,
    },
    /// The requesting frame interrupted the request.
    Interrupted {
        /// Event-log position of the interruption transition.
        seq: Seq,
    },
}

/// Restored request information available to a trusted question-list projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanRequestSnapshot {
    /// Immutable request ticket.
    pub ticket: HumanRequestTicket,
    /// The validated request document.
    pub request: HumanRequestPayload,
    /// Current durable outcome.
    pub outcome: HumanRequestOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Pending {
        ticket: HumanRequestTicket,
    },
    Answered {
        ticket: HumanRequestTicket,
        answer: Cid,
        seq: Seq,
    },
    Interrupted {
        ticket: HumanRequestTicket,
        seq: Seq,
    },
}
impl RequestState {
    fn ticket(self) -> HumanRequestTicket {
        match self {
            Self::Pending { ticket }
            | Self::Answered { ticket, .. }
            | Self::Interrupted { ticket, .. } => ticket,
        }
    }
    fn outcome(self) -> HumanRequestOutcome {
        match self {
            Self::Pending { .. } => HumanRequestOutcome::Pending,
            Self::Answered { answer, seq, .. } => HumanRequestOutcome::Answered { answer, seq },
            Self::Interrupted { seq, .. } => HumanRequestOutcome::Interrupted { seq },
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAnswerReceipt {
    schema: u16,
    session: SessionId,
    request_id: HumanRequestId,
    frame: FrameId,
    command_frame: FrameId,
    command: CommandId,
    revision: Cid,
    answer: Cid,
    approval: Cid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnswerReceipt {
    request_id: HumanRequestId,
    command_frame: FrameId,
    revision: Cid,
    answer: Cid,
    seq: Seq,
}

#[derive(Debug)]
struct ServiceState {
    requests: BTreeMap<HumanRequestId, RequestState>,
    commands: BTreeMap<CommandId, AnswerReceipt>,
}

type RequestNotifications = Arc<Mutex<BTreeMap<HumanRequestId, Arc<Notify>>>>;
// Owned by the context; neither field keeps a context back-reference.
#[derive(Debug)]
pub(crate) struct HumanRegistryCache {
    root: FrameId,
    state: Arc<Mutex<ServiceState>>,
    notifications: RequestNotifications,
}

/// Durable human request service attached to one actual kernel session and root frame.
///
/// Construction replays the session's real event log and validates every request/answer CAS
/// object before exposing pending requests. It performs blocking disk reads; call from trusted
/// composition startup, not the frame executor.
#[derive(Debug)]
pub struct HumanRequestService {
    context: Arc<KernelCtx>,
    session: SessionId,
    root_frame: FrameId,
    state: Arc<Mutex<ServiceState>>,
    notifications: RequestNotifications,
}

impl HumanRequestService {
    /// Share one session's durable owner; first construction replays the actual kernel stores.
    ///
    /// `root_frame` is supplied by the trusted `KernelHandle` composition root. This constructor
    /// does not grant an answer or frame capability.
    ///
    /// # Errors
    /// Fails on closed storage, corrupt/inconsistent history, missing or malformed CAS payloads,
    /// or a duplicate pending request that violates the persisted invariants.
    pub fn new(
        context: Arc<KernelCtx>,
        session: SessionId,
        root_frame: FrameId,
    ) -> Result<Self, HumanRequestError> {
        context.ensure_open()?;
        let mut registries = context
            .human_registries
            .lock()
            .map_err(|_| HumanRequestError::Poisoned)?;
        if let Some(existing) = registries.get(&session) {
            if existing.root != root_frame {
                return Err(HumanRequestError::Rejected(
                    "session human owner root mismatch",
                ));
            }
            return Ok(Self {
                context: Arc::clone(&context),
                session,
                root_frame,
                state: Arc::clone(&existing.state),
                notifications: Arc::clone(&existing.notifications),
            });
        }
        if registries.len() >= 64 {
            return Err(HumanRequestError::Rejected(
                "context human session bound exceeded",
            ));
        }

        let _writer = context.lock_writer()?;
        let records = context
            .bus
            .store()
            .replay_iter(Seq::new(MAX_REPLAY_RECORDS))?;
        let mut state = ServiceState {
            requests: BTreeMap::new(),
            commands: BTreeMap::new(),
        };
        let mut session_seen = false;
        for record in records {
            let record = record?;
            if record.session() != session {
                continue;
            }
            session_seen = true;
            restore_record(&context, &mut state, record, root_frame)?;
        }
        if !session_seen {
            return Err(KernelError::UnknownSession(session).into());
        }

        let state = Arc::new(Mutex::new(state));
        let notifications = Arc::new(Mutex::new(BTreeMap::new()));
        registries.insert(
            session,
            HumanRegistryCache {
                root: root_frame,
                state: Arc::clone(&state),
                notifications: Arc::clone(&notifications),
            },
        );
        drop(_writer);
        drop(registries);
        Ok(Self {
            context,
            session,
            root_frame,
            state,
            notifications,
        })
    }

    /// Admit and durably store one request from the exact live frame and its ask_user capability.
    ///
    /// The supplied binding is sealed by `polaris-runtime-frames`; `token` must both match that
    /// frame instance and authorize the exact `tools/ask_user` resource under `issuer`.
    ///
    /// # Errors
    /// Rejects wrong root/session/frame, invalid capability, invalid or oversized questions,
    /// duplicate/pending requests, session count overflow, closed storage or durable write errors.
    pub async fn ask(
        &self,
        binding: &FrameToolBinding,
        token: &Token,
        issuer: &PublicKeyBytes,
        origin_step: u32,
        questions: Vec<HumanQuestion>,
    ) -> Result<HumanRequestTicket, HumanRequestError> {
        if binding.root_frame_id() != self.root_frame || !binding.matches_token(token) {
            return Err(HumanRequestError::Rejected("frame binding mismatch"));
        }
        let ask_resource = ResourcePath::new("tools/ask_user")?;
        let facts = Facts {
            resource: &ask_resource,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
            right: Right::ExecuteTool,
        };
        if authorize(token, &facts, issuer, &SystemClock)? != Authorization::Allowed {
            return Err(HumanRequestError::Rejected("ask_user capability denied"));
        }
        if questions.is_empty() || questions.len() > polaris_sdk::human::MAX_HUMAN_QUESTIONS {
            return Err(HumanContractError::Invalid("question count").into());
        }
        let registry_context = Arc::clone(&self.context);
        let registry_binding = binding.clone();
        let registry_session = self.session;
        let registry_root = self.root_frame;
        run_blocking(move || {
            let registry = crate::FrameCancellationService::new(
                registry_context,
                registry_session,
                registry_root,
                CancellationToken::new(),
            )
            .map_err(|_| HumanRequestError::Evidence)?;
            let _registered = registry.bind_blocking(registry_binding).map_err(|_| {
                HumanRequestError::Rejected("requesting frame registry rejected binding")
            })?;
            Ok(())
        })
        .await?;
        let frame = binding.frame_id();
        let request_id = HumanRequestId::new_v7();
        let payload =
            HumanRequestPayload::new(request_id, self.session, frame, origin_step, questions)?;
        let bytes = payload.encode()?;
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        let session = self.session;
        let root_frame = self.root_frame;
        run_blocking(move || {
            context.ensure_open()?;
            let _writer = context.lock_writer()?;
            if crate::frame_cancellation::durable_frame_cancelled(
                &context, session, root_frame, frame,
            )
            .map_err(|_| HumanRequestError::Evidence)?
            {
                return Err(HumanRequestError::Rejected(
                    "requesting frame is durably cancelled",
                ));
            }

            let mut state = state.lock().map_err(|_| HumanRequestError::Poisoned)?;
            if state.requests.contains_key(&request_id) {
                return Err(HumanRequestError::Integrity("request id collision"));
            }
            if state.requests.len() >= MAX_SESSION_REQUESTS {
                return Err(HumanRequestError::Rejected(
                    "session human request capacity reached",
                ));
            }
            if state.requests.values().any(|entry| {
                entry.ticket().frame == frame && matches!(entry, RequestState::Pending { .. })
            }) {
                return Err(HumanRequestError::Rejected(
                    "frame already has a pending human request",
                ));
            }
            let revision = context.cas.put(&bytes)?;
            let ticket = HumanRequestTicket {
                request_id,
                revision,
                request: revision,
                frame,
            };
            let _record = context.publish(
                session,
                Event::HumanRequestCreated {
                    request_id,
                    frame,
                    origin_step,
                    request: revision,
                },
            )?;
            state
                .requests
                .insert(request_id, RequestState::Pending { ticket });
            Ok(ticket)
        })
        .await
    }

    /// Validate and store a complete answer document in this session's CAS without resolving it.
    /// The returned Cid must be placed in `CommandBody::AnswerHumanRequest` and sent through the
    /// answer verifier/receipt path. Uncommitted staged answers are immutable orphan CAS objects.
    ///
    /// # Errors
    /// Rejects an unknown request, stale revision, invalid/incomplete response batch, over-limit
    /// serialized payload, or storage failure.
    pub async fn stage_answer(
        &self,
        request_id: HumanRequestId,
        revision: Cid,
        answers: Vec<polaris_sdk::human::HumanQuestionAnswer>,
    ) -> Result<Cid, HumanRequestError> {
        if self.outcome(request_id)? != Some(HumanRequestOutcome::Pending) {
            return Err(HumanRequestError::Rejected("request is not pending"));
        }
        if self.ticket(request_id)?.revision != revision {
            return Err(HumanRequestError::Rejected("stale request revision"));
        }
        let request = self.request_document(request_id).await?;
        let payload = HumanAnswerPayload::new(&request, revision, answers)?;
        let bytes = payload.encode_for(&request, revision)?;
        let context = Arc::clone(&self.context);
        run_blocking(move || {
            context.ensure_open()?;
            Ok(context.cas.put(&bytes)?)
        })
        .await
    }

    /// Commit one verified answer command. The answer payload is fully validated before event
    /// append; repeated identical command ids return their original durable receipt.
    ///
    /// The caller must have run the dedicated session/frame-scoped human-request verifier and
    /// must also bind this command's witness to the kernel's durable verifier receipt pipeline.
    ///
    /// # Errors
    /// Rejects wrong request/session/frame/revision, malformed answer CAS data, stale or terminal
    /// requests, duplicate command reuse, closed storage or event-log errors.
    pub async fn answer(
        &self,
        command: Command<Verified>,
    ) -> Result<HumanRequestOutcome, HumanRequestError> {
        let CommandBody::AnswerHumanRequest {
            request_id,
            revision,
            answer,
        } = *command.body()
        else {
            return Err(HumanRequestError::Rejected("not an answer command"));
        };
        let command_frame = command.frame();
        let command_id = command.id();
        let approval = command.witness().evidence();
        if command.witness().approver().as_str() != "polaris-human-request-verifier" {
            return Err(HumanRequestError::Rejected(
                "unexpected human answer verifier",
            ));
        }
        if command_frame != self.root_frame {
            return Err(HumanRequestError::Rejected(
                "answer command did not originate from the trusted host frame",
            ));
        }
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        let session = self.session;
        let result = run_blocking(move || {
            context.ensure_open()?;
            let _writer = context.lock_writer()?;
            let mut state = state.lock().map_err(|_| HumanRequestError::Poisoned)?;
            if let Some(receipt) = state.commands.get(&command_id).copied() {
                if receipt.request_id == request_id
                    && receipt.command_frame == command_frame
                    && receipt.revision == revision
                    && receipt.answer == answer
                {
                    return Ok((
                        HumanRequestOutcome::Answered {
                            answer: receipt.answer,
                            seq: receipt.seq,
                        },
                        request_id,
                    ));
                }
                return Err(HumanRequestError::Rejected(
                    "conflicting duplicate command id",
                ));
            }
            let current = state
                .requests
                .get(&request_id)
                .copied()
                .ok_or(HumanRequestError::UnknownRequest)?;
            let ticket = match current {
                RequestState::Pending { ticket } => ticket,
                RequestState::Interrupted { seq, .. } => {
                    return Ok((HumanRequestOutcome::Interrupted { seq }, request_id));
                }
                RequestState::Answered { .. } => {
                    return Err(HumanRequestError::Rejected(
                        "request already has a different answer command",
                    ));
                }
            };

            if crate::frame_cancellation::durable_frame_cancelled(
                &context,
                session,
                command_frame,
                ticket.frame,
            )
            .map_err(|_| HumanRequestError::Evidence)?
            {
                return Err(HumanRequestError::Rejected(
                    "requesting frame is durably cancelled",
                ));
            }
            if ticket.revision != revision {
                return Err(HumanRequestError::Rejected("stale request revision"));
            }
            let request = load_request_for_session(&context, ticket, session)?;
            let bytes = context.cas.get_bounded(
                &answer,
                u64::try_from(polaris_sdk::human::MAX_HUMAN_ANSWER_BYTES).map_err(|_| {
                    HumanRequestError::Integrity("answer bound is not representable")
                })?,
            )?;
            let payload: HumanAnswerPayload = serde_json::from_slice(&bytes)
                .map_err(|_| HumanRequestError::Rejected("answer payload is malformed"))?;
            payload.validate_for(&request, revision)?;
            let audit = StoredAnswerReceipt {
                schema: 1,
                session,
                request_id,
                frame: ticket.frame,
                command_frame,
                command: command_id,
                revision,
                answer,
                approval,
            };
            verify_answer_evidence(&context, &audit)?;
            let receipt_bytes =
                serde_json::to_vec(&audit).map_err(|_| HumanRequestError::Encoding)?;
            if u64::try_from(receipt_bytes.len()).map_err(|_| HumanRequestError::Encoding)?
                > MAX_RECEIPT_BYTES
            {
                return Err(HumanRequestError::Encoding);
            }
            let receipt = context.cas.put(&receipt_bytes)?;
            if context
                .cas
                .get_bounded(&receipt, MAX_RECEIPT_BYTES)?
                .as_ref()
                != receipt_bytes.as_slice()
            {
                return Err(HumanRequestError::Evidence);
            }
            let record = context.publish(
                session,
                Event::HumanRequestAnswered {
                    request_id,
                    frame: ticket.frame,
                    receipt,
                },
            )?;
            state.commands.insert(
                command_id,
                AnswerReceipt {
                    request_id,
                    command_frame,
                    revision,
                    answer,
                    seq: record.seq(),
                },
            );
            state.requests.insert(
                request_id,
                RequestState::Answered {
                    ticket,
                    answer,
                    seq: record.seq(),
                },
            );
            Ok((
                HumanRequestOutcome::Answered {
                    answer,
                    seq: record.seq(),
                },
                request_id,
            ))
        })
        .await?;
        self.notify(result.1)?;
        Ok(result.0)
    }

    /// Durably interrupt the single pending request owned by `frame` and wake only its waiter.
    /// This is called by the verified CancelFrame route for that actual runtime frame.
    ///
    /// # Errors
    /// Rejects a frame with no pending request or closed/corrupt storage; concurrent answer/cancel
    /// transitions serialize and the losing caller receives the already-committed outcome.
    pub async fn interrupt_frame(
        &self,
        frame: FrameId,
    ) -> Result<HumanRequestOutcome, HumanRequestError> {
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        let session = self.session;
        let result = run_blocking(move || {
            context.ensure_open()?;
            let _writer = context.lock_writer()?;
            let mut state = state.lock().map_err(|_| HumanRequestError::Poisoned)?;
            let pending = state.requests.iter().find_map(|(id, entry)| match entry {
                RequestState::Pending { ticket } if ticket.frame == frame => Some((*id, *ticket)),
                _ => None,
            });
            let Some((request_id, ticket)) = pending else {
                return state
                    .requests
                    .values()
                    .find_map(|entry| {
                        let ticket = entry.ticket();
                        if ticket.frame != frame {
                            return None;
                        }
                        match entry {
                            RequestState::Answered { answer, seq, .. } => Some(Ok((
                                HumanRequestOutcome::Answered {
                                    answer: *answer,
                                    seq: *seq,
                                },
                                ticket.request_id,
                            ))),
                            RequestState::Interrupted { seq, .. } => Some(Ok((
                                HumanRequestOutcome::Interrupted { seq: *seq },
                                ticket.request_id,
                            ))),
                            RequestState::Pending { .. } => None,
                        }
                    })
                    .ok_or(HumanRequestError::NoPendingRequest)?;
            };
            let record = context.publish(
                session,
                Event::HumanRequestInterrupted {
                    request_id,
                    frame,
                    revision: ticket.revision,
                },
            )?;
            state.requests.insert(
                request_id,
                RequestState::Interrupted {
                    ticket,
                    seq: record.seq(),
                },
            );
            Ok((
                HumanRequestOutcome::Interrupted { seq: record.seq() },
                request_id,
            ))
        })
        .await?;
        self.notify(result.1)?;
        Ok(result.0)
    }

    /// Wait for this request only. Cancellation commits a requester-scoped interruption; dropping
    /// the waiter leaves the durable request pending so process restart can resume it.
    ///
    /// # Errors
    /// Returns closed/corrupt state or an unknown request; no sibling or session token is touched.
    pub async fn wait(
        &self,
        request_id: HumanRequestId,
        cancellation: CancellationToken,
    ) -> Result<HumanRequestOutcome, HumanRequestError> {
        loop {
            let notify = self.notifier(request_id)?;
            if let Some(outcome) = self.outcome(request_id)?
                && outcome != HumanRequestOutcome::Pending
            {
                return Ok(outcome);
            }
            tokio::select! {
                () = cancellation.cancelled() => {
                    let frame = self.ticket(request_id)?.frame;
                    let _outcome = self.interrupt_frame(frame).await?;
                }
                () = notify.notified() => {}
            }
        }
    }

    /// Return a validated snapshot of all requests for this session, in stable request-id order.
    /// # Errors
    /// Returns closed, poisoned or corrupt CAS state.
    pub async fn list(&self) -> Result<Vec<HumanRequestSnapshot>, HumanRequestError> {
        let context = Arc::clone(&self.context);
        let state = Arc::clone(&self.state);
        let session = self.session;
        run_blocking(move || {
            context.ensure_open()?;
            let state = state.lock().map_err(|_| HumanRequestError::Poisoned)?;
            state
                .requests
                .values()
                .map(|entry| {
                    let ticket = entry.ticket();
                    Ok(HumanRequestSnapshot {
                        ticket,
                        request: load_request_for_session(&context, ticket, session)?,
                        outcome: entry.outcome(),
                    })
                })
                .collect()
        })
        .await
    }

    /// Read the durable outcome for one request.
    /// # Errors
    /// Returns a typed error for an unknown id or poisoned state.
    pub fn outcome(
        &self,
        request_id: HumanRequestId,
    ) -> Result<Option<HumanRequestOutcome>, HumanRequestError> {
        let state = self.state.lock().map_err(|_| HumanRequestError::Poisoned)?;
        Ok(state
            .requests
            .get(&request_id)
            .copied()
            .map(RequestState::outcome))
    }

    /// Read a request ticket for trusted frame/runtime composition.
    /// # Errors
    /// Returns a typed error for an unknown id or poisoned state.
    pub fn ticket(
        &self,
        request_id: HumanRequestId,
    ) -> Result<HumanRequestTicket, HumanRequestError> {
        let state = self.state.lock().map_err(|_| HumanRequestError::Poisoned)?;
        state
            .requests
            .get(&request_id)
            .copied()
            .map(RequestState::ticket)
            .ok_or(HumanRequestError::UnknownRequest)
    }

    async fn request_document(
        &self,
        request_id: HumanRequestId,
    ) -> Result<HumanRequestPayload, HumanRequestError> {
        let context = Arc::clone(&self.context);
        let session = self.session;
        let ticket = self.ticket(request_id)?;
        run_blocking(move || load_request_for_session(&context, ticket, session)).await
    }

    fn notifier(&self, request_id: HumanRequestId) -> Result<Arc<Notify>, HumanRequestError> {
        if !self
            .state
            .lock()
            .map_err(|_| HumanRequestError::Poisoned)?
            .requests
            .contains_key(&request_id)
        {
            return Err(HumanRequestError::UnknownRequest);
        }
        let mut notifications = self
            .notifications
            .lock()
            .map_err(|_| HumanRequestError::Poisoned)?;
        if let Some(notify) = notifications.get(&request_id) {
            return Ok(Arc::clone(notify));
        }
        let notify = Arc::new(Notify::new());
        notifications.insert(request_id, Arc::clone(&notify));
        Ok(notify)
    }

    fn notify(&self, request_id: HumanRequestId) -> Result<(), HumanRequestError> {
        if let Some(notify) = self
            .notifications
            .lock()
            .map_err(|_| HumanRequestError::Poisoned)?
            .get(&request_id)
        {
            notify.notify_one();
        }
        Ok(())
    }
}

async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, HumanRequestError> + Send + 'static,
) -> Result<T, HumanRequestError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| HumanRequestError::Owner)?
}

fn restore_record(
    context: &KernelCtx,
    state: &mut ServiceState,
    record: EventRecord,
    root_frame: FrameId,
) -> Result<(), HumanRequestError> {
    match *record.event() {
        Event::HumanRequestCreated {
            request_id,
            frame,
            origin_step,
            request,
        } => {
            let revision = request;
            let ticket = HumanRequestTicket {
                request_id,
                revision,
                request,
                frame,
            };
            let payload = load_request_for_session(context, ticket, record.session())?;
            if payload.origin_step != origin_step {
                return Err(HumanRequestError::Integrity(
                    "origin step differs from request payload",
                ));
            }
            if state.requests.contains_key(&request_id) {
                return Err(HumanRequestError::Integrity("duplicate request creation"));
            }
            if state.requests.values().any(|entry| {
                entry.ticket().frame == frame && matches!(entry, RequestState::Pending { .. })
            }) {
                return Err(HumanRequestError::Integrity(
                    "multiple pending requests for frame",
                ));
            }
            if state.requests.len() >= MAX_SESSION_REQUESTS {
                return Err(HumanRequestError::Integrity(
                    "session request limit exceeded in history",
                ));
            }
            state
                .requests
                .insert(request_id, RequestState::Pending { ticket });
        }
        Event::HumanRequestAnswered {
            request_id,
            frame,
            receipt,
        } => {
            let bytes = context.cas.get_bounded(&receipt, MAX_RECEIPT_BYTES)?;
            let audit: StoredAnswerReceipt = serde_json::from_slice(&bytes)
                .map_err(|_| HumanRequestError::Integrity("malformed answer receipt"))?;
            if audit.schema != 1
                || audit.session != record.session()
                || audit.request_id != request_id
                || audit.frame != frame
            {
                return Err(HumanRequestError::Integrity(
                    "answer receipt binding mismatch",
                ));
            }
            verify_answer_evidence(context, &audit)?;
            let StoredAnswerReceipt {
                command_frame,
                command,
                revision,
                answer,
                ..
            } = audit;
            let RequestState::Pending { ticket } = state
                .requests
                .get(&request_id)
                .copied()
                .ok_or(HumanRequestError::Integrity("answer without request"))?
            else {
                return Err(HumanRequestError::Integrity(
                    "answer after terminal outcome",
                ));
            };
            if ticket.frame != frame || ticket.revision != revision {
                return Err(HumanRequestError::Integrity(
                    "answer owner or revision mismatch",
                ));
            }
            if command_frame != root_frame {
                return Err(HumanRequestError::Integrity(
                    "answer command came from another host frame",
                ));
            }
            let request = load_request_for_session(context, ticket, record.session())?;
            let answer_payload = load_answer(context, answer)?;
            answer_payload
                .validate_for(&request, revision)
                .map_err(|_| HumanRequestError::Integrity("answer payload violates request"))?;
            if state
                .commands
                .insert(
                    command,
                    AnswerReceipt {
                        request_id,
                        command_frame,
                        revision,
                        answer,
                        seq: record.seq(),
                    },
                )
                .is_some()
            {
                return Err(HumanRequestError::Integrity("answer command id reused"));
            }
            state.requests.insert(
                request_id,
                RequestState::Answered {
                    ticket,
                    answer,
                    seq: record.seq(),
                },
            );
        }
        Event::HumanRequestInterrupted {
            request_id,
            frame,
            revision,
        } => {
            let RequestState::Pending { ticket } = state
                .requests
                .get(&request_id)
                .copied()
                .ok_or(HumanRequestError::Integrity("interruption without request"))?
            else {
                return Err(HumanRequestError::Integrity(
                    "interruption after terminal outcome",
                ));
            };
            if ticket.frame != frame || ticket.revision != revision {
                return Err(HumanRequestError::Integrity(
                    "interruption owner or revision mismatch",
                ));
            }
            state.requests.insert(
                request_id,
                RequestState::Interrupted {
                    ticket,
                    seq: record.seq(),
                },
            );
        }
        _ => {}
    }
    Ok(())
}

fn load_request_for_session(
    context: &KernelCtx,
    ticket: HumanRequestTicket,
    session: SessionId,
) -> Result<HumanRequestPayload, HumanRequestError> {
    let bytes = context.cas.get_bounded(
        &ticket.request,
        u64::try_from(polaris_sdk::human::MAX_HUMAN_REQUEST_BYTES)
            .map_err(|_| HumanRequestError::Integrity("request bound is not representable"))?,
    )?;
    let payload: HumanRequestPayload = serde_json::from_slice(&bytes)
        .map_err(|_| HumanRequestError::Integrity("request payload is malformed"))?;
    payload
        .validate()
        .map_err(|_| HumanRequestError::Integrity("request payload violates its schema"))?;
    if payload.request_id != ticket.request_id
        || payload.frame != ticket.frame
        || payload.session != session
        || ticket.request != ticket.revision
    {
        return Err(HumanRequestError::Integrity(
            "request payload and event identity differ",
        ));
    }
    Ok(payload)
}

fn verify_answer_evidence(
    context: &KernelCtx,
    audit: &StoredAnswerReceipt,
) -> Result<(), HumanRequestError> {
    let body = CommandBody::AnswerHumanRequest {
        request_id: audit.request_id,
        revision: audit.revision,
        answer: audit.answer,
    };
    let expected = serde_json::to_vec(&(
        "polaris.human-answer.v1",
        audit.command,
        audit.command_frame,
        body,
    ))
    .map_err(|_| HumanRequestError::Encoding)?;
    let actual = context
        .cas
        .get_bounded(&audit.approval, MAX_RECEIPT_BYTES)?;
    if actual.as_ref() != expected.as_slice() {
        return Err(HumanRequestError::Evidence);
    }
    Ok(())
}

fn load_answer(context: &KernelCtx, answer: Cid) -> Result<HumanAnswerPayload, HumanRequestError> {
    let bytes = context.cas.get_bounded(
        &answer,
        u64::try_from(polaris_sdk::human::MAX_HUMAN_ANSWER_BYTES)
            .map_err(|_| HumanRequestError::Integrity("answer bound is not representable"))?,
    )?;
    serde_json::from_slice(&bytes)
        .map_err(|_| HumanRequestError::Integrity("answer payload is malformed"))
}

/// Fail-closed errors from human request construction, authorization, persistence and replay.
#[derive(Debug, thiserror::Error)]
pub enum HumanRequestError {
    /// Input or capability denied without revealing credential details.
    #[error("human request rejected: {0}")]
    Rejected(&'static str),
    /// No request exists for the requested id or frame.
    #[error("no pending human request")]
    NoPendingRequest,
    /// Requested identity is not in this session.
    #[error("unknown human request")]
    UnknownRequest,
    /// Persisted request lifecycle is inconsistent or corrupt.
    #[error("human request replay integrity failure: {0}")]
    Integrity(&'static str),
    /// Request state mutex was poisoned.
    #[error("human request state poisoned")]
    Poisoned,
    /// JSON serialization failed.
    #[error("human request encoding failed")]
    Encoding,
    /// CAS evidence could not be verified.
    #[error("human request verifier evidence failed")]
    Evidence,
    /// Human payload validation failure.
    #[error(transparent)]
    Contract(#[from] HumanContractError),
    /// Kernel durable storage failure.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// Storage CAS error.
    #[error(transparent)]
    Cas(#[from] crate::storage::cas::CasError),
    /// Event-log error.
    #[error(transparent)]
    EventLog(#[from] crate::storage::eventlog::EventLogError),
    /// Capability validation failure.
    #[error(transparent)]
    Capability(#[from] polaris_security::capability::CapabilityError),
    /// Verifier evidence error.
    #[error(transparent)]
    VerifierEvidence(#[from] crate::verifier::EvidenceError),
    /// Join or owner task error.
    #[error("human request storage owner failed")]
    Owner,
}

impl From<tokio::task::JoinError> for HumanRequestError {
    fn from(_error: tokio::task::JoinError) -> Self {
        Self::Owner
    }
}
