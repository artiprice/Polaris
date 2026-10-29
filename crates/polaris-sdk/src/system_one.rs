//! System-One (fast classifier) primitives: Choice / Score / Noul questions and their answers.
//! Serde uses internally tagged enums (`"type": …`); the chatlog's `untagged` form is rejected as
//! ambiguous to decode.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::error::ParseError;
use crate::registry::DecisionError;

/// One semantic PII range in the exact UTF-8 request text.
///
/// Offsets are half-open byte positions, not character counts. The matched text and model label
/// are intentionally absent so local worker diagnostics cannot accidentally echo entity values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PiiEntitySpan {
    start: u32,
    end: u32,
    confidence: Probability,
}

impl PiiEntitySpan {
    /// Validates a nonempty half-open byte range and a finite confidence in `0.0..=1.0`.
    ///
    /// # Errors
    /// Returns `InvalidValue` for empty/reversed ranges and `InvalidProbability` for invalid
    /// confidence values. UTF-8 bounds are validated by the consumer against the original text.
    pub fn new(start: u32, end: u32, confidence: f32) -> Result<Self, ParseError> {
        if start >= end {
            return Err(ParseError::InvalidValue {
                what: "PII entity span",
                reason: "range must be nonempty and ordered",
            });
        }
        Ok(Self {
            start,
            end,
            confidence: Probability::new(confidence)?,
        })
    }

    /// Start byte offset in the exact raw request text.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Exclusive end byte offset in the exact raw request text.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Model confidence for this candidate span.
    #[must_use]
    pub const fn confidence(self) -> f32 {
        self.confidence.get()
    }
}

/// Async local model port that returns semantic PII byte spans for exact raw text.
pub trait PiiSpanDetector: Send + Sync {
    /// Returns bounded entity spans without returning matched text or labels.
    fn detect_pii_spans(
        &self,
        raw_text: &str,
    ) -> impl core::future::Future<Output = Result<Vec<PiiEntitySpan>, DecisionError>> + Send;
}

/// A probability: finite and within `0.0..=1.0` (NaN rejected), with a total order.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Probability(f32);

impl Probability {
    /// Validates a probability.
    ///
    /// # Errors
    /// [`ParseError::InvalidProbability`] for NaN, infinities and values outside `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, ParseError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            // -0.0 passes the range check but orders below 0.0 under `total_cmp`; store +0.0.
            Ok(Self(if value == 0.0 { 0.0 } else { value }))
        } else {
            Err(ParseError::InvalidProbability)
        }
    }

    /// The value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Probability {
    type Error = ParseError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Probability> for f32 {
    fn from(value: Probability) -> Self {
        value.0
    }
}

impl PartialEq for Probability {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Probability {}

impl PartialOrd for Probability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Probability {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for Probability {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl fmt::Debug for Probability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Probability({})", self.0)
    }
}

macro_rules! short_name {
    ($(#[$doc:meta])* $name:ident, $what:literal, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Box<str>);

        impl $name {
            /// Longest accepted value in bytes.
            pub const MAX_LEN: usize = $max;

            /// Validates and wraps the value: non-empty, at most [`Self::MAX_LEN`] bytes, no
            /// control characters.
            ///
            /// # Errors
            /// [`ParseError::InvalidValue`] when a rule is violated.
            pub fn new(value: &str) -> Result<Self, ParseError> {
                if value.trim().is_empty() {
                    return Err(ParseError::InvalidValue { what: $what, reason: "empty" });
                }
                if value.len() > Self::MAX_LEN {
                    return Err(ParseError::InvalidValue { what: $what, reason: "too long" });
                }
                if value.chars().any(char::is_control) {
                    return Err(ParseError::InvalidValue { what: $what, reason: "contains a control character" });
                }
                Ok(Self(value.into()))
            }

            /// The text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(&value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0.into_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

short_name!(
    /// Identifier of a question inside a decision schema (e.g. `intent`, `needs_sandbox`).
    QuestionId,
    "question id",
    64
);
short_name!(
    /// A class label of a Choice question (e.g. `refactor`, `0`..`10` for ordinal scores).
    Label,
    "label",
    128
);

/// What is asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionKind {
    /// Pick one label out of `options`.
    Choice {
        /// Candidate labels, in schema order.
        options: Vec<Label>,
    },
    /// An integer score within `min..=max` (ordinal labels such as `"0"`..`"10"`).
    Score {
        /// Lowest score.
        min: i32,
        /// Highest score.
        max: i32,
    },
    /// A yes/no judgement of `statement` ("noul" in System One's vocabulary).
    Noul {
        /// The statement to judge.
        statement: String,
    },
}

/// One question of a decision schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Stable id used to look the answer up.
    pub id: QuestionId,
    /// The question.
    pub kind: QuestionKind,
}

/// A label with its probability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelScore {
    /// The label.
    pub label: Label,
    /// Its probability.
    pub probability: Probability,
}

/// The engine's answer to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// Answer to a Choice question.
    Choice {
        /// The top label.
        selected: Label,
        /// Every candidate with its probability, descending.
        probs: Vec<LabelScore>,
    },
    /// Answer to a Score question.
    Score {
        /// The score.
        value: i32,
        /// Confidence of the score.
        confidence: Probability,
    },
    /// Answer to a Noul question.
    Noul {
        /// Probability that the statement holds.
        yes: Probability,
    },
}

/// A question id with its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAnswer {
    /// The question answered.
    pub question: QuestionId,
    /// The answer.
    pub answer: Answer,
}

