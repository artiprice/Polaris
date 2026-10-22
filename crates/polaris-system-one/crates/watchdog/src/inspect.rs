//! Deterministic watchdog checks followed by bounded, schema-backed S1 drift analysis.

use crate::{
    FrameSnapshot, HealthVerdict, LoopWindow, ProposedStep, RatioThreshold, RoleTable, StepIndex,
    StepOutcome, UncheckedReason, WatchdogConfig, build_step_record, loops::ratio_at_least,
};
use polaris_sdk::{
    registry::DecisionEngine,
    system_one::{Answer, DecisionInput, QuestionKind},
};
use polaris_system_one_schemas::{
    CalibrationContext, Consumer, SchemaCatalog, SchemaKind, Verdict,
};
use std::time::Duration;

const CONGRUENCE_SCHEMA: &str = "watch.congruence";
const CONGRUENCE_QUESTION: &str = "congruent";

/// Configuration or construction error for the watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WatchdogError {
    /// A required size, threshold, or timeout bound is invalid.
    #[error("watchdog configuration is invalid")]
    InvalidConfig,
}

/// Per-step advisory watchdog.
#[derive(Debug)]
pub struct Watchdog<D, C> {
    engine: D,
    catalog: C,
    role_table: RoleTable,
    config: WatchdogConfig,
    calibration: CalibrationContext,
}

impl<D, C> Watchdog<D, C>
where
    D: DecisionEngine,
    C: SchemaCatalog,
{
    /// Construct a watchdog with real injected S1, schema catalog, role table, and bounds.
    ///
    /// # Errors
    /// Returns [`WatchdogError::InvalidConfig`] when any limit or ratio is invalid.
    pub fn new(
        engine: D,
        catalog: C,
        role_table: RoleTable,
        config: WatchdogConfig,
        calibration: CalibrationContext,
    ) -> Result<Self, WatchdogError> {
        validate_config(config)?;
        Ok(Self {
            engine,
            catalog,
            role_table,
            config,
            calibration,
        })
    }

    /// Inspect a proposed step before it reaches S2.
    ///
    /// Role policy, loop detection, and context-pressure checks run deterministically first. S1
    /// is queried only when those checks are nominal. Missing schemas, mismatched calibration,
    /// engine errors, timeouts, and malformed decisions return [`HealthVerdict::Unchecked`],
    /// preserving the watchdog's fail-open advisory policy. Capability authorization remains the
    /// only source of tool authority.
    pub async fn inspect(&self, snapshot: &FrameSnapshot, step: &ProposedStep) -> HealthVerdict {
        if let Some(call) = step.tool_call.as_ref() {
            if !self.role_table.allows(snapshot.role, call) {
                return HealthVerdict::RoleViolation {
                    tool: call.tool.as_str().to_owned(),
                    role: snapshot.role,
                };
            }
        }

        let index = match next_index(&snapshot.recent) {
            Some(index) => index,
            None => {
                return HealthVerdict::Unchecked {
                    reason: UncheckedReason::StepIndexOverflow,
                };
            }
        };
        let next = match build_step_record(
            index,
            step,
            self.config.loops.shingle_size,
            self.config.max_step_bytes,
            StepOutcome::Unknown,
        ) {
            Ok(record) => record,
            Err(crate::StepBuildError::TooLarge) => {
                return HealthVerdict::Unchecked {
                    reason: UncheckedReason::StepTooLarge,
                };
            }
            Err(
                crate::StepBuildError::InvalidShingleSize
                | crate::StepBuildError::InvalidByteLimit
                | crate::StepBuildError::InvalidWindowSize
                | crate::StepBuildError::TooManyShingles,
            ) => {
                return HealthVerdict::Unchecked {
                    reason: UncheckedReason::InvalidDecision,
                };
            }
        };

        let mut window = match LoopWindow::new(self.config.loops.window_size) {
            Ok(window) => window,
            Err(_) => {
                return HealthVerdict::Unchecked {
                    reason: UncheckedReason::InvalidDecision,
                };
            }
        };
        let recent_window = snapshot
            .recent
            .iter()
            .rev()
            .take(self.config.loops.window_size)
            .collect::<Vec<_>>();
        for record in recent_window.iter().rev() {
            window.push((**record).clone());
        }
        if let Some(found) = window.detect(&next, &self.config) {
            return HealthVerdict::Loop {
                repeats: found.repeats,
                disproven: found.disproven,
                similarity: found.similarity,
            };
        }

        if snapshot.context_budget == 0 {
            return HealthVerdict::Unchecked {
                reason: UncheckedReason::InvalidContextBudget,
            };
        }
        let pressure = ratio_at_least(
            snapshot.context_tokens,
            snapshot.context_budget,
            self.config.fatigue.pressure,
        );
        let repetition = window.highest_similarity(&next);
        let repeated = ratio_at_least(
            repetition.intersection,
            repetition.union,
            self.config.fatigue.repetition,
        );
        let fatigue = if self.config.fatigue.require_both_signals {
            pressure && repeated
        } else {
            pressure
        };
        if fatigue {
            return HealthVerdict::ContextFatigue {
                context_tokens: snapshot.context_tokens,
                context_budget: snapshot.context_budget,
                repetition,
            };
        }

        if snapshot.root_goal.trim().is_empty() {
            return HealthVerdict::Unchecked {
                reason: UncheckedReason::EmptyGoal,
            };
        }
        self.goal_drift(snapshot, step).await
    }

    async fn goal_drift(&self, snapshot: &FrameSnapshot, step: &ProposedStep) -> HealthVerdict {
        let Some(schema) = self.catalog.latest(CONGRUENCE_SCHEMA) else {
            return unchecked(UncheckedReason::SchemaUnavailable);
        };
        if schema.kind != SchemaKind::Congruence
            || schema.fail.consumer != Consumer::Watchdog
            || schema.fail.deadline_ms == 0
            || !schema.is_calibrated_for(&self.calibration)
            || schema.questions.len() != 1
        {
            return unchecked(UncheckedReason::SchemaUnavailable);
        }
        let Some(question) = schema.questions.first() else {
            return unchecked(UncheckedReason::SchemaUnavailable);
        };
        if question.id.to_string() != CONGRUENCE_QUESTION
            || !matches!(&question.kind, QuestionKind::Noul { .. })
        {
            return unchecked(UncheckedReason::SchemaUnavailable);
        }
        let max_chars = match usize::try_from(schema.max_chars) {
            Ok(value) if value > " [truncated]".chars().count() => value,
            _ => return unchecked(UncheckedReason::SchemaUnavailable),
        };
        let input = DecisionInput {
            text: bounded_text(&decision_step_text(step), max_chars),
            context: Some(bounded_text(&snapshot.root_goal, max_chars)),
        };
        let deadline = Duration::from_millis(u64::from(schema.fail.deadline_ms));
        let decision =
            match tokio::time::timeout(deadline, self.engine.decide(&schema.questions, &input))
                .await
            {
                Ok(Ok(decision)) => decision,
                Ok(Err(_)) => return unchecked(UncheckedReason::EngineFailure),
                Err(_) => return unchecked(UncheckedReason::EngineTimeout),
            };
        let verdicts = match schema.apply(&decision) {
            Ok(verdicts) => verdicts,
            Err(_) => return unchecked(UncheckedReason::InvalidDecision),
        };
        let alignment = match decision
            .answers
            .iter()
            .find(|answer| answer.question == question.id)
            .map(|answer| &answer.answer)
        {
            Some(Answer::Noul { yes }) => *yes,
            _ => return unchecked(UncheckedReason::InvalidDecision),
        };
        let Some((_, verdict)) = verdicts
            .iter()
            .find(|(question_id, _)| question_id == &question.id)
        else {
            return unchecked(UncheckedReason::InvalidDecision);
        };
        match verdict {
            Verdict::Reject => HealthVerdict::GoalDrift {
                alignment,
                anchor: bounded_text(&snapshot.root_goal, 256),
            },
            Verdict::Accept | Verdict::Abstain => HealthVerdict::Nominal,
            Verdict::NeedsApproval => unchecked(UncheckedReason::InvalidDecision),
        }
    }
}

