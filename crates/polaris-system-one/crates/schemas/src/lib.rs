//! Versioned System-One schemas and deterministic threshold application.
pub mod calibration;
use polaris_sdk::system_one::{
    Answer, Decision, DecisionSchema, DecisionSchemaCalibration, DecisionSchemaCalibrationRecord,
    DecisionSchemaCalibrationState, DecisionSchemaCatalog as SdkDecisionSchemaCatalog,
    DecisionSchemaError as SdkDecisionSchemaError, DecisionSchemaFailMode,
    DecisionSchemaFailPolicy, DecisionSchemaIntegrity, DecisionSchemaThresholds, Probability,
    Question, QuestionId, QuestionKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
const BUILTINS: &str = include_str!("builtin.toml");
/// Decision consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Consumer {
    /// Command and diff safety checks.
    SafetyGate,
    /// Personally identifying information checks.
    PiiMask,
    /// File and chunk relevance.
    Triage,
    /// Tool selection and pruning.
    ToolPrune,
    /// Goal drift and loop monitoring.
    Watchdog,
    /// Search result filtering.
    Sifter,
    /// Effort routing.
    EffortRouter,
    /// Postflight verification.
    Postflight,
    /// Concurrent diff arbitration.
    DiffArbiter,
    /// Sandbox selection.
    SandboxRoute,
    /// Environment matching.
    EnvMatch,
    /// NVIDIA stack profiling.
    NvidiaProfile,
    /// Skill selection.
    SkillSelect,
}
/// Timeout behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailMode {
    /// Continue with a conservative caller default.
    Open,
    /// Require deterministic review before proceeding.
    Closed,
}
/// Integrity requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    /// Ordinary schema and decision validation.
    Normal,
    /// Require matched calibration evidence to accept safety-sensitive output.
    Strict,
}
/// Semantic category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    /// User request intent.
    Intent,
    /// Effort level.
    Effort,
    /// Tool relevance.
    ToolRelevance,
    /// File relevance.
    FileRelevance,
    /// Search chunk relevance.
    ChunkRelevance,
    /// Search snippet relevance.
    SnippetRelevance,
    /// Command safety.
    SafetyCommand,
    /// Diff tampering.
    DiffTampering,
    /// Semantic secret detection.
    SecretSemantic,
    /// Test tautology detection.
    Tautology,
    /// Goal congruence.
    Congruence,
    /// Semantic loop detection.
    LoopSemantic,
    /// Role boundary classification.
    RoleBoundary,
    /// Sandbox routing.
    SandboxRoute,
    /// Environment matching.
    EnvMatch,
    /// PII detection.
    Pii,
    /// Security finding arbitration.
    SecurityArbitration,
    /// NVIDIA stack classification.
    NvidiaStack,
    /// Skill relevance.
    SkillRelevance,
    /// Merge congruence.
    MergeCongruence,
}
/// Calibration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationStatus {
    /// No measured fixture calibration is available.
    Uncalibrated,
    /// Calibration exists and must match caller-provided digests.
    Calibrated,
}
/// Calibration provenance and quality measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationRecord {
    /// State.
    pub status: CalibrationStatus,
    /// Fixture collection name.
    pub fixture_set: String,
    /// Fixture BLAKE3 digest.
    pub fixture_blake3: Option<String>,
    /// Labelled sample count.
    pub n: u32,
    /// Target precision.
    pub precision_target: Option<f32>,
    /// Measured precision.
    pub observed_precision: Option<f32>,
    /// Measured recall.
    pub observed_recall: Option<f32>,
    /// Expected calibration error.
    pub ece: Option<f32>,
    /// Inference checkpoint BLAKE3 digest.
    pub checkpoint_blake3: Option<String>,
    /// RFC 3339 calibration date.
    pub date: Option<String>,
}
/// Active fixture and checkpoint digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationContext {
    /// Fixture digest.
    pub fixture_blake3: String,
    /// Checkpoint digest.
    pub checkpoint_blake3: String,
}
/// Probability boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    /// Acceptance lower bound.
    pub accept: Probability,
    /// Rejection upper bound.
    pub reject: Probability,
    /// Inclusive abstention interval.
    pub abstain_band: (Probability, Probability),
}
/// Per-consumer timeout policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailPolicy {
    /// Consumer.
    pub consumer: Consumer,
    /// Timeout/error behavior.
    pub on_timeout: FailMode,
    /// Deadline milliseconds.
    pub deadline_ms: u32,
    /// Integrity mode.
    pub integrity: Integrity,
}
/// Integer ratio used by deterministic watchdog thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchdogRatio {
    /// Threshold numerator.
    pub numerator: u32,
    /// Nonzero threshold denominator.
    pub denominator: u32,
}
/// Data-only deterministic watchdog bounds; this is not model calibration or authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeterministicWatchdogConfig {
    /// Configuration format version.
    pub version: u16,
    /// Maximum UTF-8 bytes in a proposed step.
    pub max_step_bytes: u32,
    /// Recent records retained per frame.
    pub window_size: u16,
    /// Tokens per normalized shingle.
    pub shingle_size: u16,
    /// Minimum Jaccard similarity for near-duplicate detection.
    pub near_duplicate: WatchdogRatio,
    /// Context-used/context-budget pressure threshold.
    pub pressure: WatchdogRatio,
    /// Repetition threshold for fatigue detection.
    pub repetition: WatchdogRatio,
    /// Require pressure and repetition together for fatigue.
    pub require_both_signals: bool,
}
/// Versioned decision schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    /// Identifier.
    pub id: String,
    /// Version.
    pub version: u32,
    /// Category.
    pub kind: SchemaKind,
    /// Primary input character limit.
    pub max_chars: u32,
    /// Questions returned in the decision.
    pub questions: Vec<Question>,
    /// Thresholds applied to answers.
    pub thresholds: Thresholds,
    /// Timeout policy.
    pub fail: FailPolicy,
    /// Calibration provenance.
    pub calibration: CalibrationRecord,
}
impl Schema {
    /// True when calibration digests match active caller digests.
    #[must_use]
    pub fn is_calibrated_for(&self, c: &CalibrationContext) -> bool {
        self.calibration.status == CalibrationStatus::Calibrated
            && self.calibration.fixture_blake3.as_deref() == Some(c.fixture_blake3.as_str())
            && self.calibration.checkpoint_blake3.as_deref() == Some(c.checkpoint_blake3.as_str())
    }
    /// Apply thresholds to a complete decision.
    ///
    /// # Errors
    /// Returns an error for malformed decision data.
    pub fn apply(&self, d: &Decision) -> Result<Vec<(QuestionId, Verdict)>, SchemaError> {
        apply_threshold(self, d)
    }
}
/// Result for one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Meets acceptance boundary.
    Accept,
    /// Meets rejection boundary.
    Reject,
    /// Lies between boundaries.
    Abstain,
    /// Strict acceptance needs matched calibration.
    NeedsApproval,
}
/// Schema parsing, validation, and application errors.
#[derive(Debug, Error)]
pub enum SchemaError {
    /// TOML parse error.
    #[error("schema TOML parse failed: {0}")]
    Parse(String),
    /// Duplicate identifier and version.
    #[error("duplicate schema {id} version {version}")]
    DuplicateId { id: String, version: u32 },
    /// Invalid schema.
    #[error("invalid schema {id}: {reason}")]
    InvalidSchema { id: String, reason: String },
    /// Threshold order error.
    #[error("threshold order must be reject <= abstain.low <= abstain.high <= accept")]
    ThresholdOrder,
    /// Stale calibration.
    #[error("calibration stale or incomplete: {id}")]
    CalibrationStale { id: String },
    /// Decision question set differs.
    #[error("decision question set invalid for {id}: {question}")]
    DecisionQuestionSet { id: String, question: String },
    /// Answer shape differs from question.
    #[error("answer shape invalid for {id}/{question}: {reason}")]
    DecisionShape {
        id: String,
        question: String,
        reason: String,
    },
    /// Invalid choice distribution.
    #[error("invalid distribution for {id}/{question}")]
    InvalidDistribution { id: String, question: String },
    /// Repository override weakens base.
    #[error("repository override weakens {id}: {reason}")]
    OverrideWeakens { id: String, reason: String },
}
/// Synchronous read-only schema lookup.
pub trait SchemaCatalog: Send + Sync {
    /// Exact version.
    fn get(&self, id: &str, version: u32) -> Option<&Schema>;
    /// Highest available version.
    fn latest(&self, id: &str) -> Option<&Schema>;
}
/// Schema set indexed by identifier and version.
#[derive(Debug, Clone, Default)]
pub struct SchemaSet {
    schemas: BTreeMap<(String, u32), Schema>,
    watchdog: Option<DeterministicWatchdogConfig>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaDocument {
    schemas: Vec<Schema>,
    #[serde(default)]
    watchdog: Option<DeterministicWatchdogConfig>,
}
impl SchemaSet {
    /// Load embedded production schemas.
    ///
    /// # Errors
    /// Returns parse or validation errors.
    pub fn load_builtin() -> Result<Self, SchemaError> {
        Self::from_toml(BUILTINS)
    }
    /// Parse a TOML document with a schemas array.
    ///
    /// # Errors
    /// Returns parse or validation errors.
    pub fn from_toml(src: &str) -> Result<Self, SchemaError> {
        if src.len() > 1_048_576 {
            return Err(SchemaError::InvalidSchema {
                id: "<document>".into(),
                reason: "schema document exceeds processing bound".into(),
            });
        }
        let doc: SchemaDocument =
            toml::from_str(src).map_err(|e| SchemaError::Parse(e.to_string()))?;
        if doc.schemas.len() > 256 {
            return Err(SchemaError::InvalidSchema {
                id: "<document>".into(),
                reason: "too many schemas".into(),
            });
        }
        let mut schemas = BTreeMap::new();
        for s in doc.schemas {
            validate_schema(&s)?;
            let k = (s.id.clone(), s.version);
            if schemas.insert(k, s.clone()).is_some() {
                return Err(SchemaError::DuplicateId {
                    id: s.id,
                    version: s.version,
                });
            }
        }
        if schemas.is_empty() {
            return Err(SchemaError::InvalidSchema {
                id: "<document>".into(),
                reason: "empty schema set".into(),
            });
        }
        validate_watchdog(doc.watchdog)?;
        Ok(Self {
            schemas,
            watchdog: doc.watchdog,
        })
    }
    /// Exact version lookup.
    #[must_use]
    pub fn get(&self, id: &str, v: u32) -> Option<&Schema> {
        self.schemas.get(&(id.to_owned(), v))
    }
    /// Highest version lookup.
    #[must_use]
    pub fn latest(&self, id: &str) -> Option<&Schema> {
        self.schemas
            .iter()
            .filter(|((key, _), _)| key == id)
            .max_by_key(|((_, v), _)| v)
            .map(|(_, s)| s)
    }
    /// Validated deterministic watchdog configuration, when supplied by this schema document.
    #[must_use]
    pub const fn watchdog_config(&self) -> Option<&DeterministicWatchdogConfig> {
        self.watchdog.as_ref()
    }
    /// Validate all records.
    ///
    /// # Errors
    /// Returns first invalid schema.
    pub fn validate(&self) -> Result<(), SchemaError> {
        validate_watchdog(self.watchdog)?;
        for s in self.schemas.values() {
            validate_schema(s)?;
        }
        Ok(())
    }
    /// Check calibration against caller digests.
    ///
    /// # Errors
    /// Returns stale calibration if a digest is invalid or mismatched.
    pub fn validate_calibration(&self, c: &CalibrationContext) -> Result<(), SchemaError> {
        digest(&c.fixture_blake3, "<context>")?;
        digest(&c.checkpoint_blake3, "<context>")?;
        for s in self.schemas.values() {
            if s.calibration.status == CalibrationStatus::Calibrated && !s.is_calibrated_for(c) {
                return Err(SchemaError::CalibrationStale { id: s.id.clone() });
            }
        }
        Ok(())
    }
    /// Apply repository overrides that only tighten behavior.
    ///
    /// # Errors
    /// Returns OverrideWeakens if an override broadens policy.
    pub fn with_repository_overrides(&self, o: &SchemaSet) -> Result<Self, SchemaError> {
        if o.watchdog.is_some() && o.watchdog != self.watchdog {
            return Err(weak("watchdog", "deterministic watchdog configuration"));
        }
        let mut out = self.clone();
        for ((id, v), c) in &o.schemas {
            let Some(b) = self.schemas.get(&(id.clone(), *v)) else {
                return Err(weak(id, "new schema version"));
            };
            override_check(b, c)?;
            out.schemas.insert((id.clone(), *v), c.clone());
        }
        Ok(out)
    }
}

fn validate_watchdog(config: Option<DeterministicWatchdogConfig>) -> Result<(), SchemaError> {
    let Some(config) = config else {
        return Ok(());
    };
    let valid_ratio = |ratio: WatchdogRatio, allow_zero: bool| {
        ratio.denominator > 0
            && ratio.numerator <= ratio.denominator
            && (allow_zero || ratio.numerator > 0)
    };
    if config.version != 1
        || config.max_step_bytes == 0
        || config.max_step_bytes > 1_048_576
        || config.window_size == 0
        || config.window_size > 128
        || config.shingle_size == 0
        || config.shingle_size > 128
        || !valid_ratio(config.near_duplicate, false)
        || !valid_ratio(config.pressure, true)
        || !valid_ratio(config.repetition, true)
    {
        return Err(SchemaError::InvalidSchema {
            id: "watchdog".to_owned(),
            reason: "deterministic watchdog configuration is outside bounds".to_owned(),
        });
    }
    Ok(())
}
impl SchemaCatalog for SchemaSet {
    fn get(&self, id: &str, v: u32) -> Option<&Schema> {
        SchemaSet::get(self, id, v)
    }
    fn latest(&self, id: &str) -> Option<&Schema> {
        SchemaSet::latest(self, id)
    }
}

/// L0 SDK view over the canonical validated L3 schema set.
#[derive(Debug, Clone, Copy)]
pub struct SdkSchemaCatalog<'a> {
    schemas: &'a SchemaSet,
}