/// Result of one `DecisionEngine::decide` call (all questions of a schema, batched).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Answers in schema order.
    pub answers: Vec<QuestionAnswer>,
    /// Engine latency in microseconds.
    pub latency_us: u64,
}

/// Text the questions are asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionInput {
    /// The primary text (already PII-masked by the caller).
    pub text: String,
    /// Optional surrounding context.
    pub context: Option<String>,
}

/// Whether a schema consumer uses a deterministic fallback or refuses an unavailable decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSchemaFailMode {
    /// The consumer may use its documented conservative fallback after timeout or engine error.
    Open,
    /// The consumer must return an error when the decision cannot be established.
    Closed,
}

/// Schema integrity policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSchemaIntegrity {
    /// Validate schema and decision structure.
    Normal,
    /// Require matching calibration provenance before treating acceptance as authoritative.
    Strict,
}

/// Calibration state included with an SDK schema snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSchemaCalibrationState {
    /// No measured fixture calibration is available.
    Uncalibrated,
    /// Measured calibration provenance is present.
    Calibrated,
}

/// Probability boundaries copied from the canonical System-One schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionSchemaThresholds {
    accept: Probability,
    reject: Probability,
    abstain_low: Probability,
    abstain_high: Probability,
}

impl DecisionSchemaThresholds {
    /// Validates a non-overlapping reject, abstain, accept ordering.
    pub fn new(
        accept: Probability,
        reject: Probability,
        abstain_low: Probability,
        abstain_high: Probability,
    ) -> Result<Self, DecisionSchemaError> {
        if reject > abstain_low || abstain_low > abstain_high || abstain_high > accept {
            return Err(DecisionSchemaError::InvalidThresholds);
        }
        Ok(Self {
            accept,
            reject,
            abstain_low,
            abstain_high,
        })
    }

    /// Acceptance lower bound.
    #[must_use]
    pub const fn accept(self) -> Probability {
        self.accept
    }

    /// Rejection upper bound.
    #[must_use]
    pub const fn reject(self) -> Probability {
        self.reject
    }

    /// Lower abstention bound.
    #[must_use]
    pub const fn abstain_low(self) -> Probability {
        self.abstain_low
    }

    /// Upper abstention bound.
    #[must_use]
    pub const fn abstain_high(self) -> Probability {
        self.abstain_high
    }
}

/// Per-consumer failure and deadline policy from the canonical schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionSchemaFailPolicy {
    consumer: String,
    on_timeout: DecisionSchemaFailMode,
    deadline_ms: u32,
    integrity: DecisionSchemaIntegrity,
}

impl DecisionSchemaFailPolicy {
    /// Validates and records a consumer's failure policy.
    pub fn new(
        consumer: String,
        on_timeout: DecisionSchemaFailMode,
        deadline_ms: u32,
        integrity: DecisionSchemaIntegrity,
    ) -> Result<Self, DecisionSchemaError> {
        if consumer.is_empty()
            || consumer.len() > 64
            || !consumer
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            || deadline_ms == 0
        {
            return Err(DecisionSchemaError::InvalidFailurePolicy);
        }
        Ok(Self {
            consumer,
            on_timeout,
            deadline_ms,
            integrity,
        })
    }

    /// Consumer id.
    #[must_use]
    pub fn consumer(&self) -> &str {
        &self.consumer
    }

    /// Timeout behavior.
    #[must_use]
    pub const fn on_timeout(&self) -> DecisionSchemaFailMode {
        self.on_timeout
    }

    /// Decision deadline in milliseconds.
    #[must_use]
    pub const fn deadline_ms(&self) -> u32 {
        self.deadline_ms
    }

