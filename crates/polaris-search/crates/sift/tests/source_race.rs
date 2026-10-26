//! Deterministic fixtures exercise asynchronous scheduling, S1 outcomes and permit ownership.
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{
        Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer, QuestionId,
        QuestionKind,
    },
};
use polaris_search_sift::{
    KnowledgeSource, Locator, RaceMode, RaceOutcome, RacePolicy, RaceSource, RawChunk,
    SemaphoreSet, SiftConfig, SiftError, SiftQuery, SiftQuestions, Sifter, SourceId, SourcePool,
    Trust,
};
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
    calls: Arc<AtomicUsize>,
    unavailable: bool,
    completed_sources: Option<Arc<AtomicUsize>>,
}
impl DecisionEngine for Engine {
    async fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(completed) = &self.completed_sources {
            while completed.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        }
        if self.unavailable {
            return Err(DecisionError::Timeout);
        }
        let score = if input.text == "better" { 0.95 } else { 0.7 };
        let probability =
            Probability::new(score).map_err(|_| DecisionError::InvalidSchema("fixture".into()))?;
        let answers = schema
            .iter()
            .map(|question| QuestionAnswer {
                question: question.id.clone(),
                answer: match question.kind {
                    QuestionKind::Noul { .. } => Answer::Noul { yes: probability },
                    QuestionKind::Score { .. } => Answer::Score {
                        value: 5,
                        confidence: probability,
                    },
                    QuestionKind::Choice { .. } => Answer::Noul { yes: probability },
                },
            })
            .collect();
        Ok(Decision {
            answers,
            latency_us: 0,
        })
    }
}
struct Source {
    id: SourceId,
    text: String,
    delay: Duration,
    count: usize,
    duplicate: bool,
    completed: Option<Arc<AtomicUsize>>,
}
impl KnowledgeSource for Source {
    fn id(&self) -> &SourceId {
        &self.id
    }
    async fn search(&self, _query: &SiftQuery) -> Result<Vec<RawChunk>, SiftError> {
        tokio::time::sleep(self.delay).await;
        let chunks = (0..self.count)
            .map(|n| RawChunk {
                source: self.id.clone(),
                locator: Locator::Url(format!(
                    "https://example.test/{}/{}",
                    self.id.as_str(),
                    if self.duplicate { 0 } else { n }
                )),
                text: self.text.clone(),
                trust: Trust::Untrusted,
            })
            .collect();
        if let Some(completed) = &self.completed {
            completed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(chunks)
    }
}
fn source(
    id: &str,
    text: &str,
    delay: Duration,
    count: usize,
) -> Result<RaceSource<Source>, SiftError> {
    Ok(RaceSource {
        source: Arc::new(Source {
            id: SourceId::new(id)?,
            text: text.into(),
            delay,
            count,
            duplicate: false,
            completed: None,
        }),
        pool: SourcePool::Web,
    })
}
fn sifter(unavailable: bool) -> Result<(Sifter<Engine>, Arc<AtomicUsize>), Box<dyn Error>> {
    sifter_with_completion(unavailable, None)
}
fn sifter_with_completion(
    unavailable: bool,
    completed_sources: Option<Arc<AtomicUsize>>,
) -> Result<(Sifter<Engine>, Arc<AtomicUsize>), Box<dyn Error>> {
    let calls = Arc::new(AtomicUsize::new(0));
    let config = SiftConfig {
        questions: SiftQuestions {
            relevance: Question {
                id: QuestionId::new("relevance")?,
                kind: QuestionKind::Noul {
                    statement: "relevant".into(),
                },
            },
            density: Question {
                id: QuestionId::new("density")?,
                kind: QuestionKind::Score { min: 0, max: 10 },
            },
        },
        accept_relevance: Probability::new(0.6)?,
        max_candidates: 16,
        batch_size: 4,
        max_chunk_chars: 4096,
        decision_deadline: Duration::from_secs(1),
    };
    Ok((
        Sifter::new(
            Engine {
                calls: Arc::clone(&calls),
                unavailable,
                completed_sources,
            },
            config,
        )?,
        calls,
    ))
}
fn query() -> SiftQuery {
    SiftQuery {
        text: "better".into(),
        goal: "better".into(),
        max_chunks: 8,
    }
}
fn policy(mode: RaceMode) -> Result<RacePolicy, Box<dyn Error>> {
    Ok(RacePolicy {
        mode,
        deadline: Duration::from_secs(2),
        accept: Probability::new(0.6)?,
        max_validations: 8,
    })
}
#[tokio::test]
async fn first_accepted_aborts_and_releases_losers() -> Result<(), Box<dyn Error>> {
    let (sifter, _) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 2)?;
    let sources = vec![
        source("fast", "good", Duration::ZERO, 1)?,
        source("loser", "better", Duration::from_secs(60), 1)?,
    ];
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &policy(RaceMode::FirstAccepted)?,
            &CancellationToken::new(),
        )
        .await?;
    match result {
        RaceOutcome::Accepted { hit, aborted, .. } => {
            assert_eq!(hit.chunk.source.as_str(), "fast");
            assert_eq!(aborted, vec![SourceId::new("loser")?]);
        }
        _ => return Err("validated source was not accepted".into()),
    }
    assert_eq!(semaphores.web.available_permits(), 2);
    Ok(())
}
#[tokio::test]
async fn best_mode_waits_for_more_relevant_source() -> Result<(), Box<dyn Error>> {
    let (sifter, _) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 2)?;
    let sources = vec![
        source("fast", "good", Duration::ZERO, 1)?,
        source("slow", "better", Duration::from_millis(10), 1)?,
    ];
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &policy(RaceMode::BestWithinDeadline)?,
            &CancellationToken::new(),
        )
        .await?;
    match result {
        RaceOutcome::Accepted { hit, aborted, .. } => {
            assert_eq!(hit.chunk.source.as_str(), "slow");
            assert!(aborted.is_empty());
        }
        _ => return Err("best validated source was not accepted".into()),
    }
    assert_eq!(semaphores.web.available_permits(), 2);
    Ok(())
}
#[tokio::test]
async fn race_failure_preserves_untrusted_fallback() -> Result<(), Box<dyn Error>> {
    let (sifter, _) = sifter(true)?;
    let semaphores = SemaphoreSet::new(1, 1, 1)?;
    let sources = vec![source("web", "better", Duration::ZERO, 1)?];
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &policy(RaceMode::BestWithinDeadline)?,
            &CancellationToken::new(),
        )
        .await?;
    match result {
        RaceOutcome::FailedOpen { fallback, .. } => {
            let hit = fallback.first().ok_or("missing fallback")?;
            assert_eq!(hit.source.as_str(), "web");
            assert_eq!(hit.trust, Trust::Untrusted);
            assert_eq!(hit.text, "better");
        }
        _ => return Err("S1 failure was incorrectly accepted".into()),
    }
    Ok(())
}
#[tokio::test]
async fn race_cancellation_and_validation_budget_are_enforced() -> Result<(), Box<dyn Error>> {
    let (sifter, calls) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 1)?;
    let sources = vec![source("web", "better", Duration::ZERO, 8)?];
    let mut limits = policy(RaceMode::BestWithinDeadline)?;
    limits.max_validations = 2;
    let _outcome = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &limits,
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = sifter
        .race(&query(), &sources, &semaphores, &limits, &cancellation)
        .await;
    assert!(matches!(result, Err(SiftError::Cancelled)));
    assert_eq!(semaphores.web.available_permits(), 1);
    Ok(())
}