impl<'a> SdkSchemaCatalog<'a> {
    /// Wrap a canonical schema set without copying or adding a layer dependency.
    #[must_use]
    pub const fn new(schemas: &'a SchemaSet) -> Self {
        Self { schemas }
    }
}

impl SdkDecisionSchemaCatalog for SdkSchemaCatalog<'_> {
    fn get(
        &self,
        id: &str,
        version: u32,
    ) -> Result<Option<DecisionSchema>, SdkDecisionSchemaError> {
        self.schemas.get(id, version).map(to_sdk_schema).transpose()
    }

    fn latest(&self, id: &str) -> Result<Option<DecisionSchema>, SdkDecisionSchemaError> {
        self.schemas.latest(id).map(to_sdk_schema).transpose()
    }
}

fn to_sdk_schema(schema: &Schema) -> Result<DecisionSchema, SdkDecisionSchemaError> {
    let thresholds = DecisionSchemaThresholds::new(
        schema.thresholds.accept,
        schema.thresholds.reject,
        schema.thresholds.abstain_band.0,
        schema.thresholds.abstain_band.1,
    )?;
    let fail = DecisionSchemaFailPolicy::new(
        consumer_id(schema.fail.consumer).to_owned(),
        match schema.fail.on_timeout {
            FailMode::Open => DecisionSchemaFailMode::Open,
            FailMode::Closed => DecisionSchemaFailMode::Closed,
        },
        schema.fail.deadline_ms,
        match schema.fail.integrity {
            Integrity::Normal => DecisionSchemaIntegrity::Normal,
            Integrity::Strict => DecisionSchemaIntegrity::Strict,
        },
    )?;
    let calibration = DecisionSchemaCalibration::try_new(DecisionSchemaCalibrationRecord {
        state: match schema.calibration.status {
            CalibrationStatus::Uncalibrated => DecisionSchemaCalibrationState::Uncalibrated,
            CalibrationStatus::Calibrated => DecisionSchemaCalibrationState::Calibrated,
        },
        fixture_set: schema.calibration.fixture_set.clone(),
        fixture_blake3: schema.calibration.fixture_blake3.clone(),
        sample_count: schema.calibration.n,
        precision_target: schema.calibration.precision_target,
        observed_precision: schema.calibration.observed_precision,
        observed_recall: schema.calibration.observed_recall,
        expected_calibration_error: schema.calibration.ece,
        checkpoint_blake3: schema.calibration.checkpoint_blake3.clone(),
        date: schema.calibration.date.clone(),
    })?;
    DecisionSchema::try_new(
        schema.id.clone(),
        schema.version,
        schema.max_chars,
        schema.questions.clone(),
        thresholds,
        fail,
        calibration,
    )
}

