use polaris_sdk::{
    ids::{AgentRole, FrameId},
    registry::{DecisionEngine, DecisionError},
    system_one::{Decision, DecisionInput, Question},
};
use polaris_system_one_schemas::{CalibrationContext, SchemaSet};
use polaris_system_one_watchdog::{
    FatigueConfig, FrameSnapshot, HealthVerdict, LoopConfig, LoopWindow, NegativeConstraint,
    ProposedStep, RatioThreshold, RoleTable, ShingleSimilarity, StepIndex, StepOutcome, Watchdog,
    WatchdogConfig, build_step_record,
};
use std::future::Future;

fn config(threshold: RatioThreshold) -> WatchdogConfig {
    WatchdogConfig {
        max_step_bytes: 4096,
        loops: LoopConfig {
            window_size: 8,
            shingle_size: 1,
            near_duplicate: threshold,
        },
        fatigue: FatigueConfig {
            pressure: RatioThreshold {
                numerator: 4,
                denominator: 5,
            },
            repetition: RatioThreshold {
                numerator: 1,
                denominator: 2,
            },
            require_both_signals: true,
        },
    }
}

fn record(
    index: u32,
    thought: &str,
    outcome: StepOutcome,
) -> Result<polaris_system_one_watchdog::StepRecord, polaris_system_one_watchdog::StepBuildError> {
    build_step_record(
        StepIndex::new(index),
        &ProposedStep {
            thought: thought.to_owned(),
            tool_calls: Vec::new(),
        },
        1,
        4096,
        outcome,
    )
}

#[test]
fn matching_rejected_hypothesis_survives_newer_unproductive_match() -> Result<(), String> {
    let provisional = record(1, "patch replace parser", StepOutcome::Unknown)
        .map_err(|error| error.to_string())?;
    let hypothesis = provisional.fingerprint;
    let rejected = record(
        1,
        "patch replace parser",
        StepOutcome::RejectedPatch { hypothesis },
    )
    .map_err(|error| error.to_string())?;
    let newer_failure =
        record(2, "patch fix parser", StepOutcome::Failed).map_err(|error| error.to_string())?;
    let next = record(3, "patch update parser", StepOutcome::Unknown)
        .map_err(|error| error.to_string())?;
    let mut window = LoopWindow::new(8).map_err(|error| error.to_string())?;
    window.push(rejected);
    window.push(newer_failure);

    let found = window
        .detect(
            &next,
            &config(RatioThreshold {
                numerator: 1,
                denominator: 2,
            }),
        )
        .ok_or_else(|| "expected near-duplicate loop match".to_owned())?;

    if found.disproven != Some(hypothesis) {
        return Err(format!(
            "expected rejected hypothesis {hypothesis:?}, got {:?}",
            found.disproven
        ));
    }
    if found.repeats != StepIndex::new(1) {
        return Err(format!(
            "expected rejected step index 1, got {:?}",
            found.repeats
        ));
    }
    Ok(())
}

#[test]
fn distinct_nonoverlapping_steps_do_not_match_at_positive_threshold() -> Result<(), String> {
    let first = record(1, "alpha beta", StepOutcome::Failed).map_err(|error| error.to_string())?;
    let next = record(2, "gamma delta", StepOutcome::Unknown).map_err(|error| error.to_string())?;
    let mut window = LoopWindow::new(4).map_err(|error| error.to_string())?;
    window.push(first);
    let found = window.detect(
        &next,
        &config(RatioThreshold {
            numerator: 1,
            denominator: 2,
        }),
    );
    if found.is_some() {
        return Err(format!("disjoint steps unexpectedly matched: {found:?}"));
    }
    if window.highest_similarity(&next) != ShingleSimilarity::zero() {
        return Err("disjoint steps should have zero shingle similarity".to_owned());
    }
    Ok(())
}

struct UnavailableEngine;

impl DecisionEngine for UnavailableEngine {
    fn decide(
        &self,
        _schema: &[Question],
        _input: &DecisionInput,
    ) -> impl Future<Output = Result<Decision, DecisionError>> + Send {
        async { Err(DecisionError::Unavailable("fixture unavailable".to_owned())) }
    }
}

#[test]
fn watchdog_rejects_zero_near_duplicate_threshold() -> Result<(), String> {
    let schemas = SchemaSet::load_builtin().map_err(|error| error.to_string())?;
    let result = Watchdog::new(
        UnavailableEngine,
        schemas,
        RoleTable::default(),
        config(RatioThreshold {
            numerator: 0,
            denominator: 1,
        }),
        CalibrationContext {
            fixture_blake3: "fixture".to_owned(),
            checkpoint_blake3: "checkpoint".to_owned(),
        },
    );
    if result.is_ok() {
        return Err("zero near-duplicate threshold should be rejected".to_owned());
    }
    Ok(())
}

#[tokio::test]
async fn persisted_negative_fingerprint_is_checked_after_recent_window_eviction()
-> Result<(), String> {
    let schemas = SchemaSet::load_builtin().map_err(|error| error.to_string())?;
    let watchdog = Watchdog::new(
        UnavailableEngine,
        schemas,
        RoleTable::default(),
        config(RatioThreshold {
            numerator: 1,
            denominator: 2,
        }),
        CalibrationContext {
            fixture_blake3: "fixture".to_owned(),
            checkpoint_blake3: "checkpoint".to_owned(),
        },
    )
    .map_err(|error| error.to_string())?;
    let rejected =
        record(1, "replace the parser", StepOutcome::Failed).map_err(|error| error.to_string())?;
    let negative = NegativeConstraint {
        hypothesis: rejected.fingerprint,
        rejected_at: StepIndex::new(1),
    };
    let snapshot = FrameSnapshot {
        frame: FrameId::from_u128(77),
        current_step: 2,
        role: AgentRole::Orchestrator,
        root_goal: "fix parser".to_owned(),
        recent: Vec::new(),
        negative_constraints: vec![negative],
        context_tokens: 0,
        context_budget: 0,
    };
    let verdict = watchdog
        .inspect(
            &snapshot,
            &ProposedStep {
                thought: "replace the parser".to_owned(),
                tool_calls: Vec::new(),
            },
        )
        .await;
    let matched = match &verdict {
        HealthVerdict::Loop {
            repeats, disproven, ..
        } => *repeats == StepIndex::new(1) && *disproven == Some(negative.hypothesis),
        _ => false,
    };
    if !matched {
        return Err(format!("persisted negative was not returned: {verdict:?}"));
    }
    Ok(())
}
