//! Synthetic engine contracts for the real consumer policy boundary; no model-quality claims.
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{
        Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer, QuestionId,
    },
};
use polaris_system_one::{
    policy::*,
    schemas::{Consumer, FailMode, SchemaSet, Verdict},
};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone)]
struct SyntheticEngine {
    calls: Arc<AtomicUsize>,
    mode: Mode,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
#[derive(Clone, Copy)]
enum Mode {
    Unavailable,
    Invalid,
    Answer,
    Wait,
}
impl SyntheticEngine {
    fn new(mode: Mode) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            mode,
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        }
    }
}
impl DecisionEngine for SyntheticEngine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.entered.notify_one();
        match self.mode {
            Mode::Unavailable => Err(DecisionError::Unavailable("synthetic outage".into())),
            Mode::Invalid => Ok(Decision {
                answers: Vec::new(),
                latency_us: 0,
            }),
            Mode::Wait => {
                self.release.notified().await;
                answer(questions)
            }
            Mode::Answer => answer(questions),
        }
    }
}
fn answer(questions: &[Question]) -> Result<Decision, DecisionError> {
    let question = questions
        .first()
        .ok_or_else(|| DecisionError::InvalidSchema("synthetic empty question".into()))?;
    Ok(Decision {
        answers: vec![QuestionAnswer {
            question: question.id.clone(),
            answer: Answer::Noul {
                yes: Probability::new(0.99)
                    .map_err(|_| DecisionError::Model("synthetic probability".into()))?,
            },
        }],
        latency_us: 1,
    })
}
fn input() -> DecisionInput {
    DecisionInput {
        text: "already masked sample".into(),
        context: None,
    }
}
fn config() -> PolicyConfig {
    PolicyConfig {
        failure_threshold: 2,
        max_inflight: 1,
        max_deadline: Duration::from_millis(40),
    }
}
fn client(
    engine: SyntheticEngine,
) -> Result<PolicyClient<SyntheticEngine, SchemaSet>, Box<dyn Error>> {
    Ok(PolicyClient::new(
        engine,
        SchemaSet::load_builtin()?,
        config(),
    )?)
}
#[test]
fn consumer_matrix_preserves_closed_safety_boundaries() -> TestResult {
    for consumer in [
        Consumer::SafetyGate,
        Consumer::PiiMask,
        Consumer::Postflight,
        Consumer::DiffArbiter,
    ] {
        assert_eq!(minimum_failure_mode(consumer), FailMode::Closed);
    }
    for consumer in [
        Consumer::Triage,
        Consumer::ToolPrune,
        Consumer::Watchdog,
        Consumer::Sifter,
        Consumer::EffortRouter,
        Consumer::SandboxRoute,
        Consumer::EnvMatch,
        Consumer::NvidiaProfile,
        Consumer::SkillSelect,
    ] {
        assert_eq!(minimum_failure_mode(consumer), FailMode::Open);
    }
    Ok(())
}
#[tokio::test]
async fn deadline_and_invalid_answers_return_typed_failure() -> TestResult {
    let waiting = client(SyntheticEngine::new(Mode::Wait))?;
    assert_eq!(
        waiting
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::Timeout)
    );
    let invalid = client(SyntheticEngine::new(Mode::Invalid))?;
    assert_eq!(
        invalid
            .decide(Consumer::SafetyGate, "safety.command", 1, &input(), None)
            .await,
        PolicyOutcome::FailedClosed(FailureReason::InvalidDecision)
    );
    assert_eq!(
        invalid
            .decide(Consumer::Triage, "safety.command", 1, &input(), None)
            .await,
        PolicyOutcome::FailedClosed(FailureReason::PolicyMismatch)
    );
    Ok(())
}
#[tokio::test]
async fn circuit_opens_and_requires_host_readiness() -> TestResult {
    let engine = SyntheticEngine::new(Mode::Unavailable);
    let calls = Arc::clone(&engine.calls);
    let client = client(engine)?;
    for _ in 0..2 {
        assert_eq!(
            client
                .decide(Consumer::Triage, "route.file", 1, &input(), None)
                .await,
            PolicyOutcome::FailedOpen(FailureReason::Unavailable)
        );
    }
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::CircuitOpen)
    );
    assert_eq!(calls.load(Ordering::Acquire), 2);
    client.report_worker_readiness(true)?;
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::Unavailable)
    );
    assert_eq!(calls.load(Ordering::Acquire), 3);
    Ok(())
}

