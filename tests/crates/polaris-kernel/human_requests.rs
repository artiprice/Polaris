//! Durable Human Request Backend (descriptor: specs/descriptors/human_request_backend.json).

use polaris_kernel::verifier::EvidenceStore;
use polaris_kernel::{
    HumanRequestOutcome, HumanRequestService, HumanRequestVerifier, Kernel, KernelEvidence,
    KernelHandle, LocalKernelConfig,
};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event},
    effort::Effort,
    human::{
        HumanOption, HumanOptionId, HumanQuestion, HumanQuestionAnswer, HumanQuestionId,
        HumanQuestionKind, HumanRequestId, HumanResponse, HumanSelection,
    },
    ids::{Cid, FrameId, Seq, SessionId},
};
use polaris_security::capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    root: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: KernelHandle,
    session: SessionId,
    frame_id: FrameId,
    frame: AgentContextFrame<Active>,
    ask_authority: Authority,
    ask_token: Token,
}

impl Fixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let session = SessionId::new_v7();
        let frame_id = FrameId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            frame_id,
        ))
        .await?;
        let ask_authority = Authority::new_session()?;
        let ask_scope = Scope {
            resource: ResourceScope::Exact(ResourcePath::new("tools/ask_user")?),
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::ExecuteTool]),
        };
        let ask_token = ask_authority.issue_root(frame_id, &ask_scope, &SystemClock)?;
        let expiry = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("frame expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            frame_id,
            AgentRole::Orchestrator,
            ask_token.clone(),
            FrameScope {
                capability: ask_scope,
                expires_at_micros: expiry,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 8,
                max_depth: 3,
                token_budget: 1000,
            },
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: polaris_sdk::ids::ModelId::new("test/model")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        Ok(Self {
            root,
            workspace,
            kernel,
            session,
            frame_id,
            frame,
            ask_authority,
            ask_token,
        })
    }

    fn service(&self) -> TestResult<HumanRequestService> {
        Ok(HumanRequestService::new(
            self.kernel.context().clone(),
            self.session,
            self.frame_id,
        )?)
    }

    fn questions() -> TestResult<Vec<HumanQuestion>> {
        Ok(vec![HumanQuestion {
            id: HumanQuestionId::new("route")?,
            prompt: "Which route should Polaris take?".to_owned(),
            required: true,
            kind: HumanQuestionKind::SingleChoice {
                options: vec![
                    HumanOption {
                        id: HumanOptionId::new("safe")?,
                        label: "Safe".to_owned(),
                    },
                    HumanOption {
                        id: HumanOptionId::new("fast")?,
                        label: "Fast".to_owned(),
                    },
                ],
            },
        }])
    }

    fn answers() -> TestResult<Vec<HumanQuestionAnswer>> {
        Ok(vec![HumanQuestionAnswer {
            question_id: HumanQuestionId::new("route")?,
            response: HumanResponse::Single {
                selection: HumanSelection::Option(HumanOptionId::new("safe")?),
            },
            notes: Some("Keep the workspace isolated".to_owned()),
        }])
    }

    fn verified_command(
        &self,
        verifier: &HumanRequestVerifier,
        request_id: HumanRequestId,
        revision: Cid,
        answer: Cid,
        command_id: u128,
    ) -> TestResult<polaris_sdk::contracts::Command<polaris_sdk::contracts::Verified>> {
        let token = verifier.credential(Duration::from_secs(60))?;
        let command = Command::new(
            polaris_sdk::ids::CommandId::from_u128(command_id),
            self.frame_id,
            token,
            CommandBody::AnswerHumanRequest {
                request_id,
                revision,
                answer,
            },
        );
        let evidence = KernelEvidence::new(self.kernel.context().clone());
        Ok(verifier.verify(command, &evidence)?)
    }
}

async fn create_request(
    fixture: &Fixture,
    service: &HumanRequestService,
) -> TestResult<polaris_kernel::HumanRequestTicket> {
    Ok(service
        .ask(
            &fixture.frame.tool_binding(),
            &fixture.ask_token,
            &fixture.ask_authority.public_key(),
            1,
            Fixture::questions()?,
        )
        .await?)
}

