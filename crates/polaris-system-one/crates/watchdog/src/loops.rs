//! Bounded exact-repeat and token-shingle loop detection.

use crate::{
    ProposedStep, ShingleSet, ShingleSimilarity, StepFingerprint, StepIndex, StepOutcome,
    StepRecord, ToolCallFacts, WatchdogConfig,
};
use std::collections::{BTreeSet, VecDeque};
use xxhash_rust::xxh3::{xxh3_128, xxh3_64};

/// Failure to form a bounded step record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StepBuildError {
    /// The composed thought and tool facts exceed the configured byte cap.
    #[error("proposed step exceeds the configured byte limit")]
    TooLarge,
    /// Maximum input byte count was zero or above the hard one-MiB safety bound.
    #[error("step byte limit must be non-zero")]
    InvalidByteLimit,
    /// The configured shingle size is zero.
    #[error("shingle size must be non-zero")]
    InvalidShingleSize,
    /// The configured ring window is zero or exceeds the hard 500-record bound.
    #[error("loop window size must be non-zero")]
    InvalidWindowSize,
    /// A shingle count exceeded the supported fixed-width comparison range.
    #[error("shingle set exceeds supported bound")]
    TooManyShingles,
}

/// A bounded ring of recent step records for one frame.
#[derive(Debug, Clone)]
pub struct LoopWindow {
    capacity: usize,
    records: VecDeque<StepRecord>,
}

impl LoopWindow {
    /// Create an empty ring with an explicit non-zero maximum size.
    ///
    /// # Errors
    /// Returns [`StepBuildError::InvalidWindowSize`] for zero or overlarge capacity.
    pub fn new(capacity: usize) -> Result<Self, StepBuildError> {
        if capacity == 0 || capacity > 500 {
            return Err(StepBuildError::InvalidWindowSize);
        }
        Ok(Self {
            capacity,
            records: VecDeque::with_capacity(capacity),
        })
    }

    /// Number of records currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the ring is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Records in oldest-first order.
    #[must_use]
    pub fn records(&self) -> &VecDeque<StepRecord> {
        &self.records
    }

    /// Add a record and evict the oldest when the configured ring is full.
    pub fn push(&mut self, record: StepRecord) {
        if self.records.len() == self.capacity {
            drop(self.records.pop_front());
        }
        self.records.push_back(record);
    }

    /// Detect the nearest exact or near-duplicate unproductive prior step.
    #[must_use]
    pub fn detect(&self, next: &StepRecord, config: &WatchdogConfig) -> Option<crate::LoopMatch> {
        for previous in self.records.iter().rev() {
            let disproven = match previous.outcome {
                StepOutcome::RejectedPatch { hypothesis } => Some(hypothesis),
                StepOutcome::Completed
                | StepOutcome::Failed
                | StepOutcome::Cancelled
                | StepOutcome::Unknown => None,
            };
            if matches!(previous.outcome, StepOutcome::Completed) {
                continue;
            }
            if previous.fingerprint == next.fingerprint {
                return Some(crate::LoopMatch {
                    repeats: previous.index,
                    disproven,
                    similarity: ShingleSimilarity::new(1, 1),
                });
            }
            let similarity = jaccard(&previous.shingles, &next.shingles)?;
            if ratio_at_least(
                similarity.intersection,
                similarity.union,
                config.loops.near_duplicate,
            ) {
                return Some(crate::LoopMatch {
                    repeats: previous.index,
                    disproven,
                    similarity,
                });
            }
        }
        None
    }

    /// Highest shingle similarity to any record in the ring, regardless of outcome.
    #[must_use]
    pub fn highest_similarity(&self, next: &StepRecord) -> ShingleSimilarity {
        let mut highest = ShingleSimilarity::zero();
        for previous in &self.records {
            if let Some(similarity) = jaccard(&previous.shingles, &next.shingles) {
                if ratio_greater(similarity, highest) {
                    highest = similarity;
                }
            }
        }
        highest
    }
}

/// Build a step record after bounding the combined thought and tool facts.
///
/// The xxh3 fingerprints and shingle hashes are advisory repetition signals only. They are not
/// collision-resistant and must never be used for capability, approval, or CAS identity.
///
/// # Errors
/// Returns a [`StepBuildError`] when the step or its shingle configuration exceeds bounds.
pub fn build_step_record(
    index: StepIndex,
    step: &ProposedStep,
    shingle_size: usize,
    max_bytes: usize,
    outcome: StepOutcome,
) -> Result<StepRecord, StepBuildError> {
    if shingle_size == 0 {
        return Err(StepBuildError::InvalidShingleSize);
    }
    if max_bytes == 0 || max_bytes > 1_048_576 {
        return Err(StepBuildError::InvalidByteLimit);
    }
    let total = composed_len(&step.thought, step.tool_call.as_ref())?;
    if total > max_bytes {
        return Err(StepBuildError::TooLarge);
    }
    let composed = compose(&step.thought, step.tool_call.as_ref());
    let normalized = normalize(&composed);
    let fingerprint = StepFingerprint::from_raw(xxh3_128(normalized.as_bytes()));
    let shingles = shingles(&normalized, shingle_size)?;
    Ok(StepRecord {
        index,
        fingerprint,
        shingles,
        tool: step.tool_call.as_ref().map(|call| call.tool.as_str().to_owned()),
        outcome,
    })
}

