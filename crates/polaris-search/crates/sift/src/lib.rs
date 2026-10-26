//! Bounded System-One ranking and context distillation with provenance-preserving fallbacks.
#![forbid(unsafe_code)]
use polaris_sdk::registry::{DecisionEngine, DecisionError};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, Probability, Question, QuestionKind,
};
use std::{collections::BTreeSet, time::Duration};
use thiserror::Error;

mod dropped;
mod race;
pub use dropped::{
    DropManifestError, DroppedManifest, MAX_DROPPED_CHUNK_BYTES, MAX_DROPPED_CHUNKS,
    MAX_DROPPED_MANIFEST_BYTES, MAX_DROPPED_RECOVERY_BYTES, recover_dropped,
};
pub use race::{
    KnowledgeSource, RaceMode, RaceOutcome, RacePolicy, RaceSource, SemaphoreSet, SiftQuery,
    SourcePool, SourceReport, SourceStatus, WebSearchSource,
};

/// Stable adapter-provided source identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(String);
impl SourceId {
    /// Construct a nonempty source identifier.
    /// # Errors
    /// Returns an error for empty or control-containing values.
    pub fn new(value: impl Into<String>) -> Result<Self, SiftError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(SiftError::InvalidSourceId);
        }
        Ok(Self(value))
    }
    /// Source identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Typed source location supplied by an adapter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Locator {
    /// Real URL returned by the adapter.
    Url(String),
    /// Document location with optional page and line coordinates.
    Document {
        /// Document identifier.
        id: String,
        /// Page number when known.
        page: Option<u32>,
        /// Starting line when known.
        line_start: Option<u32>,
        /// Ending line when known.
        line_end: Option<u32>,
    },
    /// Repository-relative symbol location.
    Symbol {
        /// Repository-relative path.
        path: String,
        /// Symbol name.
        name: String,
    },
}
/// Trust annotation provided by the source adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// No independent source verification is asserted.
    Untrusted,
    /// Adapter asserts a known local or curated source.
    CallerValidated,
}
/// Retrieved text with caller-provided provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    /// Source identifier.
    pub source: SourceId,
    /// Actual source locator; never synthesized by Sifter.
    pub locator: Locator,
    /// Original retrieved text.
    pub text: String,
    /// Trust annotation from the source adapter.
    pub trust: Trust,
}
/// Injected S1 questions. Relevance must be Noul and density must be Score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiftQuestions {
    /// Whether the chunk helps satisfy the goal.
    pub relevance: Question,
    /// Ordinal information-density score.
    pub density: Question,
}
/// Sift limits and injected question policy.
#[derive(Debug, Clone)]
pub struct SiftConfig {
    /// Questions supplied by the composition root.
    pub questions: SiftQuestions,
    /// Minimum relevance probability for keeping a chunk after successful S1 ranking.
    pub accept_relevance: Probability,
    /// Maximum candidates considered by one operation.
    pub max_candidates: usize,
    /// Maximum candidates in one sequential validation batch.
    pub batch_size: usize,
    /// Maximum primary text characters sent to S1.
    pub max_chunk_chars: usize,
    /// Per-candidate engine deadline.
    pub decision_deadline: Duration,
}
/// Token budget measured in approximate whitespace-separated units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudget(usize);
impl TokenBudget {
    /// Construct a budget.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self(value)
    }
    /// Budget value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}