#[tokio::test]
async fn completed_losers_are_retrieved_instead_of_cancelled() -> Result<(), Box<dyn Error>> {
    // This current-thread runtime cannot observe a source's completion counter before its
    // search future returns: there is no await between the increment and returning the value.
    // S1 waits for both counters, so the second result is queued before selection can accept.
    let completed = Arc::new(AtomicUsize::new(0));
    let (sifter, _) = sifter_with_completion(false, Some(Arc::clone(&completed)))?;
    let semaphores = SemaphoreSet::new(1, 1, 2)?;
    let mut sources = vec![
        source("fast", "good", Duration::ZERO, 1)?,
        source("queued", "better", Duration::ZERO, 1)?,
    ];
    for entry in &mut sources {
        Arc::get_mut(&mut entry.source)
            .ok_or("fixture source shared too early")?
            .completed = Some(Arc::clone(&completed));
    }
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &policy(RaceMode::FirstAccepted)?,
            &CancellationToken::new(),
        )
        .await?;
    match result {
        RaceOutcome::Accepted {
            aborted, sources, ..
        } => {
            assert!(aborted.is_empty());
            assert_eq!(sources.len(), 2);
            assert!(
                sources
                    .iter()
                    .all(|source| source.status == polaris_search_sift::SourceStatus::Retrieved)
            );
        }
        _ => return Err("missing acceptance".into()),
    }
    Ok(())
}
#[tokio::test]
async fn duplicate_evidence_is_validated_only_once() -> Result<(), Box<dyn Error>> {
    let (sifter, calls) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 1)?;
    let mut entry = source("duplicates", "good", Duration::ZERO, 8)?;
    Arc::get_mut(&mut entry.source)
        .ok_or("fixture already shared")?
        .duplicate = true;
    let _outcome = sifter
        .race(
            &query(),
            &[entry],
            &semaphores,
            &policy(RaceMode::BestWithinDeadline)?,
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}
#[tokio::test]
async fn cancellation_after_permit_acquisition_releases_it() -> Result<(), Box<dyn Error>> {
    let (sifter, _) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 1)?;
    let sources = vec![source("blocked", "good", Duration::from_secs(60), 1)?];
    let cancellation = CancellationToken::new();
    let limits = policy(RaceMode::BestWithinDeadline)?;
    let query = query();
    let cancel_when_running = async {
        while semaphores.web.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        cancellation.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(
            sifter.race(&query, &sources, &semaphores, &limits, &cancellation),
            cancel_when_running
        )
    })
    .await?;
    assert!(matches!(result, Err(SiftError::Cancelled)));
    assert_eq!(semaphores.web.available_permits(), 1);
    Ok(())
}
#[tokio::test]
async fn deadline_and_closed_source_pool_have_explicit_outcomes() -> Result<(), Box<dyn Error>> {
    let (sifter, _) = sifter(false)?;
    let semaphores = SemaphoreSet::new(1, 1, 1)?;
    let sources = vec![source("blocked", "good", Duration::from_secs(60), 1)?];
    let mut limits = policy(RaceMode::BestWithinDeadline)?;
    limits.deadline = Duration::from_millis(10);
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &limits,
            &CancellationToken::new(),
        )
        .await?;
    assert!(matches!(
        result,
        RaceOutcome::NoneSufficient { best: None, .. }
    ));
    assert_eq!(semaphores.web.available_permits(), 1);
    semaphores.web.close();
    let result = sifter
        .race(
            &query(),
            &sources,
            &semaphores,
            &limits,
            &CancellationToken::new(),
        )
        .await?;
    assert!(matches!(result, RaceOutcome::FailedOpen { fallback, .. } if fallback.is_empty()));
    Ok(())
}