    /// Integrity mode.
    #[must_use]
    pub const fn integrity(&self) -> DecisionSchemaIntegrity {
        self.integrity
    }
}

/// Calibration state and immutable provenance from the canonical schema.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionSchemaCalibration {
    state: DecisionSchemaCalibrationState,
    fixture_set: String,
    fixture_blake3: Option<String>,
    sample_count: u32,
    precision_target: Option<f32>,
    observed_precision: Option<f32>,
    observed_recall: Option<f32>,
    expected_calibration_error: Option<f32>,
    checkpoint_blake3: Option<String>,
    date: Option<String>,
}

/// Raw calibration fields consumed by [`DecisionSchemaCalibration::try_new`].
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionSchemaCalibrationRecord {
    /// State.
    pub state: DecisionSchemaCalibrationState,
    /// Fixture collection name.
    pub fixture_set: String,
    /// Fixture BLAKE3 digest.
    pub fixture_blake3: Option<String>,
    /// Number of labelled samples.
    pub sample_count: u32,
    /// Target precision.
    pub precision_target: Option<f32>,
    /// Measured precision.
    pub observed_precision: Option<f32>,
    /// Measured recall.
    pub observed_recall: Option<f32>,
    /// Expected calibration error.
    pub expected_calibration_error: Option<f32>,
    /// Checkpoint BLAKE3 digest.
    pub checkpoint_blake3: Option<String>,
    /// Calibration date.
    pub date: Option<String>,
}

impl DecisionSchemaCalibration {
    /// Validates and records complete calibration provenance.
    pub fn try_new(record: DecisionSchemaCalibrationRecord) -> Result<Self, DecisionSchemaError> {
        let DecisionSchemaCalibrationRecord {
            state,
            fixture_set,
            fixture_blake3,
            sample_count,
            precision_target,
            observed_precision,
            observed_recall,
            expected_calibration_error,
            checkpoint_blake3,
            date,
        } = record;
        let probabilities = [
            precision_target,
            observed_precision,
            observed_recall,
            expected_calibration_error,
        ];
        if probabilities
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(DecisionSchemaError::InvalidCalibration);
        }
        let value = Self {
            state,
            fixture_set,
            fixture_blake3,
            sample_count,
            precision_target,
            observed_precision,
            observed_recall,
            expected_calibration_error,
            checkpoint_blake3,
            date,
        };
        if !value.is_consistent() {
            return Err(DecisionSchemaError::InvalidCalibration);
        }
        Ok(value)
    }

    fn is_consistent(&self) -> bool {
        match self.state {
            DecisionSchemaCalibrationState::Uncalibrated => {
                self.sample_count == 0
                    && self.fixture_blake3.is_none()
                    && self.checkpoint_blake3.is_none()
                    && self.observed_precision.is_none()
                    && self.observed_recall.is_none()
                    && self.expected_calibration_error.is_none()
                    && self.date.is_none()
            }
            DecisionSchemaCalibrationState::Calibrated => {
                let valid_digest = |digest: &Option<String>| {
                    digest.as_ref().is_some_and(|value| {
                        value.len() == 64
                            && value
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                };
                let valid_date = self
                    .date
                    .as_ref()
                    .is_some_and(|value| value.parse::<crate::contracts::Timestamp>().is_ok());
                !self.fixture_set.trim().is_empty()
                    && self.sample_count > 0
                    && valid_digest(&self.fixture_blake3)
                    && valid_digest(&self.checkpoint_blake3)
                    && self.precision_target.is_some()
                    && self.observed_precision.is_some()
                    && self.observed_recall.is_some()
                    && self.expected_calibration_error.is_some()
                    && valid_date
                    && match (self.precision_target, self.observed_precision) {
                        (Some(target), Some(observed)) => observed >= target,
                        _ => false,
                    }
            }
        }
    }

    /// Calibration state.
    #[must_use]
    pub const fn state(&self) -> DecisionSchemaCalibrationState {
        self.state
    }

    /// Fixture collection name.
    #[must_use]
    pub fn fixture_set(&self) -> &str {
        &self.fixture_set
    }

    /// Fixture digest.
    #[must_use]
    pub fn fixture_blake3(&self) -> Option<&str> {
        self.fixture_blake3.as_deref()
    }

    /// Number of labelled calibration samples.
    #[must_use]
    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// Target precision.
    #[must_use]
    pub const fn precision_target(&self) -> Option<f32> {
        self.precision_target
    }

    /// Measured precision.
    #[must_use]
    pub const fn observed_precision(&self) -> Option<f32> {
        self.observed_precision
    }

    /// Measured recall.
    #[must_use]
    pub const fn observed_recall(&self) -> Option<f32> {
        self.observed_recall
    }

    /// Expected calibration error.
    #[must_use]
    pub const fn expected_calibration_error(&self) -> Option<f32> {
        self.expected_calibration_error
    }

    /// Inference checkpoint digest.
    #[must_use]
    pub fn checkpoint_blake3(&self) -> Option<&str> {
        self.checkpoint_blake3.as_deref()
    }

    /// Calibration date.
    #[must_use]
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }
}