fn consumer_id(consumer: Consumer) -> &'static str {
    match consumer {
        Consumer::SafetyGate => "safety_gate",
        Consumer::PiiMask => "pii_mask",
        Consumer::Triage => "triage",
        Consumer::ToolPrune => "tool_prune",
        Consumer::Watchdog => "watchdog",
        Consumer::Sifter => "sifter",
        Consumer::EffortRouter => "effort_router",
        Consumer::Postflight => "postflight",
        Consumer::DiffArbiter => "diff_arbiter",
        Consumer::SandboxRoute => "sandbox_route",
        Consumer::EnvMatch => "env_match",
        Consumer::NvidiaProfile => "nvidia_profile",
        Consumer::SkillSelect => "skill_select",
    }
}
/// Apply thresholds without active calibration context.
///
/// Strict schemas cannot Accept without digest matching.
///
/// # Errors
/// Returns error for invalid decision shape.
pub fn apply_threshold(
    s: &Schema,
    d: &Decision,
) -> Result<Vec<(QuestionId, Verdict)>, SchemaError> {
    apply_threshold_with_context(s, d, None)
}
/// Apply thresholds with optional active calibration context.
///
/// # Errors
/// Returns error for invalid schema or decision shape.
pub fn apply_threshold_with_context(
    s: &Schema,
    d: &Decision,
    c: Option<&CalibrationContext>,
) -> Result<Vec<(QuestionId, Verdict)>, SchemaError> {
    validate_schema(s)?;
    let a = d
        .answers
        .iter()
        .map(|x| (x.question.clone(), &x.answer))
        .collect::<BTreeMap<_, _>>();
    if a.len() != d.answers.len() {
        return Err(SchemaError::DecisionQuestionSet {
            id: s.id.clone(),
            question: "duplicate answer".into(),
        });
    }
    let expected = s
        .questions
        .iter()
        .map(|q| q.id.clone())
        .collect::<BTreeSet<_>>();
    if expected.len() != s.questions.len() || expected.len() != a.len() {
        return Err(SchemaError::DecisionQuestionSet {
            id: s.id.clone(),
            question: "answer count differs".into(),
        });
    }
    let mut out = Vec::with_capacity(s.questions.len());
    for q in &s.questions {
        let Some(ans) = a.get(&q.id) else {
            return Err(SchemaError::DecisionQuestionSet {
                id: s.id.clone(),
                question: q.id.to_string(),
            });
        };
        let p = answer_probability(s, q, ans)?;
        let mut v = if p <= s.thresholds.reject {
            Verdict::Reject
        } else if p >= s.thresholds.accept {
            Verdict::Accept
        } else {
            Verdict::Abstain
        };
        if v == Verdict::Accept
            && s.fail.integrity == Integrity::Strict
            && !c.is_some_and(|ctx| s.is_calibrated_for(ctx))
        {
            v = Verdict::NeedsApproval;
        }
        out.push((q.id.clone(), v));
    }
    if a.keys().any(|id| !expected.contains(id)) {
        return Err(SchemaError::DecisionQuestionSet {
            id: s.id.clone(),
            question: "unexpected answer".into(),
        });
    }
    Ok(out)
}
fn answer_probability(s: &Schema, q: &Question, a: &Answer) -> Result<Probability, SchemaError> {
    let bad = |reason: &str| SchemaError::DecisionShape {
        id: s.id.clone(),
        question: q.id.to_string(),
        reason: reason.into(),
    };
    match (&q.kind, a) {
        (QuestionKind::Choice { options }, Answer::Choice { selected, probs }) => {
            let opts = options.iter().cloned().collect::<BTreeSet<_>>();
            if opts.len() != options.len() || !opts.contains(selected) {
                return Err(bad("invalid option set or selection"));
            }
            let mut scores = BTreeMap::new();
            for p in probs {
                if scores.insert(p.label.clone(), p.probability).is_some() {
                    return Err(SchemaError::InvalidDistribution {
                        id: s.id.clone(),
                        question: q.id.to_string(),
                    });
                }
            }
            if scores.len() != opts.len()
                || scores.keys().any(|l| !opts.contains(l))
                || opts.iter().any(|l| !scores.contains_key(l))
            {
                return Err(SchemaError::InvalidDistribution {
                    id: s.id.clone(),
                    question: q.id.to_string(),
                });
            }
            if probs
                .iter()
                .zip(probs.iter().skip(1))
                .any(|(previous, next)| previous.probability < next.probability)
            {
                return Err(SchemaError::InvalidDistribution {
                    id: s.id.clone(),
                    question: q.id.to_string(),
                });
            }
            let selected =
                scores
                    .get(selected)
                    .copied()
                    .ok_or_else(|| SchemaError::InvalidDistribution {
                        id: s.id.clone(),
                        question: q.id.to_string(),
                    })?;
            let max =
                scores
                    .values()
                    .copied()
                    .max()
                    .ok_or_else(|| SchemaError::InvalidDistribution {
                        id: s.id.clone(),
                        question: q.id.to_string(),
                    })?;
            let sum = scores.values().map(|p| p.get()).sum::<f32>();
            if selected != max || !sum.is_finite() || (sum - 1.0).abs() > 0.001 {
                return Err(SchemaError::InvalidDistribution {
                    id: s.id.clone(),
                    question: q.id.to_string(),
                });
            }
            Ok(selected)
        }
        (QuestionKind::Score { min, max }, Answer::Score { value, confidence }) => {
            if min > max || value < min || value > max {
                return Err(bad("score out of range"));
            }
            Ok(*confidence)
        }
        (QuestionKind::Noul { .. }, Answer::Noul { yes }) => Ok(*yes),
        (QuestionKind::Choice { .. }, _) => Err(bad("expected choice")),
        (QuestionKind::Score { .. }, _) => Err(bad("expected score")),
        (QuestionKind::Noul { .. }, _) => Err(bad("expected noul")),
    }
}
fn validate_schema(s: &Schema) -> Result<(), SchemaError> {
    if s.id.trim().is_empty()
        || s.id.len() > 128
        || s.id.chars().any(char::is_control)
        || s.version == 0
        || s.max_chars == 0
        || s.max_chars > 1_048_576
        || s.questions.is_empty()
        || s.questions.len() > 64
    {
        return Err(SchemaError::InvalidSchema {
            id: s.id.clone(),
            reason: "invalid schema identity or processing bounds".into(),
        });
    }
    let t = s.thresholds;
    if t.reject > t.abstain_band.0
        || t.abstain_band.0 > t.abstain_band.1
        || t.abstain_band.1 > t.accept
    {
        return Err(SchemaError::ThresholdOrder);
    }
    let mut ids = BTreeSet::new();
    for q in &s.questions {
        if !ids.insert(q.id.clone()) {
            return Err(SchemaError::InvalidSchema {
                id: s.id.clone(),
                reason: format!("duplicate question {}", q.id),
            });
        }
        match &q.kind {
            QuestionKind::Choice { options }
                if options.is_empty()
                    || options.len() > 128
                    || options.iter().collect::<BTreeSet<_>>().len() != options.len() =>
            {
                return Err(SchemaError::InvalidSchema {
                    id: s.id.clone(),
                    reason: format!("bad options {}", q.id),
                });
            }
            QuestionKind::Score { min, max }
                if min > max || i64::from(*max).saturating_sub(i64::from(*min)) > 256 =>
            {
                return Err(SchemaError::InvalidSchema {
                    id: s.id.clone(),
                    reason: format!("bad range {}", q.id),
                });
            }
            QuestionKind::Noul { statement }
                if statement.trim().is_empty() || statement.len() > 4096 =>
            {
                return Err(SchemaError::InvalidSchema {
                    id: s.id.clone(),
                    reason: format!("invalid statement {}", q.id),
                });
            }
            _ => {}
        }
    }
    if s.fail.deadline_ms == 0 {
        return Err(SchemaError::InvalidSchema {
            id: s.id.clone(),
            reason: "zero deadline".into(),
        });
    }
    calibration(s)
}
fn calibration(s: &Schema) -> Result<(), SchemaError> {
    let r = &s.calibration;
    let vals = [
        r.precision_target,
        r.observed_precision,
        r.observed_recall,
        r.ece,
    ];
    if vals
        .iter()
        .flatten()
        .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
    {
        return Err(SchemaError::CalibrationStale { id: s.id.clone() });
    }
    if r.status == CalibrationStatus::Uncalibrated {
        if r.n != 0
            || r.fixture_blake3.is_some()
            || r.checkpoint_blake3.is_some()
            || r.observed_precision.is_some()
            || r.observed_recall.is_some()
            || r.ece.is_some()
            || r.date.is_some()
        {
            return Err(SchemaError::CalibrationStale { id: s.id.clone() });
        }
        return Ok(());
    }
    let Some(f) = r.fixture_blake3.as_deref() else {
        return Err(SchemaError::CalibrationStale { id: s.id.clone() });
    };
    let Some(c) = r.checkpoint_blake3.as_deref() else {
        return Err(SchemaError::CalibrationStale { id: s.id.clone() });
    };
    digest(f, &s.id)?;
    digest(c, &s.id)?;
    if r.fixture_set.trim().is_empty()
        || r.n == 0
        || r.precision_target.is_none()
        || r.observed_precision.is_none()
        || r.observed_recall.is_none()
        || r.ece.is_none()
        || r.date.as_deref().is_none_or(|d| !rfc3339(d))
    {
        return Err(SchemaError::CalibrationStale { id: s.id.clone() });
    }
    if let (Some(target), Some(observed)) = (r.precision_target, r.observed_precision)
        && observed < target
    {
        return Err(SchemaError::CalibrationStale { id: s.id.clone() });
    }
    Ok(())
}
fn digest(d: &str, id: &str) -> Result<(), SchemaError> {
    if d.len() != 64
        || d.chars()
            .any(|c| !c.is_ascii_digit() && !("abcdef".contains(c)))
    {
        return Err(SchemaError::CalibrationStale { id: id.into() });
    }
    Ok(())
}
fn rfc3339(d: &str) -> bool {
    d.parse::<polaris_sdk::contracts::Timestamp>().is_ok()
}
fn weak(id: &str, r: &str) -> SchemaError {
    SchemaError::OverrideWeakens {
        id: id.into(),
        reason: r.into(),
    }
}
fn override_check(b: &Schema, c: &Schema) -> Result<(), SchemaError> {
    validate_schema(c)?;
    if c.id != b.id || c.version != b.version || c.kind != b.kind || c.questions != b.questions {
        return Err(weak(&b.id, "identity/questions immutable"));
    }
    if c.max_chars > b.max_chars
        || c.thresholds.accept < b.thresholds.accept
        || c.thresholds.reject < b.thresholds.reject
        || c.thresholds.abstain_band.0 > b.thresholds.abstain_band.0
        || c.thresholds.abstain_band.1 < b.thresholds.abstain_band.1
    {
        return Err(weak(&b.id, "thresholds/input limits may only tighten"));
    }
    if b.fail.on_timeout == FailMode::Closed && c.fail.on_timeout != FailMode::Closed
        || b.fail.integrity == Integrity::Strict && c.fail.integrity != Integrity::Strict
        || c.fail.deadline_ms > b.fail.deadline_ms
        || c.fail.consumer != b.fail.consumer
    {
        return Err(weak(&b.id, "failure policy may only tighten"));
    }
    if b.calibration.status == CalibrationStatus::Calibrated && b.calibration != c.calibration {
        return Err(weak(&b.id, "calibration provenance immutable"));
    }
    Ok(())
}
