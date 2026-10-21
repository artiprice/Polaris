//! Bounded, masked inference and calibrated safety acceptance.
use std::{collections::BTreeMap, time::Duration};
use polaris_code_intel::ParserPool;
use polaris_sdk::{registry::{DecisionEngine, DecisionError}, system_one::{DecisionInput, QuestionKind}};
use polaris_security_automata::EntropyCfg;
use polaris_system_one_schemas::{CalibrationContext, SchemaCatalog, Verdict, apply_threshold_with_context};
use tokio::sync::Semaphore;
use crate::{DegradedReason, Finding, GateVerdict, MutationSafety, SafetyAssessment, SafetyRequest, Scanner, VerifierError};

/// Expected calibration provenance injected by the composition root.
#[derive(Debug, Clone, Default)]
pub struct CalibrationBindings {
    contexts: BTreeMap<String, CalibrationContext>,
}

impl CalibrationBindings {
    /// Bind a schema id to trusted fixture/checkpoint identities, not model-provided claims.
    pub fn insert(&mut self, id: String, context: CalibrationContext) -> Option<CalibrationContext> {
        self.contexts.insert(id, context)
    }
    /// Look up the active expected identities for a schema.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CalibrationContext> { self.contexts.get(id) }
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
    pub fn new(engine: D, catalog: C, scanner: S, config: VerifierConfig, calibration: CalibrationBindings)
        -> Result<Self, VerifierError>
    {
        if config.max_input_bytes == 0 || config.max_scan_matches == 0 || config.max_inflight == 0
            || config.max_inflight > Semaphore::MAX_PERMITS || config.max_decision_time.is_zero()
            || config.entropy.min_len == 0 || config.entropy.threshold_milli > 8000
        {
            return Err(VerifierError::InvalidInput { reason: "invalid verifier processing bounds" });
        }
        Ok(Self { engine, catalog, scanner, config, calibration, parsers: ParserPool::new(),
            permits: Semaphore::new(config.max_inflight) })
    }

    pub(crate) fn bound(&self, bytes: &[u8]) -> Result<(), VerifierError> {
        if bytes.len() > self.config.max_input_bytes {
            return Err(VerifierError::InputLimit { limit: self.config.max_input_bytes });
        }
        Ok(())
    }

    pub(crate) async fn semantic(&self, id: &'static str, text: &[u8], context: Option<&[u8]>)
        -> Result<GateVerdict, VerifierError>
    {
        let mut attempted = false;
        self.semantic_tracked(id, text, context, &mut attempted).await
    }

    pub(crate) async fn semantic_tracked(&self, id: &'static str, text: &[u8], context: Option<&[u8]>, attempted: &mut bool)
        -> Result<GateVerdict, VerifierError>
    {
        self.bound(text)?;
        if let Some(context) = context { self.bound(context)?; }
        let Some(schema) = self.catalog.latest(id) else {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::MissingSchema });
        };
        // These boundary statements must remain yes/no assertions of safe behavior. A repository
        // cannot swap in a high-confidence Choice question and thereby change permission meaning.
        if schema.questions.is_empty() || schema.questions.iter().any(|question| {
            !matches!(question.kind, QuestionKind::Noul { .. })
        }) {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::InvalidDecision });
        }
        let input = DecisionInput {
            text: crate::scan::mask(&self.scanner, text, self.config.max_scan_matches, self.config.entropy)?,
            context: match context {
                Some(bytes) => Some(crate::scan::mask(&self.scanner, bytes, self.config.max_scan_matches, self.config.entropy)?),
                None => None,
            },
        };
        let max_chars = usize::try_from(schema.max_chars).map_err(|_| VerifierError::InvalidInput { reason: "schema input limit is unsupported" })?;
        let total_chars = input.text.chars().count().saturating_add(match &input.context {
            Some(context) => context.chars().count(), None => 0,
        });
        if total_chars > max_chars {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::IncompleteAnalysis });
        }
        let calibration = self.calibration.get(id);
        if !calibration.is_some_and(|context| schema.is_calibrated_for(context)) {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::Uncalibrated });
        }
        let Ok(_permit) = self.permits.try_acquire() else {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::EngineUnavailable });
        };
        let deadline = Duration::from_millis(u64::from(schema.fail.deadline_ms)).min(self.config.max_decision_time);
        if deadline.is_zero() {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::InvalidDecision });
        }
        *attempted = true;
        let decision = match tokio::time::timeout(deadline, self.engine.decide(&schema.questions, &input)).await {
            Err(_) | Ok(Err(DecisionError::Timeout)) => return Ok(GateVerdict::AskUser { reason: DegradedReason::EngineTimeout }),
            Ok(Err(DecisionError::Unavailable(_))) => return Ok(GateVerdict::AskUser { reason: DegradedReason::EngineUnavailable }),
            Ok(Err(DecisionError::InvalidSchema(_) | DecisionError::Model(_))) => return Ok(GateVerdict::AskUser { reason: DegradedReason::InvalidDecision }),
            Ok(Ok(decision)) => decision,
        };
        let results = match apply_threshold_with_context(schema, &decision, calibration) {
            Ok(results) => results,
            Err(_) => return Ok(GateVerdict::AskUser { reason: DegradedReason::InvalidDecision }),
        };
        if results.iter().any(|(_, verdict)| *verdict == Verdict::Reject) {
            return Ok(GateVerdict::Block { findings: vec![Finding::SemanticRejection { schema: id }] });
        }
        if results.len() == schema.questions.len() && results.iter().all(|(_, verdict)| *verdict == Verdict::Accept) {
            return Ok(GateVerdict::Allow);
        }
        Ok(GateVerdict::AskUser { reason: DegradedReason::AmbiguousDecision })
    }
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> MutationSafety for Verifier<D, C, S> {
    async fn assess(&self, request: &SafetyRequest<'_>) -> Result<SafetyAssessment, VerifierError> {
        let report = self.preflight(request).await?;
        match report.verdict {
            GateVerdict::Block { findings } => return Ok(SafetyAssessment::Block { findings }),
            GateVerdict::AskUser { reason } => return Ok(SafetyAssessment::NeedsApproval { reason }),
            GateVerdict::Allow => {},
        }
        let text = mutation_context(request)?;
        match self.semantic("watch.congruence", text.as_bytes(), None).await? {
            GateVerdict::Allow => Ok(SafetyAssessment::Allow),
            GateVerdict::Block { findings } => Ok(SafetyAssessment::Block { findings }),
            GateVerdict::AskUser { reason } => Ok(SafetyAssessment::NeedsApproval { reason }),
        }
    }
}

pub(crate) fn mutation_context(request: &SafetyRequest<'_>) -> Result<String, VerifierError> {
    let old = std::str::from_utf8(request.old.source()).map_err(|_| VerifierError::InvalidInput { reason: "old source is not UTF-8" })?;
    let new = std::str::from_utf8(request.new.source()).map_err(|_| VerifierError::InvalidInput { reason: "new source is not UTF-8" })?;
    // Explicit lengths distinguish sections even if hostile source contains section headings.
    // These delimiters help interpretation; they are not a prompt-injection security guarantee.
    Ok(format!("TARGET {} bytes\n{}\nINTENT {} bytes\n{}\nBEFORE {} bytes\n{}\nAFTER {} bytes\n{}",
        request.target.len(), request.target, request.intent.len(), request.intent,
        old.len(), old, new.len(), new))
}
