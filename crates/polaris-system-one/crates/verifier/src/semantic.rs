//! Bounded, masked inference and calibrated safety acceptance.
use crate::{
    DegradedReason, Finding, GateVerdict, MutationSafety, SafetyAssessment, SafetyRequest, Scanner,
    VerifierError,
};
use polaris_code_intel::ParserPool;
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{DecisionInput, QuestionKind},
};
use polaris_security_automata::EntropyCfg;
use polaris_system_one_schemas::{
    CalibrationContext, SchemaCatalog, Verdict, apply_threshold_with_context,
};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::Semaphore;

/// Expected calibration provenance injected by the composition root.
#[derive(Debug, Clone, Default)]
pub struct CalibrationBindings {
    contexts: BTreeMap<String, CalibrationContext>,
}

impl CalibrationBindings {
    /// Bind a schema id to trusted fixture/checkpoint identities, not model-provided claims.
    pub fn insert(
        &mut self,
        id: String,
        context: CalibrationContext,
    ) -> Option<CalibrationContext> {
        self.contexts.insert(id, context)
    }
    /// Look up the active expected identities for a schema.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CalibrationContext> {
        self.contexts.get(id)
    }

    /// Resolve the common exact native checkpoint identity expected by these calibrations.
    /// Configured production consumers use this to fence inference against actual worker health.
    /// # Errors
    /// Rejects malformed digests or contexts referring to different native checkpoints.
    pub fn checkpoint_identity(&self) -> Result<Option<[u8; 32]>, VerifierError> {
        let mut expected = None;
        for context in self.contexts.values() {
            let text = &context.checkpoint_blake3;
            if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(VerifierError::InvalidInput {
                    reason: "invalid calibration checkpoint identity",
                });
            }
            let mut digest = [0; 32];
            for (slot, pair) in digest.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
                let pair = std::str::from_utf8(pair).map_err(|_| VerifierError::InvalidInput {
                    reason: "invalid calibration checkpoint identity",
                })?;
                *slot = u8::from_str_radix(pair, 16).map_err(|_| VerifierError::InvalidInput {
                    reason: "invalid calibration checkpoint identity",
                })?;
            }
            if expected.is_some_and(|previous| previous != digest) {
                return Err(VerifierError::InvalidInput {
                    reason: "calibration checkpoints differ",
                });
            }
            expected = Some(digest);
        }
        Ok(expected)
    }
}

/// Processing bounds. Defaults are operational limits, not measured performance claims.
#[derive(Debug, Clone, Copy)]
pub struct VerifierConfig {
    /// Largest individual source snapshot or command.
    pub max_input_bytes: usize,
    /// Maximum matches in any one signature/entropy pass.
    pub max_scan_matches: usize,
    /// Maximum concurrently active inference calls; excess work asks for approval.
    pub max_inflight: usize,
    /// Upper bound on any schema deadline.
    pub max_decision_time: Duration,
    /// Entropy detector policy; heuristic until calibrated on the user's repositories.
    pub entropy: EntropyCfg,
    /// Maximum diagnostics retained in a postflight report.
    pub diagnostic_limit: usize,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            max_input_bytes: 4 * 1024 * 1024,
            max_scan_matches: 8192,
            max_inflight: 8,
            max_decision_time: Duration::from_secs(5),
            entropy: EntropyCfg::default(),
            diagnostic_limit: 64,
        }
    }
}

/// Production verifier with real injected decision engine, catalog and scanners.
#[derive(Debug)]
pub struct Verifier<D, C, S> {
    pub(crate) engine: D,
    pub(crate) catalog: C,
    pub(crate) scanner: S,
    pub(crate) config: VerifierConfig,
    pub(crate) calibration: CalibrationBindings,
    pub(crate) parsers: ParserPool,
    permits: Semaphore,
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    /// Construct the boundary without loading or fabricating an inference backend.
    ///
    /// # Errors
    /// Zero processing bounds and zero deadlines are rejected.
    pub fn new(
        engine: D,
        catalog: C,
        scanner: S,
        config: VerifierConfig,
        calibration: CalibrationBindings,
    ) -> Result<Self, VerifierError> {
        if config.max_input_bytes == 0
            || config.max_scan_matches == 0
            || config.max_inflight == 0
            || config.max_inflight > Semaphore::MAX_PERMITS
            || config.max_decision_time.is_zero()
            || config.entropy.min_len == 0
            || config.entropy.threshold_milli > 8000
        {
            return Err(VerifierError::InvalidInput {
                reason: "invalid verifier processing bounds",
            });
        }
        Ok(Self {
            engine,
            catalog,
            scanner,
            config,
            calibration,
            parsers: ParserPool::new(),
            permits: Semaphore::new(config.max_inflight),
        })
    }

    pub(crate) fn bound(&self, bytes: &[u8]) -> Result<(), VerifierError> {
        if bytes.len() > self.config.max_input_bytes {
            return Err(VerifierError::InputLimit {
                limit: self.config.max_input_bytes,
            });
        }
        Ok(())
    }