#[tokio::test]
async fn human_request_answer_is_atomic_and_durable() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let ticket = create_request(&fixture, &service).await?;
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let command =
        fixture.verified_command(&verifier, ticket.request_id, ticket.revision, answer, 7)?;
    let outcome = service.answer(command).await?;
    let HumanRequestOutcome::Answered {
        answer: committed,
        seq,
    } = outcome
    else {
        return Err("answer was not durably committed".into());
    };
    assert_eq!(committed, answer);
    if let Some(Event::HumanRequestAnswered { receipt, .. }) = fixture
        .kernel
        .context()
        .records(seq, seq)?
        .first()
        .map(|r| *r.event())
    {
        let audit: serde_json::Value = serde_json::from_slice(
            &KernelEvidence::new(fixture.kernel.context().clone())
                .get(&receipt)?
                .ok_or("receipt CAS missing")?,
        )?;
        assert_eq!(audit.get("answer"), Some(&serde_json::to_value(answer)?));
    } else {
        return Err("missing durable answer receipt".into());
    }
    let persisted = fixture.kernel.context().records(seq, seq)?;
    assert!(
        matches!(persisted.first().map(|record| record.event()), Some(Event::HumanRequestAnswered { request_id, .. }) if *request_id == ticket.request_id)
    );
    assert_eq!(
        service
            .wait(ticket.request_id, CancellationToken::new())
            .await?,
        outcome
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn human_request_rejects_invalid_stale_and_duplicate_answers() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let ticket = create_request(&fixture, &service).await?;
    assert!(
        service
            .stage_answer(
                ticket.request_id,
                Cid::from_bytes([99; 32]),
                Fixture::answers()?
            )
            .await
            .is_err()
    );
    assert!(
        service
            .stage_answer(ticket.request_id, ticket.revision, vec![])
            .await
            .is_err()
    );
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let command =
        fixture.verified_command(&verifier, ticket.request_id, ticket.revision, answer, 8)?;
    let first = service.answer(command).await?;
    let repeated =
        fixture.verified_command(&verifier, ticket.request_id, ticket.revision, answer, 8)?;
    assert_eq!(service.answer(repeated).await?, first);
    let conflicting = fixture.verified_command(
        &verifier,
        ticket.request_id,
        ticket.revision,
        Cid::from_bytes([91; 32]),
        8,
    )?;
    assert!(service.answer(conflicting).await.is_err());
    assert_eq!(service.outcome(ticket.request_id)?, Some(first));
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_request_answer_cancel_race_has_one_winner() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = HumanRequestService::new(
        fixture.kernel.context().clone(),
        fixture.session,
        fixture.frame_id,
    )?;
    let ticket = create_request(&fixture, &service).await?;
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let command =
        fixture.verified_command(&verifier, ticket.request_id, ticket.revision, answer, 9)?;
    let (answer_result, cancel_result) = tokio::join!(
        service.answer(command),
        service.interrupt_frame(ticket.frame),
    );
    let answer_result = answer_result?;
    let cancel_result = cancel_result?;
    assert_eq!(answer_result, cancel_result);
    let records = fixture
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let terminal_events = records
        .iter()
        .filter(|record| record.session() == fixture.session)
        .filter(|record| matches!(record.event(), Event::HumanRequestAnswered { request_id, .. } | Event::HumanRequestInterrupted { request_id, .. } if *request_id == ticket.request_id))
        .count();
    assert_eq!(terminal_events, 1);
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn human_request_rehydrates_pending_and_terminal_states() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let answered_ticket = create_request(&fixture, &service).await?;
    let answer = service
        .stage_answer(
            answered_ticket.request_id,
            answered_ticket.revision,
            Fixture::answers()?,
        )
        .await?;
    let verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let command = fixture.verified_command(
        &verifier,
        answered_ticket.request_id,
        answered_ticket.revision,
        answer,
        10,
    )?;
    let _answered = service.answer(command).await?;
    let pending_ticket = create_request(&fixture, &service).await?;
    let _interrupted = service.interrupt_frame(pending_ticket.frame).await?;
    let live_pending_ticket = create_request(&fixture, &service).await?;
    assert_eq!(
        service.outcome(live_pending_ticket.request_id)?,
        Some(HumanRequestOutcome::Pending)
    );
    drop(service);
    let _first_report = fixture.kernel.shutdown().await?;
    drop(fixture.kernel);

    let reopened = Kernel::start(LocalKernelConfig::new(
        fixture.root.path(),
        fixture.workspace.path(),
        fixture.session,
        fixture.frame_id,
    ))
    .await?;
    let restored = HumanRequestService::new(
        reopened.context().clone(),
        fixture.session,
        fixture.frame_id,
    )?;
    let snapshots = restored.list().await?;
    assert_eq!(snapshots.len(), 3);
    assert_eq!(
        restored.outcome(answered_ticket.request_id)?,
        Some(HumanRequestOutcome::Answered {
            answer,
            seq: snapshots
                .iter()
                .find(|snapshot| snapshot.ticket.request_id == answered_ticket.request_id)
                .and_then(|snapshot| match snapshot.outcome {
                    HumanRequestOutcome::Answered { seq, .. } => Some(seq),
                    HumanRequestOutcome::Pending | HumanRequestOutcome::Interrupted { .. } => None,
                })
                .ok_or("answered transition missing after rehydration")?,
        })
    );
    assert_eq!(
        restored.outcome(pending_ticket.request_id)?,
        Some(HumanRequestOutcome::Interrupted {
            seq: snapshots
                .iter()
                .find(|snapshot| snapshot.ticket.request_id == pending_ticket.request_id)
                .and_then(|snapshot| match snapshot.outcome {
                    HumanRequestOutcome::Interrupted { seq } => Some(seq),
                    HumanRequestOutcome::Pending | HumanRequestOutcome::Answered { .. } => None,
                })
                .ok_or("interrupted transition missing after rehydration")?,
        })
    );
    assert_eq!(
        restored.outcome(live_pending_ticket.request_id)?,
        Some(HumanRequestOutcome::Pending)
    );
    let _second_report = reopened.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn host_answer_resumes_only_its_pending_request() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.kernel.human_requests();
    let ticket = create_request(&fixture, service).await?;
    let receipt = fixture
        .kernel
        .answer_human_request(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let outcome = service
        .wait(ticket.request_id, CancellationToken::new())
        .await?;
    assert!(
        matches!(outcome, HumanRequestOutcome::Answered { seq, .. } if seq == receipt.accepted_at)
    );
    let record = fixture
        .kernel
        .context()
        .records(receipt.accepted_at, receipt.accepted_at)?;
    assert!(
        matches!(record.first().map(|r| r.event()), Some(Event::HumanRequestAnswered { request_id, .. }) if *request_id == ticket.request_id)
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn host_answer_rejects_untrusted_control_token() -> TestResult {
    use polaris_sdk::registry::KernelApi;
    let fixture = Fixture::new().await?;
    let service = fixture.kernel.human_requests();
    let ticket = create_request(&fixture, service).await?;
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let command = Command::new(
        polaris_sdk::ids::CommandId::new_v7(),
        fixture.frame_id,
        fixture.ask_token.to_bytes().clone(),
        CommandBody::AnswerHumanRequest {
            request_id: ticket.request_id,
            revision: ticket.revision,
            answer,
        },
    );
    assert!(fixture.kernel.submit(command).await.is_err());
    assert_eq!(
        service.outcome(ticket.request_id)?,
        Some(HumanRequestOutcome::Pending)
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

// Explicit synthetic adapter: bytes survive verifier read-back, but never enter kernel CAS.
#[derive(Default)]
struct NonDurableEvidence(std::sync::Mutex<Option<(Cid, Vec<u8>)>>);
impl EvidenceStore for NonDurableEvidence {
    fn put(&self, bytes: &[u8]) -> Result<Cid, polaris_kernel::verifier::EvidenceError> {
        let cid = polaris_storage::cas::content_id(bytes);
        *self
            .0
            .lock()
            .map_err(|_| polaris_kernel::verifier::EvidenceError::Backend)? =
            Some((cid, bytes.to_vec()));
        Ok(cid)
    }
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, polaris_kernel::verifier::EvidenceError> {
        let state = self
            .0
            .lock()
            .map_err(|_| polaris_kernel::verifier::EvidenceError::Backend)?;
        Ok(match state.as_ref() {
            Some((stored, bytes)) if stored == cid => Some(bytes.clone()),
            _ => None,
        })
    }
}
#[tokio::test]
async fn human_request_rejects_missing_durable_witness_before_terminal_event() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.kernel.human_requests();
    let ticket = create_request(&fixture, service).await?;
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let authority = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let command = Command::new(
        polaris_sdk::ids::CommandId::new_v7(),
        fixture.frame_id,
        authority.credential(Duration::from_secs(60))?,
        CommandBody::AnswerHumanRequest {
            request_id: ticket.request_id,
            revision: ticket.revision,
            answer,
        },
    );
    let verified = authority.verify(command, &NonDurableEvidence::default())?;
    assert!(
        service.answer(verified).await.is_err(),
        "non-durable witness must not resolve request"
    );
    assert_eq!(
        service.outcome(ticket.request_id)?,
        Some(HumanRequestOutcome::Pending)
    );
    let _report = fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn durable_cancel_prevents_later_question_and_answer() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.service()?;
    let cancellations = polaris_kernel::FrameCancellationService::new(
        fixture.kernel.context().clone(),
        fixture.session,
        fixture.frame_id,
        CancellationToken::new(),
    )?;
    cancellations.register(fixture.frame.tool_binding()).await?;
    let ticket = create_request(&fixture, &service).await?;
    let answer = service
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let answer_verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let staged = fixture.verified_command(
        &answer_verifier,
        ticket.request_id,
        ticket.revision,
        answer,
        90,
    )?;
    let cancel_verifier = polaris_kernel::verifier::FrameCancellationVerifier::new(
        fixture.session,
        fixture.frame_id,
    )?;
    let cancel = Command::new(
        polaris_sdk::ids::CommandId::new_v7(),
        fixture.frame_id,
        cancel_verifier.credential(fixture.frame_id, Duration::from_secs(60))?,
        CommandBody::CancelFrame {
            frame: fixture.frame_id,
        },
    );
    cancellations
        .apply_verified(cancel_verifier.verify(
            cancel,
            &KernelEvidence::new(fixture.kernel.context().clone()),
        )?)
        .await?;
    assert!(
        service.answer(staged).await.is_err(),
        "an answer may not cross an already committed cancellation intent"
    );
    service.interrupt_frame(fixture.frame_id).await?;
    assert!(
        create_request(&fixture, &service).await.is_err(),
        "a cancelled identity cannot publish another question after its prior interruption"
    );
    assert_eq!(
        service.list().await?.len(),
        1,
        "cancelled frame cannot create durable extra requests"
    );
    fixture.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn restart_interrupts_request_left_after_durable_cancel_intent() -> TestResult {
    let fixture = Fixture::new().await?;
    let service = fixture.kernel.human_requests().clone();
    let cancellations = polaris_kernel::FrameCancellationService::new(
        fixture.kernel.context().clone(),
        fixture.session,
        fixture.frame_id,
        CancellationToken::new(),
    )?;
    cancellations.register(fixture.frame.tool_binding()).await?;
    let ticket = create_request(&fixture, &service).await?;
    let verifier = polaris_kernel::verifier::FrameCancellationVerifier::new(
        fixture.session,
        fixture.frame_id,
    )?;
    let command = Command::new(
        polaris_sdk::ids::CommandId::new_v7(),
        fixture.frame_id,
        verifier.credential(fixture.frame_id, Duration::from_secs(60))?,
        CommandBody::CancelFrame {
            frame: fixture.frame_id,
        },
    );
    cancellations
        .apply_verified(verifier.verify(
            command,
            &KernelEvidence::new(fixture.kernel.context().clone()),
        )?)
        .await?;
    assert_eq!(
        service
            .list()
            .await?
            .first()
            .ok_or("missing pending request")?
            .outcome,
        HumanRequestOutcome::Pending,
        "fault fixture stops between durable intent and request interruption"
    );
    fixture.kernel.shutdown().await?;
    drop(service);
    drop(cancellations);
    let Fixture {
        root,
        workspace,
        kernel,
        session,
        frame_id,
        ..
    } = fixture;
    drop(kernel);
    let resumed = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        session,
        frame_id,
    ))
    .await?;
    let requests = resumed.human_requests().list().await?;
    assert!(
        matches!(
            requests.first().map(|r| r.outcome),
            Some(HumanRequestOutcome::Interrupted { .. })
        ),
        "startup must complete durable cancellation interruption before exposing host admission"
    );
    assert_eq!(resumed.context().records(Seq::new(1),Seq::new(u64::MAX))?.iter().filter(|r|matches!(r.event(),Event::HumanRequestInterrupted {request_id,..} if *request_id==ticket.request_id)).count(),1);
    resumed.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn independent_service_handles_cannot_commit_two_answers() -> TestResult {
    let fixture = Fixture::new().await?;
    let first = fixture.service()?;
    let ticket = create_request(&fixture, &first).await?;
    let second = fixture.service()?;
    let a = first
        .stage_answer(ticket.request_id, ticket.revision, Fixture::answers()?)
        .await?;
    let mut different = Fixture::answers()?;
    different.first_mut().ok_or("missing answer")?.notes =
        Some("Independent second command".into());
    let b = second
        .stage_answer(ticket.request_id, ticket.revision, different)
        .await?;
    let verifier = HumanRequestVerifier::new(fixture.session, fixture.frame_id)?;
    let a = fixture.verified_command(&verifier, ticket.request_id, ticket.revision, a, 101)?;
    let b = fixture.verified_command(&verifier, ticket.request_id, ticket.revision, b, 102)?;
    let (a, b) = tokio::join!(first.answer(a), second.answer(b));
    assert_eq!(
        [&a, &b]
            .iter()
            .filter(|result| matches!(result, Ok(HumanRequestOutcome::Answered { .. })))
            .count(),
        1,
        "separately constructed services must share one atomic answer state"
    );
    assert_eq!(fixture.kernel.context().records(Seq::new(1),Seq::new(u64::MAX))?.iter().filter(|record|matches!(record.event(),Event::HumanRequestAnswered {request_id,..} if *request_id==ticket.request_id)).count(),1,"exactly one durable answer transition");
    fixture.kernel.shutdown().await?;
    Ok(())
}
