//! Cold retrieval orchestration. Aborting a source cancels futures, not external side effects.
use crate::{Locator, RankedChunk, RawChunk, ScoreBasis, SiftError, Sifter, SourceId, Trust};
use polaris_sdk::{
    registry::{DecisionEngine, SearchQuery, WebSearch},
    system_one::Probability,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::Semaphore, task::JoinSet};
use tokio_util::sync::CancellationToken;

const MAX_SOURCES: usize = 32;
const MAX_SOURCE_BYTES: usize = 1_048_576;

/// Retrieval input, bounded before source tasks are started.
#[derive(Debug, Clone)]
pub struct SiftQuery {
    /// Search text sent through source-specific redaction before egress.
    pub text: String,
    /// Goal supplied to the relevance engine.
    pub goal: String,
    /// Maximum chunks accepted from each source, 1–1024.
    pub max_chunks: u16,
}
/// Source adapter. Implementations must yield at cancellation points and bound their own I/O.
pub trait KnowledgeSource: Send + Sync {
    /// Stable configured identity; returned chunks must carry this identity.
    fn id(&self) -> &SourceId;
    /// Retrieve data without granting it authority or changing its provenance.
    /// # Errors
    /// Source-specific failures, represented without credentials or remote error bodies.
    fn search(
        &self,
        query: &SiftQuery,
    ) -> impl Future<Output = Result<Vec<RawChunk>, SiftError>> + Send;
}
/// Configured concurrency pool for a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePool {
    /// Curated documentation retrieval.
    Docs,
    /// Local indexed retrieval.
    Rag,
    /// Remote web retrieval.
    Web,
}
/// One source and the named concurrency budget charged by its retrieval.
#[derive(Debug)]
pub struct RaceSource<S> {
    /// Adapter instance. A composition-root enum can dispatch heterogeneous adapters.
    pub source: Arc<S>,
    /// Pool shared with concurrent races.
    pub pool: SourcePool,
}
/// Explicit shared budgets, with no assumed provider quotas.
#[derive(Debug, Clone)]
pub struct SemaphoreSet {
    /// Documentation permits.
    pub docs: Arc<Semaphore>,
    /// Local retrieval permits.
    pub rag: Arc<Semaphore>,
    /// Web retrieval permits.
    pub web: Arc<Semaphore>,
}
impl SemaphoreSet {
    /// Create pools with 1–2048 permits each.
    /// # Errors
    /// A zero or excessive pool size.
    pub fn new(docs: usize, rag: usize, web: usize) -> Result<Self, SiftError> {
        if [docs, rag, web]
            .iter()
            .any(|size| !(1..=2048).contains(size))
        {
            return Err(invalid("invalid source concurrency"));
        }
        Ok(Self {
            docs: Arc::new(Semaphore::new(docs)),
            rag: Arc::new(Semaphore::new(rag)),
            web: Arc::new(Semaphore::new(web)),
        })
    }
    fn pool(&self, pool: SourcePool) -> Arc<Semaphore> {
        Arc::clone(match pool {
            SourcePool::Docs => &self.docs,
            SourcePool::Rag => &self.rag,
            SourcePool::Web => &self.web,
        })
    }
}
/// Retrieval acceptance strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceMode {
    /// Cancel unconsumed tasks after the first independently S1-accepted hit.
    FirstAccepted,
    /// Gather until completion or deadline, selecting the highest validated score.
    BestWithinDeadline,
}
/// Caller-supplied ranking and latency policy; scores do not imply calibration.
#[derive(Debug, Clone, Copy)]
pub struct RacePolicy {
    /// Acceptance strategy.
    pub mode: RaceMode,
    /// Selection deadline, including semaphore waits and validation, at most 120 s.
    /// Cooperative source teardown is drained afterwards; blocking code can delay return.
    pub deadline: Duration,
    /// Minimum S1 relevance probability, supplied by caller policy.
    pub accept: Probability,
    /// Maximum candidates sent to S1 across all sources, 1–4096.
    pub max_validations: usize,
}
/// Retrieval status, without remote error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    /// Retrieval returned a valid bounded list, including an empty list.
    Retrieved,
    /// Adapter failed, returned invalid provenance, or its task failed.
    Failed,
    /// Unconsumed source task was cancelled at selection or deadline.
    Cancelled,
}
/// One configured source's observed retrieval outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    /// Stable source identity.
    pub source: SourceId,
    /// Observed status.
    pub status: SourceStatus,
}
/// A search result never promotes lexical fallback into S1 acceptance.
#[derive(Debug, Clone)]
pub enum RaceOutcome {
    /// At least one S1-validated hit met caller policy.
    Accepted {
        /// Best validated hit observed under the selected strategy.
        hit: RankedChunk,
        /// All bounded retrieved chunks, including unvalidated candidates and completed losers.
        retrieved: Vec<RawChunk>,
        /// Tasks whose cancellation was acknowledged by the task runtime and drained.
        aborted: Vec<SourceId>,
        /// Retrieval outcomes in stable source order.
        sources: Vec<SourceReport>,
    },
    /// No validated score met policy; deadline expiry alone does not assert relevance.
    NoneSufficient {
        /// Best S1-validated hit, when available.
        best: Option<RankedChunk>,
        /// All bounded retrieved chunks, without asserting S1 acceptance.
        retrieved: Vec<RawChunk>,
        /// Retrieval outcomes.
        sources: Vec<SourceReport>,
    },
    /// No hit was accepted, and S1/source failure or validation limits left uncertainty.
    FailedOpen {
        /// Original bounded chunks with unchanged text, trust and provenance.
        fallback: Vec<RawChunk>,
        /// Retrieval outcomes.
        sources: Vec<SourceReport>,
    },
}
fn invalid(message: &str) -> SiftError {
    SiftError::InvalidConfig(message.into())
}

