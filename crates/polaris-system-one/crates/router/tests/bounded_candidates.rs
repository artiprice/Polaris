use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer},
};
use polaris_system_one_router::*;
use polaris_system_one_schemas::SchemaSet;
use std::{
    error::Error,
    path::PathBuf,
    sync::{Arc, Mutex},
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone, Default)]
struct Engine {
    calls: Arc<Mutex<usize>>,
    unavailable: bool,
}

impl DecisionEngine for Engine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| DecisionError::Model("fixture lock".into()))?;
        *calls = calls.saturating_add(1);
        drop(calls);
        if self.unavailable {
            return Err(DecisionError::Unavailable("synthetic outage".into()));
        }
        let mut answers = Vec::new();
        for question in questions {
            answers.push(QuestionAnswer {
                question: question.id.clone(),
                answer: Answer::Noul {
                    yes: Probability::new(0.95)
                        .map_err(|_| DecisionError::Model("probability".into()))?,
                },
            });
        }
        Ok(Decision {
            answers,
            latency_us: 1,
        })
    }
}

fn router(
    engine: Engine,
    max_candidates: usize,
) -> Result<Router<Engine, SchemaSet>, Box<dyn Error>> {
    let config = RouterConfig {
        max_candidates,
        ..RouterConfig::default()
    };
    Ok(Router::new(engine, SchemaSet::load_builtin()?, config)?)
}

#[tokio::test]
async fn prune_tools_bounds_engine_calls_output_and_rejects_duplicate_ids() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, 2)?;
    let tools = vec![
        ToolCandidate {
            id: ToolId::new("tool-c")?,
            name: "search".into(),
            summary: "read repository".into(),
        },
        ToolCandidate {
            id: ToolId::new("tool-b")?,
            name: "read".into(),
            summary: "read files".into(),
        },
        ToolCandidate {
            id: ToolId::new("tool-a")?,
            name: "read".into(),
            summary: "search files".into(),
        },
    ];
    let ranked = router
        .prune_tools("read files", &tools, TopK::new(1)?)
        .await?;
    assert_eq!(ranked.len(), 1);
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 2);
    let first_tool = tools.first().cloned().ok_or("missing first tool fixture")?;
    let duplicate = vec![
        first_tool.clone(),
        ToolCandidate {
            id: first_tool.id,
            name: "other".into(),
            summary: "other".into(),
        },
    ];
    assert!(
        router
            .prune_tools("read", &duplicate, TopK::new(1)?)
            .await
            .is_err()
    );
    let oversized_name = vec![ToolCandidate {
        id: ToolId::new("large")?,
        name: "n".repeat(513),
        summary: "read".into(),
    }];
    assert!(
        router
            .prune_tools("read", &oversized_name, TopK::new(1)?)
            .await
            .is_err()
    );
    assert!(
        router
            .prune_tools(&"q".repeat(16_385), &tools, TopK::new(1)?)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn rank_files_bounds_engine_calls_orders_fallback_and_rejects_duplicates() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, 2)?;
    let files = vec![
        FileCandidate {
            path: PathBuf::from("z.rs"),
            signature_text: "read files".into(),
            lexical_score: 0.5,
        },
        FileCandidate {
            path: PathBuf::from("a.rs"),
            signature_text: "read files".into(),
            lexical_score: 0.5,
        },
        FileCandidate {
            path: PathBuf::from("m.rs"),
            signature_text: "read files".into(),
            lexical_score: 0.4,
        },
    ];
    let ranked = router
        .rank_files("read files", &files, TopK::new(2)?)
        .await?;
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 2);
    assert_eq!(
        ranked
            .iter()
            .map(|item| item.item.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from("a.rs"), PathBuf::from("z.rs")]
    );
    let first_file = files.first().cloned().ok_or("missing first file fixture")?;
    let duplicate = vec![first_file.clone(), first_file];
    assert!(
        router
            .rank_files("read", &duplicate, TopK::new(1)?)
            .await
            .is_err()
    );
    assert!(
        router
            .rank_files(&"q".repeat(16_385), &files, TopK::new(1)?)
            .await
            .is_err()
    );
    Ok(())
}

#[test]
fn router_config_rejects_limits_above_hard_ceilings() -> TestResult {
    let schemas = SchemaSet::load_builtin()?;
    let config = RouterConfig {
        max_candidates: 257,
        ..RouterConfig::default()
    };
    assert!(Router::new(Engine::default(), schemas.clone(), config).is_err());
    let config = RouterConfig {
        max_candidate_chars: 4_097,
        ..RouterConfig::default()
    };
    assert!(Router::new(Engine::default(), schemas, config).is_err());
    Ok(())
}

#[tokio::test]
async fn sandbox_rules_keep_deterministic_isolation_constraints() -> TestResult {
    let engine = Engine {
        unavailable: true,
        ..Engine::default()
    };
    let config = RouterConfig {
        allowed_sandbox_tiers: [SandboxTier::E0, SandboxTier::E1, SandboxTier::E2]
            .into_iter()
            .collect(),
        ..RouterConfig::default()
    };
    let router = Router::new(engine, SchemaSet::load_builtin()?, config)?;
    let untrusted_without_kvm = SandboxFacts {
        untrusted_code: true,
        nested_kvm_ok: false,
        ..SandboxFacts::default()
    };
    assert!(
        router
            .route_sandbox(&untrusted_without_kvm, SandboxDirective::Auto)
            .await
            .is_err()
    );
    let safe_facts = SandboxFacts::default();
    assert_eq!(
        router
            .route_sandbox(&safe_facts, SandboxDirective::Require(SandboxTier::E1))
            .await?,
        SandboxTier::E1
    );
    assert_eq!(
        router
            .route_sandbox(&safe_facts, SandboxDirective::Auto)
            .await?,
        SandboxTier::E0
    );
    let untrusted_with_kvm = SandboxFacts {
        untrusted_code: true,
        nested_kvm_ok: true,
        ..SandboxFacts::default()
    };
    assert_eq!(
        router
            .route_sandbox(&untrusted_with_kvm, SandboxDirective::Auto)
            .await?,
        SandboxTier::E2
    );
    Ok(())
}
