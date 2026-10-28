//! Synthetic calibrated catalog exercises the real adapter egress, not model calibration.
use super::{ConfiguredFrameWatchdog, FrameProposalObserver};
use polaris_sdk::{
    ids::{AgentRole, FrameId, SessionId},
    registry::{DecisionEngine, DecisionError},
    system_one::{Decision, DecisionInput, Question},
};
use polaris_security::{
    automata::EntropyCfg,
    pii::{PiiLimits, PiiVault},
};
use polaris_system_one::{
    schemas::{CalibrationContext, SchemaSet},
    watchdog::{
        FatigueConfig, FrameSnapshot, HealthVerdict, LoopConfig, ProposedStep, RatioThreshold,
        RoleTable, Watchdog, WatchdogConfig,
    },
};
use std::{
    error::Error,
    sync::{Arc, Mutex},
};
type TestResult = Result<(), Box<dyn Error>>;
struct CapturingEngine(Arc<Mutex<Vec<DecisionInput>>>);
impl DecisionEngine for CapturingEngine {
    async fn decide(
        &self,
        _: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.0
            .lock()
            .map_err(|_| DecisionError::Unavailable("fixture state".into()))?
            .push(input.clone());
        Err(DecisionError::Unavailable("synthetic outage".into()))
    }
}
fn adapter(
    calls: Arc<Mutex<Vec<DecisionInput>>>,
    max_input: usize,
) -> Result<ConfiguredFrameWatchdog<CapturingEngine>, Box<dyn Error>> {
    let digest = "a".repeat(64);
    let source = format!(
        r#"
[[schemas]]
id="watch.congruence"
version=1
kind="congruence"
max_chars=4096
questions=[{{id="congruent",kind={{type="noul",statement="The step advances the goal."}}}}]
thresholds={{accept=0.9,reject=0.1,abstain_band=[0.2,0.8]}}
fail={{consumer="watchdog",on_timeout="open",deadline_ms=100,integrity="normal"}}
calibration={{status="calibrated",fixture_set="SYNTHETIC privacy contract",fixture_blake3="{digest}",checkpoint_blake3="{digest}",n=1,precision_target=1.0,observed_precision=1.0,observed_recall=1.0,ece=0.0,date="2026-10-27T00:00:00Z"}}
"#
    );
    let watchdog = Watchdog::new(
        CapturingEngine(calls),
        SchemaSet::from_toml(&source)?,
        RoleTable::default(),
        WatchdogConfig {
            max_step_bytes: 4096,
            loops: LoopConfig {
                window_size: 8,
                shingle_size: 2,
                near_duplicate: RatioThreshold {
                    numerator: 1,
                    denominator: 1,
                },
            },
            fatigue: FatigueConfig {
                pressure: RatioThreshold {
                    numerator: 1,
                    denominator: 1,
                },
                repetition: RatioThreshold {
                    numerator: 1,
                    denominator: 1,
                },
                require_both_signals: true,
            },
        },
        CalibrationContext {
            fixture_blake3: digest.clone(),
            checkpoint_blake3: digest,
        },
    )?;
    let vault = PiiVault::new(
        SessionId::from_u128(77),
        FrameId::from_u128(77),
        EntropyCfg::default(),
        PiiLimits {
            max_input_bytes: max_input,
            ..PiiLimits::default()
        },
    )?;
    Ok(ConfiguredFrameWatchdog {
        watchdog: Arc::new(watchdog),
        vault: Arc::new(vault),
    })
}
fn snapshot(goal: String) -> FrameSnapshot {
    FrameSnapshot {
        frame: FrameId::from_u128(77),
        current_step: 1,
        role: AgentRole::Orchestrator,
        root_goal: goal,
        recent: Vec::new(),
        negative_constraints: Vec::new(),
        context_tokens: 1,
        context_budget: 4096,
    }
}