impl<D: DecisionEngine> Sifter<D> {
    /// Race configured sources, validate within a shared cap, and drain all cancelled tasks.
    /// The deadline includes S1 work. Blocking source work cannot be forcibly interrupted.
    /// # Errors
    /// Invalid bounds, duplicate source identities, cancellation, or invalid ranking policy.
    pub async fn race<S: KnowledgeSource + 'static>(
        &self,
        query: &SiftQuery,
        sources: &[RaceSource<S>],
        semaphores: &SemaphoreSet,
        policy: &RacePolicy,
        cancellation: &CancellationToken,
    ) -> Result<RaceOutcome, SiftError> {
        if sources.is_empty()
            || sources.len() > MAX_SOURCES
            || query.text.trim().is_empty()
            || query.text.len() > 16_384
            || query.goal.len() > 65_536
            || !(1..=1024).contains(&query.max_chunks)
            || policy.deadline.is_zero()
            || policy.deadline > Duration::from_secs(120)
            || !(1..=4096).contains(&policy.max_validations)
        {
            return Err(invalid("invalid search race bounds"));
        }
        let mut pending = BTreeSet::new();
        for entry in sources {
            if entry.source.id().as_str().len() > 256 || !pending.insert(entry.source.id().clone())
            {
                return Err(invalid("duplicate source identity"));
            }
        }
        if cancellation.is_cancelled() {
            return Err(SiftError::Cancelled);
        }
        let deadline = tokio::time::Instant::now() + policy.deadline;
        let mut tasks = JoinSet::new();
        let mut task_sources = HashMap::new();
        for entry in sources {
            let source = Arc::clone(&entry.source);
            let source_id = source.id().clone();
            let query = query.clone();
            let semaphore = semaphores.pool(entry.pool);
            let handle = tasks.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|_| invalid("source pool closed"))?;
                source.search(&query).await
            });
            task_sources.insert(handle.id(), source_id);
        }
        let mut reports = Vec::with_capacity(sources.len());
        let mut fallback = Vec::new();
        let mut best: Option<RankedChunk> = None;
        let mut validations = 0_usize;
        let mut failed_open = false;
        while !tasks.is_empty() {
            let joined = tokio::select! {
                biased;
                () = cancellation.cancelled() => { drain(&mut tasks, &mut task_sources, &mut reports, &mut fallback, query.max_chunks).await?; return Err(SiftError::Cancelled); },
                () = tokio::time::sleep_until(deadline) => break,
                joined = tasks.join_next_with_id() => joined,
            };
            let Some(joined) = joined else {
                break;
            };
            let (task_id, result) = match joined {
                Ok((id, result)) => (id, result),
                Err(error) => (error.id(), Err(invalid("source task failed"))),
            };
            let id = task_sources
                .remove(&task_id)
                .ok_or_else(|| invalid("unknown source task"))?;
            let chunks = match result {
                Ok(chunks) => validated_chunks(chunks, &id, query.max_chunks),
                Err(_) => None,
            };
            let Some(chunks) = chunks else {
                reports.push(SourceReport {
                    source: id,
                    status: SourceStatus::Failed,
                });
                failed_open = true;
                continue;
            };
            reports.push(SourceReport {
                source: id,
                status: SourceStatus::Retrieved,
            });
            let available = policy
                .max_validations
                .saturating_sub(validations)
                .min(self.config.max_candidates);
            let considered = chunks.len().min(available);
            validations = validations
                .checked_add(considered)
                .ok_or_else(|| invalid("validation count overflow"))?;
            if considered < chunks.len() {
                failed_open = true;
            }
            fallback.extend(chunks.iter().cloned());
            if considered == 0 {
                continue;
            }
            let candidates = chunks
                .get(..considered)
                .ok_or_else(|| invalid("invalid candidate range"))?;
            let ranked = tokio::select! {
                biased;
                () = cancellation.cancelled() => { drain(&mut tasks, &mut task_sources, &mut reports, &mut fallback, query.max_chunks).await?; return Err(SiftError::Cancelled); },
                () = tokio::time::sleep_until(deadline) => { failed_open = true; break; },
                ranked = self.rank_chunks(&query.goal, candidates) => ranked?,
            };
            for hit in ranked {
                if hit.basis != ScoreBasis::S1 {
                    failed_open = true;
                    continue;
                }
                if best.as_ref().is_none_or(|current| better(&hit, current)) {
                    best = Some(hit);
                }
            }
            if policy.mode == RaceMode::FirstAccepted
                && best
                    .as_ref()
                    .is_some_and(|hit| hit.relevance >= policy.accept)
            {
                break;
            }
        }
        let aborted = drain(
            &mut tasks,
            &mut task_sources,
            &mut reports,
            &mut fallback,
            query.max_chunks,
        )
        .await?;
        failed_open |= reports
            .iter()
            .any(|report| report.status == SourceStatus::Failed);
        failed_open |= fallback.len() > validations;
        reports.sort_by(|left, right| left.source.cmp(&right.source));
        fallback.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.locator.cmp(&right.locator))
        });
        if let Some(hit) = best.as_ref().filter(|hit| hit.relevance >= policy.accept) {
            return Ok(RaceOutcome::Accepted {
                hit: hit.clone(),
                retrieved: fallback,
                aborted,
                sources: reports,
            });
        }
        if failed_open {
            fallback.sort_by(|left, right| {
                left.source
                    .cmp(&right.source)
                    .then_with(|| left.locator.cmp(&right.locator))
            });
            return Ok(RaceOutcome::FailedOpen {
                fallback,
                sources: reports,
            });
        }
        Ok(RaceOutcome::NoneSufficient {
            best,
            retrieved: fallback,
            sources: reports,
        })
    }
}
fn better(candidate: &RankedChunk, current: &RankedChunk) -> bool {
    candidate.relevance > current.relevance
        || (candidate.relevance == current.relevance
            && (candidate.density > current.density
                || (candidate.density == current.density
                    && (&candidate.chunk.source, &candidate.chunk.locator)
                        < (&current.chunk.source, &current.chunk.locator))))
}
fn validated_chunks(chunks: Vec<RawChunk>, source: &SourceId, cap: u16) -> Option<Vec<RawChunk>> {
    if chunks.len() > usize::from(cap) {
        return None;
    }
    let mut bytes = 0_usize;
    let mut identities = BTreeMap::new();
    let mut unique: Vec<RawChunk> = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if &chunk.source != source {
            return None;
        }
        let locator_bytes = match &chunk.locator {
            Locator::Url(url) => url.len(),
            Locator::Document { id, .. } => id.len(),
            Locator::Symbol { path, name } => path.len().saturating_add(name.len()),
        };
        bytes = bytes
            .saturating_add(chunk.text.len())
            .saturating_add(locator_bytes)
            .saturating_add(chunk.source.as_str().len());
        if locator_bytes > 8192 || bytes > MAX_SOURCE_BYTES {
            return None;
        }
        if let Some(position) = identities.get(&chunk.locator) {
            if unique.get(*position)? != &chunk {
                return None;
            }
            continue;
        }
        identities.insert(chunk.locator.clone(), unique.len());
        unique.push(chunk);
    }
    Some(unique)
}
async fn drain(
    tasks: &mut JoinSet<Result<Vec<RawChunk>, SiftError>>,
    task_sources: &mut HashMap<tokio::task::Id, SourceId>,
    reports: &mut Vec<SourceReport>,
    retrieved: &mut Vec<RawChunk>,
    max_chunks: u16,
) -> Result<Vec<SourceId>, SiftError> {
    tasks.abort_all();
    let mut aborted = Vec::new();
    while let Some(result) = tasks.join_next_with_id().await {
        let (task_id, result, cancelled) = match result {
            Ok((task_id, result)) => (task_id, result, false),
            Err(error) => (
                error.id(),
                Err(invalid("source task ended")),
                error.is_cancelled(),
            ),
        };
        let source = task_sources
            .remove(&task_id)
            .ok_or_else(|| invalid("unknown source task"))?;
        let status = if cancelled {
            aborted.push(source.clone());
            SourceStatus::Cancelled
        } else {
            match result
                .ok()
                .and_then(|chunks| validated_chunks(chunks, &source, max_chunks))
            {
                Some(chunks) => {
                    retrieved.extend(chunks);
                    SourceStatus::Retrieved
                }
                None => SourceStatus::Failed,
            }
        };
        reports.push(SourceReport { source, status });
    }
    if !task_sources.is_empty() {
        return Err(invalid("source task ownership lost"));
    }
    aborted.sort();
    Ok(aborted)
}
/// Concrete bridge from either SDK web provider to untrusted provenance-bearing chunks.
#[derive(Debug)]
pub struct WebSearchSource<W> {
    id: SourceId,
    provider: W,
}
impl<W: WebSearch> WebSearchSource<W> {
    /// Inject a redacting SDK web provider and stable source identity.
    #[must_use]
    pub const fn new(id: SourceId, provider: W) -> Self {
        Self { id, provider }
    }
}
impl<W: WebSearch> KnowledgeSource for WebSearchSource<W> {
    fn id(&self) -> &SourceId {
        &self.id
    }
    async fn search(&self, query: &SiftQuery) -> Result<Vec<RawChunk>, SiftError> {
        let max_results = query.max_chunks;
        let hits = self
            .provider
            .search(&SearchQuery {
                text: query.text.clone(),
                max_results,
                site: None,
                recency_days: None,
            })
            .await
            .map_err(|_| invalid("web source failed"))?;
        Ok(hits
            .into_iter()
            .take(usize::from(max_results))
            .map(|hit| RawChunk {
                source: self.id.clone(),
                locator: Locator::Url(hit.url),
                text: format!("{}\n{}", hit.title, hit.snippet),
                trust: Trust::Untrusted,
            })
            .collect())
    }
}