fn composed_len(thought: &str, tool: Option<&ToolCallFacts>) -> Result<usize, StepBuildError> {
    let Some(call) = tool else {
        return Ok(thought.len());
    };
    let mut length = thought
        .len()
        .checked_add(6)
        .and_then(|size| size.checked_add(call.tool.as_str().len()))
        .and_then(|size| size.checked_add(6))
        .and_then(|size| size.checked_add(call.workspace_path.as_deref().map_or(0, str::len)))
        .and_then(|size| size.checked_add(7))
        .ok_or(StepBuildError::TooLarge)?;
    length = length
        .checked_add(bool_len(call.writes_workspace))
        .and_then(|size| size.checked_add(9))
        .and_then(|size| size.checked_add(bool_len(call.network)))
        .and_then(|size| size.checked_add(9))
        .and_then(|size| size.checked_add(bool_len(call.executes)))
        .ok_or(StepBuildError::TooLarge)?;
    Ok(length)
}

fn compose(thought: &str, tool: Option<&ToolCallFacts>) -> String {
    let Some(call) = tool else {
        return thought.to_owned();
    };
    let path = match call.workspace_path.as_deref() {
        Some(path) => path,
        None => "",
    };
    format!(
        "{thought}\ntool={}\npath={}\nwrite={}\nnetwork={}\nexecute={}",
        call.tool.as_str(),
        path,
        call.writes_workspace,
        call.network,
        call.executes,
    )
}

fn bool_len(value: bool) -> usize {
    if value { 4 } else { 5 }
}

fn normalize(input: &str) -> String {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.join(" ")
}

fn shingles(normalized: &str, size: usize) -> Result<ShingleSet, StepBuildError> {
    if normalized.is_empty() {
        return Ok(ShingleSet::from_sorted(Vec::new()));
    }
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut values = BTreeSet::new();
    if tokens.len() < size {
        values.insert(xxh3_64(normalized.as_bytes()));
    } else {
        for window in tokens.windows(size) {
            values.insert(xxh3_64(window.join(" ").as_bytes()));
        }
    }
    let count = u32::try_from(values.len()).map_err(|_| StepBuildError::TooManyShingles)?;
    if count > 1_000_000 {
        return Err(StepBuildError::TooManyShingles);
    }
    Ok(ShingleSet::from_sorted(values.into_iter().collect()))
}

fn jaccard(left: &ShingleSet, right: &ShingleSet) -> Option<ShingleSimilarity> {
    if left.is_empty() || right.is_empty() {
        return Some(ShingleSimilarity::zero());
    }
    let mut left_iter = left.values().iter().peekable();
    let mut right_iter = right.values().iter().peekable();
    let mut intersection = 0_u32;
    let mut union = 0_u32;
    loop {
        match (left_iter.peek(), right_iter.peek()) {
            (Some(left_value), Some(right_value)) if left_value == right_value => {
                intersection = intersection.checked_add(1)?;
                union = union.checked_add(1)?;
                let _left = left_iter.next();
                let _right = right_iter.next();
            }
            (Some(left_value), Some(right_value)) if left_value < right_value => {
                union = union.checked_add(1)?;
                let _left = left_iter.next();
            }
            (Some(_), Some(_)) => {
                union = union.checked_add(1)?;
                let _right = right_iter.next();
            }
            (Some(_), None) => {
                union = union.checked_add(1)?;
                let _left = left_iter.next();
            }
            (None, Some(_)) => {
                union = union.checked_add(1)?;
                let _right = right_iter.next();
            }
            (None, None) => break,
        }
    }
    Some(ShingleSimilarity::new(intersection, union))
}

pub(crate) fn ratio_at_least(
    numerator: u32,
    denominator: u32,
    threshold: crate::RatioThreshold,
) -> bool {
    if denominator == 0 || threshold.denominator == 0 {
        return false;
    }
    u128::from(numerator) * u128::from(threshold.denominator)
        >= u128::from(denominator) * u128::from(threshold.numerator)
}

fn ratio_greater(left: ShingleSimilarity, right: ShingleSimilarity) -> bool {
    u128::from(left.intersection) * u128::from(right.union)
        > u128::from(right.intersection) * u128::from(left.union)
}
