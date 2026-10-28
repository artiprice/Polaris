//! Synthetic malformed answers exercise workload accounting, not model quality.
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{
        Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
        QuestionId, QuestionKind,
    },
};
use polaris_spike_s1::{BurstConfig, BurstRequest, RequestOutcome, WorkloadError, run_burst};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
type TestResult = Result<(), Box<dyn Error>>;
struct Engine {
    answer: Answer,
    calls: Arc<AtomicUsize>,
}
impl DecisionEngine for Engine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Ok(Decision {
            answers: questions
                .iter()
                .map(|question| QuestionAnswer {
                    question: question.id.clone(),
                    answer: self.answer.clone(),
                })
                .collect(),
            latency_us: 0,
        })
    }
}
fn config() -> BurstConfig {
    BurstConfig {
        max_owned_tasks: 1,
        request_timeout: Duration::from_secs(1),
    }
}
fn request(kind: QuestionKind) -> Result<BurstRequest, Box<dyn Error>> {
    Ok(BurstRequest {
        questions: vec![Question {
            id: QuestionId::new("decision")?,
            kind,
        }],
        input: DecisionInput {
            text: "masked input".into(),
            context: None,
        },
    })
}
fn score(label: &str, value: f32) -> Result<LabelScore, Box<dyn Error>> {
    Ok(LabelScore {
        label: Label::new(label)?,
        probability: Probability::new(value)?,
    })
}
#[tokio::test]
async fn out_of_range_scores_are_not_success() -> TestResult {
    let engine = Arc::new(Engine {
        answer: Answer::Score {
            value: 6,
            confidence: Probability::new(1.0)?,
        },
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let report = run_burst(
        engine,
        vec![request(QuestionKind::Score { min: 1, max: 5 })?],
        config(),
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(report.succeeded(), 0);
    assert_eq!(
        report.records.first().ok_or("missing record")?.outcome,
        RequestOutcome::InvalidDecision
    );
    Ok(())
}
#[tokio::test]
async fn malformed_choice_distributions_are_not_success() -> TestResult {
    let cases = vec![
        ("a", vec![score("a", 0.5)?, score("a", 0.5)?]),
        ("a", vec![score("a", 0.7)?, score("foreign", 0.3)?]),
        ("a", vec![score("a", 1.0)?]),
        ("a", vec![score("a", 0.2)?, score("b", 0.1)?]),
        ("b", vec![score("a", 0.8)?, score("b", 0.2)?]),
        ("a", vec![score("b", 0.2)?, score("a", 0.8)?]),
    ];
    for (selected, probs) in cases {
        let engine = Arc::new(Engine {
            answer: Answer::Choice {
                selected: Label::new(selected)?,
                probs,
            },
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let report = run_burst(
            engine,
            vec![request(QuestionKind::Choice {
                options: vec![Label::new("a")?, Label::new("b")?],
            })?],
            config(),
            &CancellationToken::new(),
        )
        .await?;
        assert_eq!(
            report.succeeded(),
            0,
            "malformed distributions must not count as answers"
        );
        assert_eq!(
            report.records.first().ok_or("missing record")?.outcome,
            RequestOutcome::InvalidDecision
        );
    }
    Ok(())
}
#[tokio::test]
async fn duplicate_choice_options_reject_before_dispatch() -> TestResult {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = Arc::new(Engine {
        answer: Answer::Choice {
            selected: Label::new("a")?,
            probs: vec![score("a", 1.0)?],
        },
        calls: Arc::clone(&calls),
    });
    let result = run_burst(
        engine,
        vec![request(QuestionKind::Choice {
            options: vec![Label::new("a")?, Label::new("a")?],
        })?],
        config(),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(WorkloadError::Bounds)));
    assert_eq!(calls.load(Ordering::Acquire), 0);
    Ok(())
}
