//! Public watchdog inputs, outputs, and bounded-analysis configuration.

use crate::ToolCallFacts;
use polaris_sdk::{
    ids::{AgentRole, FrameId},
    system_one::Probability,
};
use serde::{Deserialize, Serialize};

/// xxh3-128 fingerprint for loop detection only; never an authorization or content identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepFingerprint(u128);

impl StepFingerprint {
    pub(crate) const fn from_raw(value: u128) -> Self {
        Self(value)
    }

    /// The raw non-cryptographic fingerprint value.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

/// One-based position assigned to a step by its owning context frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StepIndex(u32);

impl StepIndex {
    /// Wrap an existing frame step number.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The raw frame step number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Sorted, unique shingle hashes for one normalized proposed step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShingleSet(Vec<u64>);

impl ShingleSet {
    /// Number of unique shingles.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the step produced no shingles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The sorted, unique shingle values.
    #[must_use]
    pub fn values(&self) -> &[u64] {
        &self.0
    }

    pub(crate) fn from_sorted(values: Vec<u64>) -> Self {
        Self(values)
    }
}

/// Exact Jaccard numerator and denominator, kept as integers to avoid unstable float comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShingleSimilarity {
    /// Number of shared shingle hashes.
    pub intersection: u32,
    /// Number of unique hashes across both sets.
    pub union: u32,
}

impl ShingleSimilarity {
    /// Similarity of two sets, returned as `intersection / union`.
    #[must_use]
    pub const fn new(intersection: u32, union: u32) -> Self {
        Self {
            intersection,
            union,
        }
    }

    /// A zero-overlap value for two empty or disjoint sets.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            intersection: 0,
            union: 1,
        }
    }
}

/// Rational threshold used for pressure and shingle-similarity comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatioThreshold {
    /// Threshold numerator.
    pub numerator: u32,
    /// Non-zero threshold denominator.
    pub denominator: u32,
}

/// Bounds and duplicate threshold for the recent-step ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopConfig {
    /// Maximum number of recent records retained for one inspection.
    pub window_size: usize,
    /// Number of normalized tokens in each shingle; short steps use one whole-step shingle.
    pub shingle_size: usize,
    /// Minimum Jaccard similarity for a near-duplicate match.
    pub near_duplicate: RatioThreshold,
}

/// Thresholds for context pressure and repetition fatigue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FatigueConfig {
    /// Context-used/context-budget ratio that triggers pressure.
    pub pressure: RatioThreshold,
    /// Repetition similarity that triggers fatigue when repetition is required.
    pub repetition: RatioThreshold,
    /// Require both high context pressure and repeated shingles to report fatigue.
    pub require_both_signals: bool,
}

/// Bounded watchdog configuration. Values are supplied by the composition root from validated
/// deployment configuration; no inference threshold or ring limit is silently hard-coded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogConfig {
    /// Largest thought/tool description accepted for hashing and S1 input.
    pub max_step_bytes: usize,
    /// Exact-repeat window and shingle settings.
    pub loops: LoopConfig,
    /// Context-pressure and repetition settings.
    pub fatigue: FatigueConfig,
}

/// Outcome of a completed prior step, used to identify failed variants that must not be repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcome {
    /// The step completed and made progress.
    Completed,
    /// A proposed patch was rejected; its fingerprint is a disproven hypothesis.
    RejectedPatch {
        /// Fingerprint of the rejected patch variant.
        hypothesis: StepFingerprint,
    },
    /// The step failed without proving a patch variant wrong.
    Failed,
    /// The step was cancelled before a result was available.
    Cancelled,
    /// Outcome could not be established.
    Unknown,
}

/// Record of one prior step retained in the frame snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    /// Frame-local step index.
    pub index: StepIndex,
    /// Fingerprint of normalized thought and tool facts.
    pub fingerprint: StepFingerprint,
    /// Sorted, unique normalized token shingles.
    pub shingles: ShingleSet,
    /// Tool id, if a tool call was proposed.
    pub tool: Option<String>,
    /// Outcome recorded by the frame owner.
    pub outcome: StepOutcome,
}