/// Source used for a relevance score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreBasis {
    /// S1 answers passed shape validation.
    S1,
    /// Deterministic lexical overlap was used.
    LexicalFallback,
}
/// Chunk with ranking output and stable input position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedChunk {
    /// Original chunk and provenance.
    pub chunk: RawChunk,
    /// Relevance probability.
    pub relevance: Probability,
    /// Information-density ordinal.
    pub density: i32,
    /// Ranking basis.
    pub basis: ScoreBasis,
    /// Position in the caller's original input.
    pub input_order: usize,
}
/// Exact drop counts; token savings are approximate whitespace-unit estimates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropStats {
    /// Number of chunks dropped.
    pub dropped: u32,
    /// Approximate whitespace-token units removed.
    pub estimated_tokens_saved: u32,
}
/// Sift result; dropped source chunks remain available in this value for recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distilled {
    /// Kept chunks in original input order.
    pub kept: Vec<RankedChunk>,
    /// Actual dropped chunks with their source provenance and original text.
    pub dropped: Vec<RawChunk>,
    /// Drop counters.
    pub stats: DropStats,
    /// True if any lexical fallback was needed.
    pub fallback: bool,
    /// True when fail-open preservation exceeded the requested budget.
    pub budget_exceeded: bool,
}
/// Sifting failures for invalid caller configuration and malformed metadata.
#[derive(Debug, Error)]
pub enum SiftError {
    /// The caller cancelled the entire retrieval operation.
    #[error("search cancelled")]
    Cancelled,
    /// Source id is empty or contains control characters.
    #[error("invalid source id")]
    InvalidSourceId,
    /// Candidate bounds or injected questions are invalid.
    #[error("invalid sift configuration: {0}")]
    InvalidConfig(String),
    /// Relevance distribution is malformed.
    #[error("malformed S1 answer for candidate {0}")]
    InvalidDecision(String),
    /// S1 decision engine failure type, available to adapters that choose strict behavior.
    #[error("decision engine failed: {0}")]
    Engine(#[from] DecisionError),
    /// Probability validation failed.
    #[error("invalid probability: {0}")]
    Probability(#[from] polaris_sdk::error::ParseError),
}
/// Bounded S1 relevance scorer.
pub struct Sifter<D> {
    engine: D,
    config: SiftConfig,
}
impl<D: DecisionEngine> Sifter<D> {
    /// Construct with injected S1 questions and explicit caps.
    /// # Errors
    /// Rejects invalid question kinds, identical question ids, or zero limits/deadline.
    pub fn new(engine: D, config: SiftConfig) -> Result<Self, SiftError> {
        if config.max_candidates == 0
            || config.batch_size == 0
            || config.max_chunk_chars == 0
            || config.decision_deadline.is_zero()
        {
            return Err(SiftError::InvalidConfig(
                "limits and deadline must be nonzero".into(),
            ));
        }
        if config.questions.relevance.id == config.questions.density.id {
            return Err(SiftError::InvalidConfig("question ids must differ".into()));
        }
        if !matches!(&config.questions.relevance.kind, QuestionKind::Noul { .. })
            || !matches!(&config.questions.density.kind,QuestionKind::Score{min,max} if min<=max)
        {
            return Err(SiftError::InvalidConfig(
                "relevance must be Noul and density must be a bounded Score".into(),
            ));
        }
        Ok(Self { engine, config })
    }
    /// Rank all chunks, using bounded S1 calls and deterministic lexical fallback.
    /// Any unavailable, timed-out, or malformed S1 response causes all original candidates
    /// to be returned with lexical scores.
    /// # Errors
    /// Returns error only when the injected policy cannot be represented safely.
    pub async fn rank_chunks(
        &self,
        goal: &str,
        chunks: &[RawChunk],
    ) -> Result<Vec<RankedChunk>, SiftError> {
        self.rank_candidates(goal, chunks).await
    }
    /// Rank web/search snippets using the same chunk and provenance contract.
    /// # Errors
    /// Returns error only when injected policy is invalid.
    pub async fn rank_snippets(
        &self,
        goal: &str,
        snippets: &[RawChunk],
    ) -> Result<Vec<RankedChunk>, SiftError> {
        self.rank_candidates(goal, snippets).await
    }
    /// Filter chunks by relevance and enforce a whitespace-token budget.
    ///
    /// On S1 failure, all unique source candidates are preserved, even if the budget is
    /// exceeded. On success, lower-relevance chunks are dropped first; original order is kept.
    /// Dropped text is returned in memory with its real locator because no CAS writer is injected.
    /// # Errors
    /// Returns error only when injected policy is invalid.
    pub async fn sift(
        &self,
        goal: &str,
        chunks: Vec<RawChunk>,
        budget: TokenBudget,
    ) -> Result<Distilled, SiftError> {
        let ranked = self.rank_candidates(goal, &chunks).await?;
        let fallback = ranked
            .iter()
            .any(|item| item.basis == ScoreBasis::LexicalFallback);
        let mut selected = if fallback {
            ranked.clone()
        } else {
            ranked
                .into_iter()
                .filter(|item| item.relevance >= self.config.accept_relevance)
                .collect()
        };
        selected.sort_by(|a, b| {
            b.relevance
                .cmp(&a.relevance)
                .then_with(|| b.density.cmp(&a.density))
                .then_with(|| a.input_order.cmp(&b.input_order))
        });
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        let mut used = 0_usize;
        for item in selected {
            let estimate = estimate_tokens(&item.chunk.text);
            if fallback || used.saturating_add(estimate) <= budget.get() {
                used = used.saturating_add(estimate);
                kept.push(item);
            } else {
                dropped.push(item.chunk);
            }
        }
        if fallback {
            for item in &chunks {
                if !contains_identity(&kept, &dropped, &item.source, &item.locator) {
                    dropped.push(item.clone());
                }
            }
        }
        let mut deduped = Vec::new();
        let mut seen = BTreeSet::new();
        let mut duplicates = Vec::new();
        for item in kept {
            let identity = (item.chunk.source.clone(), item.chunk.locator.clone());
            if seen.insert(identity) {
                deduped.push(item);
            } else {
                duplicates.push(item.chunk);
            }
        }
        dropped.extend(duplicates);
        for item in &chunks {
            if !contains_identity(&deduped, &dropped, &item.source, &item.locator) {
                dropped.push(item.clone());
            }
        }
        deduped.sort_by_key(|item| item.input_order);
        let saved = dropped
            .iter()
            .map(|chunk| estimate_tokens(&chunk.text))
            .fold(0_usize, usize::saturating_add);
        let dropped_count = saturating_count(dropped.len());
        let saved_count = saturating_count(saved);
        let budget_exceeded = fallback
            && deduped
                .iter()
                .map(|item| estimate_tokens(&item.chunk.text))
                .fold(0_usize, usize::saturating_add)
                > budget.get();
        Ok(Distilled {
            kept: deduped,
            dropped,
            stats: DropStats {
                dropped: dropped_count,
                estimated_tokens_saved: saved_count,
            },
            fallback,
            budget_exceeded,
        })
    }
    async fn rank_candidates(
        &self,
        goal: &str,
        chunks: &[RawChunk],
    ) -> Result<Vec<RankedChunk>, SiftError> {
        let limit = chunks.len().min(self.config.max_candidates);
        let mut ranked = Vec::with_capacity(chunks.len());
        let candidates = chunks.iter().enumerate().take(limit).collect::<Vec<_>>();
        for batch in candidates.chunks(self.config.batch_size) {
            for (order, chunk) in batch {
                let Some(score) = self.score_one(goal, chunk).await else {
                    return self.lexical_all(goal, chunks);
                };
                ranked.push(RankedChunk {
                    chunk: (*chunk).clone(),
                    relevance: score.0,
                    density: score.1,
                    basis: ScoreBasis::S1,
                    input_order: *order,
                });
            }
        }
        for (order, chunk) in chunks.iter().enumerate().skip(limit) {
            ranked.push(RankedChunk {
                chunk: chunk.clone(),
                relevance: Probability::new(lexical_overlap(goal, &chunk.text))?,
                density: 0,
                basis: ScoreBasis::LexicalFallback,
                input_order: order,
            });
        }
        let mut ordered = Vec::with_capacity(ranked.len());
        let mut seen = BTreeSet::new();
        ranked.sort_by(|a, b| {
            b.relevance
                .cmp(&a.relevance)
                .then_with(|| b.density.cmp(&a.density))
                .then_with(|| a.input_order.cmp(&b.input_order))
        });
        for item in ranked {
            let key = (item.chunk.source.clone(), item.chunk.locator.clone());
            if seen.insert(key) {
                ordered.push(item);
            }
        }
        Ok(ordered)
    }
    async fn score_one(&self, goal: &str, chunk: &RawChunk) -> Option<(Probability, i32)> {
        let text = truncate_chars(&chunk.text, self.config.max_chunk_chars);
        let input = DecisionInput {
            text,
            context: Some(goal.to_owned()),
        };
        let questions = [
            self.config.questions.relevance.clone(),
            self.config.questions.density.clone(),
        ];
        let response = tokio::time::timeout(
            self.config.decision_deadline,
            self.engine.decide(&questions, &input),
        )
        .await
        .ok()?
        .ok()?;
        parse_decision(&self.config.questions, &response)
    }
    fn lexical_all(&self, goal: &str, chunks: &[RawChunk]) -> Result<Vec<RankedChunk>, SiftError> {
        let mut output = Vec::with_capacity(chunks.len());
        let mut seen = BTreeSet::new();
        for (order, chunk) in chunks.iter().enumerate() {
            let key = (chunk.source.clone(), chunk.locator.clone());
            if !seen.insert(key) {
                continue;
            }
            output.push(RankedChunk {
                chunk: chunk.clone(),
                relevance: Probability::new(lexical_overlap(goal, &chunk.text))?,
                density: 0,
                basis: ScoreBasis::LexicalFallback,
                input_order: order,
            });
        }
        output.sort_by(|a, b| {
            b.relevance
                .cmp(&a.relevance)
                .then_with(|| a.input_order.cmp(&b.input_order))
        });
        Ok(output)
    }
}
fn parse_decision(q: &SiftQuestions, d: &Decision) -> Option<(Probability, i32)> {
    if d.answers.len() != 2 {
        return None;
    }
    let mut relevance = None;
    let mut density = None;
    let mut seen = BTreeSet::new();
    for answer in &d.answers {
        if !seen.insert(answer.question.clone()) {
            return None;
        }
        if answer.question == q.relevance.id {
            if let Answer::Noul { yes } = &answer.answer {
                relevance = Some(*yes);
            } else {
                return None;
            }
        } else if answer.question == q.density.id {
            if let Answer::Score { value, .. } = &answer.answer {
                if let QuestionKind::Score { min, max } = &q.density.kind {
                    if value < min || value > max {
                        return None;
                    }
                    density = Some(*value);
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            return None;
        }
    }
    Some((relevance?, density?))
}
fn lexical_overlap(goal: &str, text: &str) -> f32 {
    let goal_terms = terms(goal);
    if goal_terms.is_empty() {
        return 0.0;
    }
    let text_terms = terms(text);
    let mut total = 0_u16;
    let mut hits = 0_u16;
    for term in goal_terms {
        total = total.saturating_add(1);
        if text_terms.contains(&term) {
            hits = hits.saturating_add(1);
        }
    }
    if total == 0 {
        0.0
    } else {
        f32::from(hits) / f32::from(total)
    }
}
fn terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.chars().count() > 1)
        .map(str::to_lowercase)
        .collect()
}
fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}
fn saturating_count(value: usize) -> u32 {
    let Ok(count) = u32::try_from(value) else {
        return u32::MAX;
    };
    count
}
fn contains_identity(
    kept: &[RankedChunk],
    dropped: &[RawChunk],
    source: &SourceId,
    locator: &Locator,
) -> bool {
    kept.iter()
        .any(|item| &item.chunk.source == source && &item.chunk.locator == locator)
        || dropped
            .iter()
            .any(|item| &item.source == source && &item.locator == locator)
}