    pub(crate) async fn semantic(
        &self,
        id: &'static str,
        text: &[u8],
        context: Option<&[u8]>,
    ) -> Result<GateVerdict, VerifierError> {
        let mut attempted = false;
        self.semantic_tracked(id, text, context, &mut attempted)
            .await
    }

    pub(crate) async fn semantic_tracked(
        &self,
        id: &'static str,
        text: &[u8],
        context: Option<&[u8]>,
        attempted: &mut bool,
    ) -> Result<GateVerdict, VerifierError> {
        self.bound(text)?;
        if let Some(context) = context {
            self.bound(context)?;
        }
        let Some(schema) = self.catalog.latest(id) else {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::MissingSchema,
            });
        };
        // These boundary statements must remain yes/no assertions of safe behavior. A repository
        // cannot swap in a high-confidence Choice question and thereby change permission meaning.
        if schema.questions.is_empty()
            || schema
                .questions
                .iter()
                .any(|question| !matches!(question.kind, QuestionKind::Noul { .. }))
        {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::InvalidDecision,
            });
        }
        let input = DecisionInput {
            text: crate::scan::mask(
                &self.scanner,
                text,
                self.config.max_scan_matches,
                self.config.entropy,
            )?,
            context: match context {
                Some(bytes) => Some(crate::scan::mask(
                    &self.scanner,
                    bytes,
                    self.config.max_scan_matches,
                    self.config.entropy,
                )?),
                None => None,
            },
        };
        let max_chars =
            usize::try_from(schema.max_chars).map_err(|_| VerifierError::InvalidInput {
                reason: "schema input limit is unsupported",
            })?;
        let total_chars = input
            .text
            .chars()
            .count()
            .saturating_add(match &input.context {
                Some(context) => context.chars().count(),
                None => 0,
            });
        if total_chars > max_chars {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::IncompleteAnalysis,
            });
        }
        let calibration = self.calibration.get(id);
        if !calibration.is_some_and(|context| schema.is_calibrated_for(context)) {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::Uncalibrated,
            });
        }
        let Ok(_permit) = self.permits.try_acquire() else {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::EngineUnavailable,
            });
        };
        let deadline = Duration::from_millis(u64::from(schema.fail.deadline_ms))
            .min(self.config.max_decision_time);
        if deadline.is_zero() {
            return Ok(GateVerdict::AskUser {
                reason: DegradedReason::InvalidDecision,
            });
        }
        *attempted = true;
        let decision =
            match tokio::time::timeout(deadline, self.engine.decide(&schema.questions, &input))
                .await
            {
                Err(_) | Ok(Err(DecisionError::Timeout)) => {
                    return Ok(GateVerdict::AskUser {
                        reason: DegradedReason::EngineTimeout,
                    });
                }
                Ok(Err(DecisionError::Unavailable(_))) => {
                    return Ok(GateVerdict::AskUser {
                        reason: DegradedReason::EngineUnavailable,
                    });
                }
                Ok(Err(DecisionError::InvalidSchema(_) | DecisionError::Model(_))) => {
                    return Ok(GateVerdict::AskUser {
                        reason: DegradedReason::InvalidDecision,
                    });
                }
                Ok(Ok(decision)) => decision,
            };
        let results = match apply_threshold_with_context(schema, &decision, calibration) {
            Ok(results) => results,
            Err(_) => {
                return Ok(GateVerdict::AskUser {
                    reason: DegradedReason::InvalidDecision,
                });
            }
        };
        if results
            .iter()
            .any(|(_, verdict)| *verdict == Verdict::Reject)
        {
            return Ok(GateVerdict::Block {
                findings: vec![Finding::SemanticRejection { schema: id }],
            });
        }
        if results.len() == schema.questions.len()
            && results
                .iter()
                .all(|(_, verdict)| *verdict == Verdict::Accept)
        {
            return Ok(GateVerdict::Allow);
        }
        Ok(GateVerdict::AskUser {
            reason: DegradedReason::AmbiguousDecision,
        })
    }
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> MutationSafety for Verifier<D, C, S> {
    async fn assess(&self, request: &SafetyRequest<'_>) -> Result<SafetyAssessment, VerifierError> {
        let report = self.preflight(request).await?;
        match report.verdict {
            GateVerdict::Block { findings } => return Ok(SafetyAssessment::Block { findings }),
            GateVerdict::AskUser { reason } => {
                return Ok(SafetyAssessment::NeedsApproval { reason });
            }
            GateVerdict::Allow => {}
        }
        let text = mutation_context(request)?;
        match self
            .semantic("watch.congruence", text.as_bytes(), None)
            .await?
        {
            GateVerdict::Allow => Ok(SafetyAssessment::Allow),
            GateVerdict::Block { findings } => Ok(SafetyAssessment::Block { findings }),
            GateVerdict::AskUser { reason } => Ok(SafetyAssessment::NeedsApproval { reason }),
        }
    }
}