/// Persistable advisory memory of a rejected hypothesis; not an authorization fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegativeConstraint {
    /// Exact rejected proposed-step fingerprint.
    pub hypothesis: StepFingerprint,
    /// Frame-local step at which it was rejected.
    pub rejected_at: StepIndex,
}

/// Read-only snapshot supplied by the owning context frame before the next S2 call.
#[derive(Debug, Clone)]
pub struct FrameSnapshot {
    /// Frame identity.
    pub frame: FrameId,
    /// Already-reserved current frame step number.
    pub current_step: u32,
    /// Assigned role; only used for advisory role-table lookup.
    pub role: AgentRole,
    /// Root task goal.
    pub root_goal: String,
    /// Bounded recent step records, oldest first.
    pub recent: Vec<StepRecord>,
    /// Bounded fingerprints of rejected hypotheses retained beyond the recent-record window.
    pub negative_constraints: Vec<NegativeConstraint>,
    /// Current context token count.
    pub context_tokens: u32,
    /// Context token budget; zero is treated as unavailable evidence.
    pub context_budget: u32,
}

/// Proposed step evaluated before it reaches S2.
#[derive(Debug, Clone)]
pub struct ProposedStep {
    /// Agent thought or intended action text.
    pub thought: String,
    /// Optional structured tool request facts.
    pub tool_calls: Vec<ToolCallFacts>,
}

/// Result from deterministic loop detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopMatch {
    /// Index of the earlier matching step.
    pub repeats: StepIndex,
    /// Fingerprint of a rejected patch hypothesis, if one was recorded.
    pub disproven: Option<StepFingerprint>,
    /// Jaccard similarity with the prior record; exact repeats have `1 / 1`.
    pub similarity: ShingleSimilarity,
}

/// Advisory watchdog result. Only the executor's capability checks control tool access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthVerdict {
    /// No deterministic warning and no confident S1 goal-drift signal.
    Nominal,
    /// S1 judged the proposed step poorly aligned with the root goal.
    GoalDrift {
        /// Probability that the step advances the root goal.
        alignment: Probability,
        /// Bounded root-goal anchor for corrective prompting.
        anchor: String,
    },
    /// The step repeats an unproductive prior step.
    Loop {
        /// Earlier frame-local step index.
        repeats: StepIndex,
        /// Rejected hypothesis fingerprint that should be added to the frame's negative context.
        disproven: Option<StepFingerprint>,
        /// Exact or shingle-based similarity evidence.
        similarity: ShingleSimilarity,
    },
    /// The role table advises that this role should not propose this tool/path/effect combination.
    RoleViolation {
        /// Requested tool.
        tool: String,
        /// Assigned role.
        role: AgentRole,
    },
    /// Context pressure and repeated content indicate likely context fatigue.
    ContextFatigue {
        /// Current token usage.
        context_tokens: u32,
        /// Configured token budget.
        context_budget: u32,
        /// Repetition evidence from the recent window.
        repetition: ShingleSimilarity,
    },
    /// S1 or deterministic context was unavailable; watchdog remains advisory and fails open.
    Unchecked {
        /// Reason no dependable watchdog result was available.
        reason: UncheckedReason,
    },
}

/// Reason watchdog evidence was unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UncheckedReason {
    /// The proposed thought exceeded the bounded hashing/input size.
    StepTooLarge,
    /// The snapshot has no usable root goal.
    EmptyGoal,
    /// Context budget is zero or otherwise invalid.
    InvalidContextBudget,
    /// Frame-local step index reached its maximum value.
    StepIndexOverflow,
    /// Goal-drift schema is missing or incompatible.
    SchemaUnavailable,
    /// S1 exceeded the schema's deadline.
    EngineTimeout,
    /// S1 backend failed.
    EngineFailure,
    /// S1 output did not match the active schema.
    InvalidDecision,
}
