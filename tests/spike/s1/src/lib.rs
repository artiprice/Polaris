//! Bounded, owned S1 burst measurements over actual injected decision engines.
//!
//! No model-quality or recovery assertion is inferred from a transport success. Reports retain
//! only request indexes, sanitized outcomes and observed timings; masked input and answers are
//! not copied to diagnostics. To exercise real IPC, inject the authenticated S1Client.
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{Answer, Decision, DecisionInput, Question, QuestionKind},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{task::JoinSet, time::Instant};
use tokio_util::sync::CancellationToken;

/// One caller-supplied, already-masked real decision request.
#[derive(Debug, Clone)]
pub struct BurstRequest {
    /// Exact question set sent to the actual engine.
    pub questions: Vec<Question>,
    /// Masked primary and optional context input.
    pub input: DecisionInput,
}
/// Bounded concurrency and total deadline per individual engine future.
#[derive(Debug, Clone, Copy)]
pub struct BurstConfig {
    /// Maximum retained child tasks, including completed tasks awaiting collection (1..=1024).
    pub max_owned_tasks: usize,
    /// Individual request deadline, at most 120 seconds.
    pub request_timeout: Duration,
}
/// Sanitized terminal status of a workload request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// Engine returned the exact question set and expected answer kinds.
    Answered,
    /// The request deadline expired; the owned engine future was dropped.
    Timeout,
    /// Engine reported missing/restarting capacity.
    Unavailable,
    /// The engine rejected the request schema.
    InvalidSchema,
    /// Native/provider inference reported a failure.
    ModelFailure,
    /// Engine result had a foreign, missing, duplicate or wrong-kind question answer.
    InvalidDecision,
    /// Parent cancellation ended this request or prevented its submission.
    Cancelled,
}
/// One actual request observation. No request text or model output is retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    /// Original caller order, starting at zero.
    pub index: usize,
    /// Sanitized observed result.
    pub outcome: RequestOutcome,
    /// Observed duration for a collected call; absent when cancellation prevented collection.
    pub latency: Option<Duration>,
}
/// Actual bounded run report; quantiles cover collected engine calls, including errors/timeouts.
#[derive(Debug, Clone)]
pub struct BurstReport {
    /// Number of original requests, including those canceled before submission.
    pub requested: usize,
    /// Maximum number of owned task handles observed during the run.
    pub peak_owned_tasks: usize,
    /// Actual monotonic run duration including cancellation drain.
    pub elapsed: Duration,
    /// Per-request records in original order, exactly one per supplied request.
    pub records: Vec<RequestRecord>,
    /// Measured p50 duration, absent when no call completed collection.
    pub p50_latency: Option<Duration>,
    /// Measured p95 duration, absent when no call completed collection.
    pub p95_latency: Option<Duration>,
    /// Measured p99 duration, absent when no call completed collection.
    pub p99_latency: Option<Duration>,
}
impl BurstReport {
    /// Count actual validated engine successes, excluding all fallback/error outcomes.
    #[must_use]
    pub fn succeeded(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.outcome == RequestOutcome::Answered)
            .count()
    }
}
/// Workload configuration, accounting, task or histogram failures; no backend error text leaks.
#[derive(Debug, thiserror::Error)]
pub enum WorkloadError {
    /// Unsupported or zero processing bounds.
    #[error("invalid S1 workload bounds")]
    Bounds,
    /// A child task failed independently of the engine's typed error path.
    #[error("S1 workload task failed")]
    Task,
    /// A count/latency could not be represented or inserted in its bounded histogram.
    #[error("S1 workload accounting failed")]
    Accounting,
}
/// Dispatch a burst with bounded owned tasks; canceling through the supplied token aborts and
/// joins every child before returning a report. Dropping this future aborts its JoinSet; synchronous
/// Drop cannot prove the runtime has completed async task destruction, so use the token and await.
///
/// # Errors
/// Invalid request/config bounds, a failed task, or an unrepresentable measurement. All supplied
/// request data is bounded before the first engine dispatch. Backend errors become report records.
pub async fn run_burst<D: DecisionEngine + 'static>(
    engine: Arc<D>,
    requests: Vec<BurstRequest>,
    config: BurstConfig,
    cancellation: &CancellationToken,
) -> Result<BurstReport, WorkloadError> {
    validate(&requests, config)?;
    let requested = requests.len();
    let started = Instant::now();
    let mut peak = 0;
    let mut remaining = requests.into_iter().enumerate();
    let mut tasks = JoinSet::new();
    let mut records = BTreeMap::new();
    loop {
        if cancellation.is_cancelled() {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            break;
        }
        while tasks.len() < config.max_owned_tasks {
            let Some((index, request)) = remaining.next() else {
                break;
            };
            let engine = Arc::clone(&engine);
            tasks.spawn(async move {
                let started = Instant::now();
                let outcome = match tokio::time::timeout(
                    config.request_timeout,
                    engine.decide(&request.questions, &request.input),
                )
                .await
                {
                    Err(_) | Ok(Err(DecisionError::Timeout)) => RequestOutcome::Timeout,
                    Ok(Err(DecisionError::Unavailable(_))) => RequestOutcome::Unavailable,
                    Ok(Err(DecisionError::InvalidSchema(_))) => RequestOutcome::InvalidSchema,
                    Ok(Err(DecisionError::Model(_))) => RequestOutcome::ModelFailure,
                    Ok(Ok(decision)) => {
                        if validate_burst_decision(&request.questions, &decision) {
                            RequestOutcome::Answered
                        } else {
                            RequestOutcome::InvalidDecision
                        }
                    }
                };
                RequestRecord {
                    index,
                    outcome,
                    latency: Some(started.elapsed()),
                }
            });
            peak = peak.max(tasks.len());
        }
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            ()=cancellation.cancelled()=>{tasks.abort_all();while tasks.join_next().await.is_some(){}break;},
            joined=tasks.join_next()=>match joined {
                Some(Ok(record))=>{if records.insert(record.index,record).is_some(){tasks.abort_all();while tasks.join_next().await.is_some(){}return Err(WorkloadError::Accounting);}},
                Some(Err(_))=>{tasks.abort_all();while tasks.join_next().await.is_some(){}return Err(WorkloadError::Task);},
                None=>break,
            }
        }
    }
    let mut histogram =
        hdrhistogram::Histogram::<u64>::new(3).map_err(|_| WorkloadError::Accounting)?;
    let mut ordered = Vec::with_capacity(requested);
    for index in 0..requested {
        let record = match records.remove(&index) {
            Some(record) => record,
            None => RequestRecord {
                index,
                outcome: RequestOutcome::Cancelled,
                latency: None,
            },
        };
        if let Some(duration) = record.latency {
            // Histogram values are nanoseconds; exact zero samples use the histogram's minimum
            // positive bin. No duration or provider latency is fabricated for canceled requests.
            let nanos = u64::try_from(duration.as_nanos())
                .map_err(|_| WorkloadError::Accounting)?
                .max(1);
            histogram
                .record(nanos)
                .map_err(|_| WorkloadError::Accounting)?;
        }
        ordered.push(record);
    }
    let quantile = |q| {
        if histogram.is_empty() {
            None
        } else {
            Some(Duration::from_nanos(histogram.value_at_quantile(q)))
        }
    };
    Ok(BurstReport {
        requested,
        peak_owned_tasks: peak,
        elapsed: started.elapsed(),
        records: ordered,
        p50_latency: quantile(0.5),
        p95_latency: quantile(0.95),
        p99_latency: quantile(0.99),
    })
}
fn validate_burst_decision(questions: &[Question], decision: &Decision) -> bool {
    decision.answers.len() == questions.len()
        && questions
            .iter()
            .zip(&decision.answers)
            .all(|(question, answer)| {
                if question.id != answer.question {
                    return false;
                }
                match (&question.kind, &answer.answer) {
                    (QuestionKind::Score { min, max }, Answer::Score { value, .. }) => {
                        (min..=max).contains(&value)
                    }
                    (QuestionKind::Noul { .. }, Answer::Noul { .. }) => true,
                    (QuestionKind::Choice { options }, Answer::Choice { selected, probs }) => {
                        if probs.len() != options.len() || !options.contains(selected) {
                            return false;
                        }
                        let labels = probs
                            .iter()
                            .map(|score| &score.label)
                            .collect::<std::collections::BTreeSet<_>>();
                        if labels.len() != options.len()
                            || options.iter().any(|option| !labels.contains(option))
                        {
                            return false;
                        }
                        if probs
                            .iter()
                            .zip(probs.iter().skip(1))
                            .any(|(left, right)| left.probability < right.probability)
                        {
                            return false;
                        }
                        let total = probs
                            .iter()
                            .map(|score| f64::from(score.probability.get()))
                            .sum::<f64>();
                        if !(0.999..=1.001).contains(&total) {
                            return false;
                        }
                        let maximum = probs.first().map(|score| score.probability);
                        probs
                            .iter()
                            .find(|score| score.label == *selected)
                            .is_some_and(|score| Some(score.probability) == maximum)
                    }
                    (QuestionKind::Choice { .. }, _)
                    | (QuestionKind::Score { .. }, _)
                    | (QuestionKind::Noul { .. }, _) => false,
                }
            })
}
fn validate(requests: &[BurstRequest], config: BurstConfig) -> Result<(), WorkloadError> {
    if requests.is_empty()
        || requests.len() > 65_536
        || config.max_owned_tasks == 0
        || config.max_owned_tasks > 1024
        || config.request_timeout.is_zero()
        || config.request_timeout > Duration::from_secs(120)
    {
        return Err(WorkloadError::Bounds);
    }
    let mut aggregate = 0_usize;
    for request in requests {
        let context = request.input.context.as_ref().map_or(0, String::len);
        let mut bytes = request
            .input
            .text
            .len()
            .checked_add(context)
            .ok_or(WorkloadError::Bounds)?;
        if bytes > 262_144 || request.questions.is_empty() || request.questions.len() > 64 {
            return Err(WorkloadError::Bounds);
        }
        let mut ids = std::collections::BTreeSet::new();
        for question in &request.questions {
            if !ids.insert(&question.id) {
                return Err(WorkloadError::Bounds);
            }
            bytes = bytes
                .checked_add(question.id.as_str().len())
                .ok_or(WorkloadError::Bounds)?;
            let extra = match &question.kind {
                QuestionKind::Noul { statement } if statement.len() <= 4096 => statement.len(),
                QuestionKind::Choice { options } if !options.is_empty() && options.len() <= 128 => {
                    let unique = options.iter().collect::<std::collections::BTreeSet<_>>();
                    if unique.len() != options.len() {
                        return Err(WorkloadError::Bounds);
                    }
                    options
                        .iter()
                        .try_fold(0_usize, |sum, label| sum.checked_add(label.as_str().len()))
                        .ok_or(WorkloadError::Bounds)?
                }
                QuestionKind::Score { min, max } if min <= max => 0,
                QuestionKind::Noul { .. }
                | QuestionKind::Choice { .. }
                | QuestionKind::Score { .. } => return Err(WorkloadError::Bounds),
            };
            bytes = bytes.checked_add(extra).ok_or(WorkloadError::Bounds)?;
        }
        aggregate = aggregate.checked_add(bytes).ok_or(WorkloadError::Bounds)?;
        if aggregate > 67_108_864 {
            return Err(WorkloadError::Bounds);
        }
    }
    Ok(())
}