struct NameDetector;
impl polaris_sdk::system_one::PiiSpanDetector for NameDetector {
    async fn detect_pii_spans(
        &self,
        text: &str,
    ) -> Result<Vec<polaris_sdk::system_one::PiiEntitySpan>, DecisionError> {
        text.match_indices("Alice")
            .map(|(start, name)| {
                let end = start
                    .checked_add(name.len())
                    .ok_or_else(|| DecisionError::Model("fixture bounds".into()))?;
                polaris_sdk::system_one::PiiEntitySpan::new(
                    u32::try_from(start)
                        .map_err(|_| DecisionError::Model("fixture bounds".into()))?,
                    u32::try_from(end)
                        .map_err(|_| DecisionError::Model("fixture bounds".into()))?,
                    0.99,
                )
                .map_err(|_| DecisionError::Model("fixture span".into()))
            })
            .collect()
    }
}

#[tokio::test]
async fn watchdog_semantic_names_are_masked_before_decision() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observer = adapter(Arc::clone(&calls), 4096)?;
    let privacy = crate::prompt_privacy::SemanticPrivacy::new(
        Arc::new(NameDetector),
        crate::SemanticPrivacyLimits::default(),
    )?;
    let frame = snapshot("Help Alice debug the parser".into());
    let step = ProposedStep {
        thought: "Inspect parser for Alice".into(),
        tool_calls: Vec::new(),
    };
    let _observation = observer
        .inspect(&frame, &step, Some(&privacy))
        .await
        .ok_or("missing advisory")?;
    let calls = calls.lock().map_err(|_| "fixture state")?;
    assert_eq!(
        calls.len(),
        1,
        "the synthetic calibrated fixture must reach S1"
    );
    let input = calls.first().ok_or("missing model request")?;
    assert!(
        !input.text.contains("Alice"),
        "semantic name must be masked in proposal"
    );
    assert!(
        !input
            .context
            .as_deref()
            .ok_or("missing goal")?
            .contains("Alice"),
        "semantic name must be masked in goal"
    );
    Ok(())
}
#[tokio::test]
async fn watchdog_masks_goal_and_proposal_before_decision() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observer = adapter(Arc::clone(&calls), 4096)?;
    let frame = snapshot("Help goal.owner@example.test debug parser".into());
    let step = ProposedStep {
        thought: "Inspect parser for step.owner@example.test".into(),
        tool_calls: Vec::new(),
    };
    let observation = observer
        .inspect(&frame, &step, None)
        .await
        .ok_or("missing observation")?;
    assert!(matches!(
        observation.verdict,
        HealthVerdict::Unchecked { .. }
    ));
    let calls = calls.lock().map_err(|_| "fixture state")?;
    assert_eq!(
        calls.len(),
        1,
        "the fixture must actually reach the decision engine"
    );
    let input = calls.first().ok_or("missing actual model request")?;
    assert!(!input.text.contains("step.owner@example.test"));
    assert!(
        !input
            .context
            .as_deref()
            .ok_or("missing goal context")?
            .contains("goal.owner@example.test"),
        "raw root goal must not escape the kernel privacy boundary"
    );
    assert!(
        input
            .context
            .as_deref()
            .is_some_and(|text| text.contains("<PII1:"))
    );
    Ok(())
}
#[tokio::test]
async fn watchdog_goal_mask_failure_skips_decision() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observer = adapter(Arc::clone(&calls), 32)?;
    let frame = snapshot("private goal ".repeat(32));
    let step = ProposedStep {
        thought: "Inspect parser".into(),
        tool_calls: Vec::new(),
    };
    let observation = observer
        .inspect(&frame, &step, None)
        .await
        .ok_or("missing unchecked advisory")?;
    assert!(matches!(
        observation.verdict,
        HealthVerdict::Unchecked { .. }
    ));
    assert!(
        calls.lock().map_err(|_| "fixture state")?.is_empty(),
        "failed goal masking must not contact S1"
    );
    Ok(())
}