fn validate_config(config: WatchdogConfig) -> Result<(), WatchdogError> {
    let valid_ratio =
        |ratio: RatioThreshold| ratio.denominator > 0 && ratio.numerator <= ratio.denominator;
    if config.max_step_bytes == 0
        || config.max_step_bytes > 1_048_576
        || config.loops.window_size == 0
        || config.loops.window_size > 500
        || config.loops.shingle_size == 0
        || config.loops.shingle_size > 128
        || !valid_ratio(config.loops.near_duplicate)
        || !valid_ratio(config.fatigue.pressure)
        || !valid_ratio(config.fatigue.repetition)
    {
        return Err(WatchdogError::InvalidConfig);
    }
    Ok(())
}

fn next_index(recent: &[crate::StepRecord]) -> Option<StepIndex> {
    match recent.last() {
        Some(record) => record.index.get().checked_add(1).map(StepIndex::new),
        None => Some(StepIndex::new(1)),
    }
}

fn bounded_text(input: &str, limit: usize) -> String {
    let mut output = String::new();
    let marker = " [truncated]";
    let max_content = limit.saturating_sub(marker.chars().count());
    let mut count = 0_usize;
    let mut truncated = false;
    for ch in input.chars() {
        if count >= max_content {
            truncated = true;
            break;
        }
        output.push(ch);
        count = count.saturating_add(1);
    }
    if truncated {
        output.push_str(marker);
    }
    output
}

fn decision_step_text(step: &ProposedStep) -> String {
    let Some(call) = step.tool_call.as_ref() else {
        return step.thought.clone();
    };
    let path = match call.workspace_path.as_deref() {
        Some(path) => path,
        None => "",
    };
    format!(
        "{}\nTool: {}\nPath: {}\nWrites workspace: {}\nNetwork: {}\nExecutes: {}",
        step.thought,
        call.tool.as_str(),
        path,
        call.writes_workspace,
        call.network,
        call.executes,
    )
}

fn unchecked(reason: UncheckedReason) -> HealthVerdict {
    HealthVerdict::Unchecked { reason }
}