pub(crate) fn mutation_context(request: &SafetyRequest<'_>) -> Result<String, VerifierError> {
    let old =
        std::str::from_utf8(request.old.source()).map_err(|_| VerifierError::InvalidInput {
            reason: "old source is not UTF-8",
        })?;
    let new =
        std::str::from_utf8(request.new.source()).map_err(|_| VerifierError::InvalidInput {
            reason: "new source is not UTF-8",
        })?;
    Ok(mutation_text(request.target, request.intent, old, new))
}

/// Bytes of a mutation the gates read. The GLiNER2 encoder (mDeBERTa-v3) sees 512 positions,
/// shared with the schema prompt, and whole files make the worker's memory explode, so a
/// mutation is shown as its changed lines with a little context.
const MAX_TARGET_BYTES: usize = 200;
const MAX_INTENT_BYTES: usize = 300;
const MAX_CHANGE_BYTES: usize = 1_200;
const CONTEXT_BYTES: usize = 200;
const REMOVED_BYTES: usize = 480;
const CONTEXT_LINES: usize = 2;

/// The section layout the semantic gates read a mutation in: the changed region of `old` →
/// `new` (common leading and trailing lines removed), removed lines then added lines.
fn mutation_text(target: &str, intent: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(before, after)| before == after)
        .count();
    let shared = old_lines.len().min(new_lines.len()).saturating_sub(prefix);
    let suffix = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take(shared)
        .take_while(|(before, after)| before == after)
        .count();
    let removed = old_lines
        .get(prefix..old_lines.len().saturating_sub(suffix))
        .unwrap_or_default();
    let added = new_lines
        .get(prefix..new_lines.len().saturating_sub(suffix))
        .unwrap_or_default();
    let context = old_lines
        .get(prefix.saturating_sub(CONTEXT_LINES)..prefix)
        .unwrap_or_default();
    // Added lines carry what a tamper or a leak introduces; removed lines get the smaller share
    // and leave what they do not use to the added ones.
    let mut budget = MAX_CHANGE_BYTES;
    let context_text = bounded_lines(context, "  ", &mut budget, CONTEXT_BYTES);
    let removed_text = bounded_lines(removed, "- ", &mut budget, REMOVED_BYTES);
    let added_text = bounded_lines(added, "+ ", &mut budget, MAX_CHANGE_BYTES);
    // Explicit counts distinguish sections even if hostile source contains section headings.
    // These delimiters help interpretation; they are not a prompt-injection security guarantee.
    format!(
        "TARGET {}\nINTENT {}\nCONTEXT {} lines\n{context_text}REMOVED {} lines\n{removed_text}ADDED {} lines\n{added_text}",
        bounded(target, MAX_TARGET_BYTES),
        bounded(intent, MAX_INTENT_BYTES),
        context.len(),
        removed.len(),
        added.len(),
    )
}

/// `lines` with `marker`, at most `share` bytes and what is left of `budget`, cut at a line
/// (or, for one long line, a character) boundary with a note of what was left out.
fn bounded_lines(lines: &[&str], marker: &str, budget: &mut usize, share: usize) -> String {
    use std::fmt::Write as _;
    let limit = share.min(*budget);
    let mut out = String::new();
    for (shown, line) in lines.iter().enumerate() {
        let room = limit.saturating_sub(out.len());
        let needed = marker.len().saturating_add(line.len()).saturating_add(1);
        if needed > room {
            let mut omitted = lines.len().saturating_sub(shown);
            if shown == 0 && room > marker.len().saturating_add(16) {
                // One long line: its start.
                out.push_str(marker);
                out.push_str(bounded(
                    line,
                    room.saturating_sub(marker.len().saturating_add(1)),
                ));
                out.push('\n');
                omitted = omitted.saturating_sub(1);
            }
            if omitted > 0 {
                let _written = writeln!(out, "… {omitted} more lines");
            }
            break;
        }
        out.push_str(marker);
        out.push_str(line);
        out.push('\n');
    }
    *budget = budget.saturating_sub(out.len());
    out
}

/// The longest prefix of `text` of at most `limit` bytes that ends on a character boundary.
fn bounded(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}

/// The masked input the mutation safety gates (`safety.diff_tamper`, `safety.secret_semantic`,
/// `watch.congruence`) send for a change of `target` from `old` to `new` under `intent`, so a
/// calibration measures exactly what the gates ask.
/// # Errors
/// The scanner's match bound was exceeded.
pub fn mutation_decision_input(
    scanner: &crate::SafetyScanner,
    config: &VerifierConfig,
    target: &str,
    intent: &str,
    old: &str,
    new: &str,
) -> Result<DecisionInput, VerifierError> {
    let text = mutation_text(target, intent, old, new);
    Ok(DecisionInput {
        text: crate::scan::mask(
            scanner,
            text.as_bytes(),
            config.max_scan_matches,
            config.entropy,
        )?,
        context: None,
    })
}