/// Validated, owned view of a versioned decision schema.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionSchema {
    id: String,
    version: u32,
    max_chars: u32,
    questions: Vec<Question>,
    thresholds: DecisionSchemaThresholds,
    fail: DecisionSchemaFailPolicy,
    calibration: DecisionSchemaCalibration,
}

impl DecisionSchema {
    /// Validates a schema snapshot for use across the SDK boundary.
    pub fn try_new(
        id: String,
        version: u32,
        max_chars: u32,
        questions: Vec<Question>,
        thresholds: DecisionSchemaThresholds,
        fail: DecisionSchemaFailPolicy,
        calibration: DecisionSchemaCalibration,
    ) -> Result<Self, DecisionSchemaError> {
        if id.trim().is_empty()
            || id.len() > 128
            || id.chars().any(char::is_control)
            || version == 0
            || max_chars == 0
            || max_chars > 1_048_576
            || questions.is_empty()
            || questions.len() > 64
        {
            return Err(DecisionSchemaError::InvalidIdentityOrBounds);
        }
        let mut ids = std::collections::BTreeSet::new();
        for question in &questions {
            if !ids.insert(question.id.as_str()) {
                return Err(DecisionSchemaError::InvalidQuestion);
            }
            let valid = match &question.kind {
                QuestionKind::Choice { options } => {
                    !options.is_empty()
                        && options.len() <= 128
                        && options
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == options.len()
                }
                QuestionKind::Score { min, max } => {
                    min <= max && i64::from(*max).saturating_sub(i64::from(*min)) <= 256
                }
                QuestionKind::Noul { statement } => {
                    !statement.trim().is_empty() && statement.len() <= 4096
                }
            };
            if !valid {
                return Err(DecisionSchemaError::InvalidQuestion);
            }
        }
        Ok(Self {
            id,
            version,
            max_chars,
            questions,
            thresholds,
            fail,
            calibration,
        })
    }

    /// Schema id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Maximum input characters.
    #[must_use]
    pub const fn max_chars(&self) -> u32 {
        self.max_chars
    }

    /// Canonical questions.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// Threshold metadata.
    #[must_use]
    pub const fn thresholds(&self) -> DecisionSchemaThresholds {
        self.thresholds
    }

    /// Failure and deadline policy.
    #[must_use]
    pub const fn fail_policy(&self) -> &DecisionSchemaFailPolicy {
        &self.fail
    }

    /// Calibration provenance.
    #[must_use]
    pub const fn calibration(&self) -> &DecisionSchemaCalibration {
        &self.calibration
    }
}

/// Typed failure while resolving or validating a decision schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecisionSchemaError {
    /// Schema identifier or processing bounds are invalid.
    #[error("decision schema identity or bounds are invalid")]
    InvalidIdentityOrBounds,
    /// Question list is invalid or contains duplicate ids.
    #[error("decision schema questions are invalid")]
    InvalidQuestion,
    /// Probability boundaries are not ordered.
    #[error("decision schema thresholds are invalid")]
    InvalidThresholds,
    /// Consumer id or deadline is invalid.
    #[error("decision schema failure policy is invalid")]
    InvalidFailurePolicy,
    /// Calibration provenance is absent, malformed, or inconsistent.
    #[error("decision schema calibration provenance is invalid")]
    InvalidCalibration,
}

/// Read-only lookup for versioned decision schemas.
///
/// Values are owned snapshots so L0 callers cannot retain a borrow into an L3 catalog. `Err`
/// means that a catalog adapter found malformed canonical data; it must not be treated as absence.
pub trait DecisionSchemaCatalog: Send + Sync {
    /// Exact schema version.
    fn get(&self, id: &str, version: u32) -> Result<Option<DecisionSchema>, DecisionSchemaError>;

    /// Highest available version.
    fn latest(&self, id: &str) -> Result<Option<DecisionSchema>, DecisionSchemaError>;
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-sdk/src/system_one.rs"]
mod decision_schema_tests;
