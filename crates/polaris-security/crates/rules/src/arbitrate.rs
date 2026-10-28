//! Fail-closed S1 arbitration bound to the canonical security schema and masked evidence.

use crate::model::Finding;
use crate::{RuleEngine, RulesError};
use polaris_code_intel::ByteSpan;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::DecisionEngine;
use polaris_sdk::system_one::{
    Answer, DecisionInput, DecisionSchema, DecisionSchemaCalibrationState, DecisionSchemaFailMode,
    DecisionSchemaIntegrity, QuestionKind,
};
use std::fmt;
use std::time::{Duration, Instant};

const MAX_ARBITRATION_FINDINGS: usize = 32;
const MAX_ARBITRATION_BUDGET: Duration = Duration::from_secs(5);
const MAX_EVIDENCE_TEXT_BYTES: usize = 16 * 1024;
const CANONICAL_SCHEMA_ID: &str = "sec.arbitrate";
const CANONICAL_SCHEMA_VERSION: u32 = 1;
const CANONICAL_QUESTION_ID: &str = "finding_valid";
const CANONICAL_QUESTION_TEXT: &str =
    "The reported security finding is supported by the available evidence.";
const CANONICAL_CONSUMER: &str = "diff_arbiter";
const CANONICAL_ACCEPT: f32 = 0.99;
const CANONICAL_REJECT: f32 = 0.01;

/// Outcome of an S1 arbitration attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitrationStatus {
    /// No disputed findings required a model call.
    NoDisputedFindings,
    /// The schema has no measured calibration; every finding was retained.
    Uncalibrated,
    /// The schema does not exactly match the canonical security-arbitration contract.
    UntrustedSchema,
    /// Calibration fixture or model checkpoint does not match host-pinned provenance.
    UntrustedCalibration,
    /// Masked source evidence is missing, duplicated, or does not bind to a finding CID/span.
    MissingEvidence,
    /// A call timed out; unresolved findings were retained.
    TimedOut,
    /// S1 returned an error; unresolved findings were retained.
    EngineError,
    /// S1 returned an invalid or ambiguous answer; unresolved findings were retained.
    InvalidDecision,
    /// Calibrated high-confidence unsupported findings were removed from the result.
    Applied,
    /// The per-request arbitration budget was capped at five seconds.
    BudgetCapped,
}

/// Findings after optional S1 arbitration, plus an explicit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbitrationReport {
    findings: Vec<Finding>,
    status: ArbitrationStatus,
    removed: usize,
}

impl ArbitrationReport {
    /// Findings that remain authoritative after arbitration.
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Arbitration status.
    #[must_use]
    pub const fn status(&self) -> ArbitrationStatus {
        self.status
    }

    /// Number of disputed findings removed by calibrated high-confidence evidence review.
    #[must_use]
    pub const fn removed(&self) -> usize {
        self.removed
    }
}

/// Host-pinned calibration digests required before a calibrated schema can remove findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbitrationTrust {
    fixture_blake3: String,
    checkpoint_blake3: String,
}

impl ArbitrationTrust {
    /// Creates a trust context from host-pinned fixture and model-checkpoint BLAKE3 digests.
    ///
    /// # Errors
    /// Returns a bounds error unless both values are lowercase 64-character BLAKE3 digests.
    pub fn new(
        fixture_blake3: impl Into<String>,
        checkpoint_blake3: impl Into<String>,
    ) -> Result<Self, RulesError> {
        let fixture_blake3 = fixture_blake3.into();
        let checkpoint_blake3 = checkpoint_blake3.into();
        if !valid_digest(&fixture_blake3) || !valid_digest(&checkpoint_blake3) {
            return Err(RulesError::Bound(String::from(
                "arbitration trust digests must be lowercase BLAKE3 hex",
            )));
        }
        Ok(Self {
            fixture_blake3,
            checkpoint_blake3,
        })
    }
}

/// Caller-attested masked evidence bound to one exact finding content id and byte range.
///
/// `masked_excerpt` and `masked_context` are the only source-derived material sent to S1.
/// The caller is responsible for applying its authoritative secret/PII masker before construction.
#[derive(Clone, PartialEq, Eq)]
pub struct ArbitrationEvidence {
    snippet_digest: Cid,
    span: ByteSpan,
    masked_excerpt: String,
    masked_context: Option<String>,
}