#[tokio::test]
async fn a_service_ready_probe_without_a_loaded_model_cannot_recover_the_circuit() -> TestResult {
    let engine = SyntheticEngine::new(Mode::Unavailable);
    let calls = Arc::clone(&engine.calls);
    let client = client(engine)?;
    for _ in 0..2 {
        assert_eq!(
            client
                .decide(Consumer::Triage, "route.file", 1, &input(), None)
                .await,
            PolicyOutcome::FailedOpen(FailureReason::Unavailable)
        );
    }
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::CircuitOpen)
    );
    assert!(!client.recover_if_model_ready(false)?);
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::CircuitOpen)
    );
    assert_eq!(calls.load(Ordering::Acquire), 2);
    assert!(client.recover_if_model_ready(true)?);
    assert!(!client.recover_if_model_ready(true)?);
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::Unavailable)
    );
    assert_eq!(calls.load(Ordering::Acquire), 3);
    Ok(())
}

#[tokio::test]
async fn repeated_model_ready_probe_does_not_fence_an_inflight_call() -> TestResult {
    let engine = SyntheticEngine::new(Mode::Wait);
    let entered = Arc::clone(&engine.entered);
    let release = Arc::clone(&engine.release);
    let policy = Arc::new(PolicyClient::new(
        engine,
        SchemaSet::load_builtin()?,
        PolicyConfig {
            max_deadline: Duration::from_secs(2),
            ..config()
        },
    )?);
    let owner = Arc::clone(&policy);
    let task = tokio::spawn(async move {
        owner
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), entered.notified()).await?;
    assert!(!policy.recover_if_model_ready(true)?);
    release.notify_one();
    assert!(matches!(task.await?, PolicyOutcome::Answered { .. }));
    Ok(())
}

#[tokio::test]
async fn retired_worker_response_cannot_cross_recovery_epoch() -> TestResult {
    let engine = SyntheticEngine::new(Mode::Wait);
    let entered = Arc::clone(&engine.entered);
    let release = Arc::clone(&engine.release);
    let client = Arc::new(client(engine)?);
    let owner = Arc::clone(&client);
    let task = tokio::spawn(async move {
        owner
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await
    });
    entered.notified().await;
    client.report_worker_readiness(false)?;
    client.report_worker_readiness(true)?;
    release.notify_one();
    assert_eq!(
        task.await?,
        PolicyOutcome::FailedOpen(FailureReason::WorkerChanged)
    );
    Ok(())
}
#[tokio::test]
async fn strict_uncalibrated_answer_still_needs_approval() -> TestResult {
    let client = client(SyntheticEngine::new(Mode::Answer))?;
    match client
        .decide(Consumer::SafetyGate, "safety.command", 1, &input(), None)
        .await
    {
        PolicyOutcome::Answered { verdicts, .. } => assert_eq!(
            verdicts,
            vec![(QuestionId::new("safe")?, Verdict::NeedsApproval)]
        ),
        other => return Err(format!("unexpected policy outcome: {other:?}").into()),
    }
    Ok(())
}
#[tokio::test]
async fn saturation_and_cancel_release_admission() -> TestResult {
    let engine = SyntheticEngine::new(Mode::Wait);
    let entered = Arc::clone(&engine.entered);
    let client = Arc::new(client(engine)?);
    let owner = Arc::clone(&client);
    let task = tokio::spawn(async move {
        owner
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await
    });
    entered.notified().await;
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::Saturated)
    );
    task.abort();
    let canceled = task.await;
    assert!(canceled.is_err());
    assert_eq!(
        client
            .decide(Consumer::Triage, "route.file", 1, &input(), None)
            .await,
        PolicyOutcome::FailedOpen(FailureReason::Timeout)
    );
    Ok(())
}
