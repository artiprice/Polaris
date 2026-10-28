//! Synthetic engine scheduling contracts; production workload dispatch is shared with S1Client.
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{
        Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer, QuestionId,
        QuestionKind,
    },
};
use polaris_spike_s1::*;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
struct Engine {
    active: Arc<AtomicUsize>,
    delay: Duration,
    invalid: bool,
}
struct ActiveCall(Arc<AtomicUsize>);
impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl DecisionEngine for Engine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.active.fetch_add(1, Ordering::AcqRel);
        let _active = ActiveCall(Arc::clone(&self.active));
        tokio::time::sleep(self.delay).await;
        let mut answers = Vec::new();
        if !self.invalid {
            for question in questions {
                answers.push(QuestionAnswer {
                    question: question.id.clone(),
                    answer: Answer::Noul {
                        yes: Probability::new(0.9)
                            .map_err(|_| DecisionError::Model("fixture probability".into()))?,
                    },
                });
            }
        }
        Ok(Decision {
            answers,
            latency_us: 0,
        })
    }
}
fn requests(count: usize) -> Result<Vec<BurstRequest>, Box<dyn Error>> {
    (0..count)
        .map(|_| {
            Ok(BurstRequest {
                questions: vec![Question {
                    id: QuestionId::new("relevant")?,
                    kind: QuestionKind::Noul {
                        statement: "The candidate is relevant.".into(),
                    },
                }],
                input: DecisionInput {
                    text: "masked candidate".into(),
                    context: None,
                },
            })
        })
        .collect()
}
fn engine(delay: Duration, invalid: bool) -> Arc<Engine> {
    Arc::new(Engine {
        active: Arc::new(AtomicUsize::new(0)),
        delay,
        invalid,
    })
}
fn bounds() -> BurstConfig {
    BurstConfig {
        max_owned_tasks: 4,
        request_timeout: Duration::from_secs(1),
    }
}
#[tokio::test]
async fn burst_records_observed_calls_and_latency() -> Result<(), Box<dyn Error>> {
    let engine = engine(Duration::from_millis(2), false);
    let report = run_burst(
        Arc::clone(&engine),
        requests(50)?,
        bounds(),
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(report.requested, 50);
    assert_eq!(report.succeeded(), 50);
    assert_eq!(report.records.len(), 50);
    assert!(report.peak_owned_tasks <= 4);
    assert!(report.p95_latency.is_some());
    for (expected, record) in report.records.iter().enumerate() {
        assert_eq!(record.index, expected);
        assert_eq!(record.outcome, RequestOutcome::Answered);
        assert!(record.latency.is_some());
    }
    assert_eq!(engine.active.load(Ordering::Acquire), 0);
    Ok(())
}
#[tokio::test]
async fn timeouts_are_measured_and_not_answers() -> Result<(), Box<dyn Error>> {
    let engine = engine(Duration::from_secs(10), false);
    let mut config = bounds();
    config.request_timeout = Duration::from_millis(10);
    let report = run_burst(
        Arc::clone(&engine),
        requests(8)?,
        config,
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(report.succeeded(), 0);
    assert!(
        report
            .records
            .iter()
            .all(|r| r.outcome == RequestOutcome::Timeout && r.latency.is_some())
    );
    assert_eq!(engine.active.load(Ordering::Acquire), 0);
    Ok(())
}
#[tokio::test]
async fn cancellation_drains_every_owned_task() -> Result<(), Box<dyn Error>> {
    let engine = engine(Duration::from_secs(10), false);
    let owner = Arc::clone(&engine);
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let requests = requests(50)?;
    let task =
        tokio::spawn(async move { run_burst(owner, requests, bounds(), &task_cancel).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while engine.active.load(Ordering::Acquire) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    cancel.cancel();
    let report = task.await??;
    assert_eq!(report.records.len(), 50);
    assert!(
        report
            .records
            .iter()
            .all(|r| r.outcome == RequestOutcome::Cancelled)
    );
    assert_eq!(engine.active.load(Ordering::Acquire), 0);
    assert!(report.p95_latency.is_none());
    Ok(())
}
#[tokio::test]
async fn malformed_engine_results_are_not_success() -> Result<(), Box<dyn Error>> {
    let report = run_burst(
        engine(Duration::ZERO, true),
        requests(3)?,
        bounds(),
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(report.succeeded(), 0);
    assert!(
        report
            .records
            .iter()
            .all(|r| r.outcome == RequestOutcome::InvalidDecision)
    );
    Ok(())
}