impl fmt::Debug for ArbitrationEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArbitrationEvidence")
            .field("snippet_digest", &self.snippet_digest)
            .field("span", &self.span)
            .field("masked_excerpt", &"<redacted>")
            .field(
                "masked_context",
                &self.masked_context.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl ArbitrationEvidence {
    /// Constructs bounded evidence after the caller has masked source and surrounding context.
    ///
    /// # Errors
    /// Returns a bounds error when evidence is empty or exceeds the maximum input size.
    pub fn new(
        snippet_digest: Cid,
        span: ByteSpan,
        masked_excerpt: String,
        masked_context: Option<String>,
    ) -> Result<Self, RulesError> {
        let total_bytes = masked_excerpt
            .len()
            .checked_add(masked_context.as_ref().map_or(0, String::len))
            .ok_or_else(|| {
                RulesError::Bound(String::from("arbitration evidence length overflow"))
            })?;
        if span.is_empty()
            || total_bytes > MAX_EVIDENCE_TEXT_BYTES
            || masked_excerpt.trim().is_empty()
        {
            return Err(RulesError::Bound(String::from(
                "arbitration evidence is empty or exceeds its bound",
            )));
        }
        Ok(Self {
            snippet_digest,
            span,
            masked_excerpt,
            masked_context,
        })
    }
}

/// Uses only the canonical calibrated security schema and matching masked evidence to arbitrate.
impl RuleEngine {
    /// Removes a disputed finding only when S1 confidently rejects its support using the exact
    /// canonical `sec.arbitrate` question and host-pinned fixture/checkpoint provenance.
    ///
    /// # Errors
    /// Returns a typed error when decision input cannot be represented within the canonical
    /// schema's bound. Untrusted schemas, evidence, calibration and S1 failures preserve findings
    /// and are reported in [`ArbitrationStatus`].
    pub async fn arbitrate<D: DecisionEngine>(
        &self,
        findings: &[Finding],
        evidence: &[ArbitrationEvidence],
        s1: &D,
        schema: &DecisionSchema,
        trust: &ArbitrationTrust,
        timeout: Duration,
    ) -> Result<ArbitrationReport, RulesError> {
        validate_finding_count(findings)?;
        let mut report = ArbitrationReport {
            findings: findings.to_vec(),
            status: ArbitrationStatus::NoDisputedFindings,
            removed: 0,
        };
        if !findings.iter().any(Finding::disputed) {
            return Ok(report);
        }
        if !canonical_schema(schema) {
            report.status = ArbitrationStatus::UntrustedSchema;
            return Ok(report);
        }
        if schema.calibration().state() != DecisionSchemaCalibrationState::Calibrated {
            report.status = ArbitrationStatus::Uncalibrated;
            return Ok(report);
        }
        if schema.calibration().fixture_blake3() != Some(trust.fixture_blake3.as_str())
            || schema.calibration().checkpoint_blake3() != Some(trust.checkpoint_blake3.as_str())
        {
            report.status = ArbitrationStatus::UntrustedCalibration;
            return Ok(report);
        }
        if !evidence_matches(findings, evidence) {
            report.status = ArbitrationStatus::MissingEvidence;
            return Ok(report);
        }
        let schema_budget = Duration::from_millis(u64::from(schema.fail_policy().deadline_ms()));
        let Some((deadline, capped)) = arbitration_deadline(timeout, schema_budget) else {
            report.status = ArbitrationStatus::BudgetCapped;
            return Ok(report);
        };
        if capped {
            report.status = ArbitrationStatus::BudgetCapped;
        }
        let question = canonical_question(schema)?;
        let mut index = 0_usize;
        while index < report.findings.len() {
            let Some(finding) = report.findings.get(index) else {
                report.status = ArbitrationStatus::InvalidDecision;
                break;
            };
            if !finding.disputed() {
                index = index.saturating_add(1);
                continue;
            }
            let Some(matched_evidence) = evidence.iter().find(|item| {
                item.snippet_digest == finding.snippet_digest() && item.span == finding.span()
            }) else {
                report.status = ArbitrationStatus::MissingEvidence;
                break;
            };
            let input = decision_input(finding, matched_evidence, schema.max_chars())?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                report.status = ArbitrationStatus::TimedOut;
                break;
            }
            let decision =
                tokio::time::timeout(remaining, s1.decide(schema.questions(), &input)).await;
            match decision {
                Err(_) => {
                    report.status = ArbitrationStatus::TimedOut;
                    break;
                }
                Ok(Err(_)) => {
                    report.status = ArbitrationStatus::EngineError;
                    break;
                }
                Ok(Ok(decision)) => {
                    let Some(answer) = decision.answers.first() else {
                        report.status = ArbitrationStatus::InvalidDecision;
                        break;
                    };
                    if decision.answers.len() != 1 || answer.question != question.id {
                        report.status = ArbitrationStatus::InvalidDecision;
                        break;
                    }
                    match answer.answer {
                        Answer::Noul { yes } if yes.get() <= schema.thresholds().reject().get() => {
                            let removed = report.findings.remove(index);
                            drop(removed);
                            report.removed = report.removed.saturating_add(1);
                            report.status = ArbitrationStatus::Applied;
                        }
                        Answer::Noul { .. } => {
                            index = index.saturating_add(1);
                        }
                        Answer::Choice { .. } | Answer::Score { .. } => {
                            report.status = ArbitrationStatus::InvalidDecision;
                            break;
                        }
                    }
                }
            }
        }
        Ok(report)
    }
}

fn arbitration_deadline(timeout: Duration, schema_budget: Duration) -> Option<(Instant, bool)> {
    let budget = timeout.min(MAX_ARBITRATION_BUDGET).min(schema_budget);
    Instant::now()
        .checked_add(budget)
        .map(|deadline| (deadline, budget < timeout))
}

fn validate_finding_count(findings: &[Finding]) -> Result<(), RulesError> {
    if findings.len() > MAX_ARBITRATION_FINDINGS {
        return Err(RulesError::Bound(String::from(
            "arbitration finding count exceeds its bound",
        )));
    }
    Ok(())
}

fn canonical_question(
    schema: &DecisionSchema,
) -> Result<&polaris_sdk::system_one::Question, RulesError> {
    schema
        .questions()
        .first()
        .ok_or_else(|| RulesError::Arbitration(String::from("canonical schema question missing")))
}

fn canonical_schema(schema: &DecisionSchema) -> bool {
    if schema.id() != CANONICAL_SCHEMA_ID
        || schema.version() != CANONICAL_SCHEMA_VERSION
        || schema.fail_policy().consumer() != CANONICAL_CONSUMER
        || schema.fail_policy().on_timeout() != DecisionSchemaFailMode::Closed
        || schema.fail_policy().integrity() != DecisionSchemaIntegrity::Strict
        || schema.questions().len() != 1
        || schema.thresholds().accept().get() != CANONICAL_ACCEPT
        || schema.thresholds().reject().get() != CANONICAL_REJECT
    {
        return false;
    }
    let Some(question) = schema.questions().first() else {
        return false;
    };
    if question.id.as_str() != CANONICAL_QUESTION_ID {
        return false;
    }
    matches!(&question.kind, QuestionKind::Noul { statement } if statement == CANONICAL_QUESTION_TEXT)
}

fn evidence_matches(findings: &[Finding], evidence: &[ArbitrationEvidence]) -> bool {
    let disputed = findings.iter().filter(|finding| finding.disputed()).count();
    if evidence.len() != disputed {
        return false;
    }
    let mut used = Vec::with_capacity(evidence.len());
    for finding in findings.iter().filter(|finding| finding.disputed()) {
        let Some(index) = evidence.iter().position(|item| {
            item.snippet_digest == finding.snippet_digest() && item.span == finding.span()
        }) else {
            return false;
        };
        if used.contains(&index) {
            return false;
        }
        used.push(index);
    }
    true
}

fn decision_input(
    finding: &Finding,
    evidence: &ArbitrationEvidence,
    max_chars: u32,
) -> Result<DecisionInput, RulesError> {
    let cwe = finding.cwe().map_or_else(
        || String::from("unmapped"),
        |cwe| format!("CWE-{}", cwe.get()),
    );
    let text = format!(
        "rule={} category={} severity={:?} cwe={}\nmasked finding evidence: {}",
        finding.rule().as_str(),
        finding.category().as_str(),
        finding.severity(),
        cwe,
        evidence.masked_excerpt
    );
    if text.chars().count()
        > usize::try_from(max_chars)
            .map_err(|_| RulesError::Bound(String::from("schema input bound conversion failed")))?
    {
        return Err(RulesError::Bound(String::from(
            "masked finding evidence exceeds schema input bound",
        )));
    }
    Ok(DecisionInput {
        text,
        context: evidence.masked_context.clone(),
    })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
