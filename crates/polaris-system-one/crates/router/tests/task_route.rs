//! Synthetic decisions exercise routing policy; they do not calibrate a real model.
use polaris_sdk::{
    effort::Effort,
    registry::{DecisionEngine, DecisionError},
    system_one::{
        Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
        QuestionKind,
    },
};
use polaris_system_one_router::*;
use polaris_system_one_schemas::SchemaSet;
use std::{
    error::Error,
    sync::{Arc, Mutex},
};
type TestResult = Result<(), Box<dyn Error>>;
#[derive(Clone, Default)]
struct Engine {
    calls: Arc<Mutex<Vec<Vec<String>>>>,
    unavailable: bool,
}
impl DecisionEngine for Engine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.calls
            .lock()
            .map_err(|_| DecisionError::Model("fixture lock".into()))?
            .push(questions.iter().map(|q| q.id.to_string()).collect());
        if self.unavailable {
            return Err(DecisionError::Unavailable("synthetic outage".into()));
        }
        let mut answers = Vec::new();
        for question in questions {
            let answer = match &question.kind {
                QuestionKind::Choice { options } => {
                    let selected = Label::new("s2")
                        .map_err(|_| DecisionError::Model("fixture label".into()))?;
                    let mut probs = options
                        .iter()
                        .map(|label| {
                            Ok(LabelScore {
                                label: label.clone(),
                                probability: Probability::new(if *label == selected {
                                    1.0
                                } else {
                                    0.0
                                })
                                .map_err(|_| DecisionError::Model("fixture probability".into()))?,
                            })
                        })
                        .collect::<Result<Vec<_>, DecisionError>>()?;
                    probs.sort_by(|left, right| {
                        right
                            .probability
                            .cmp(&left.probability)
                            .then_with(|| left.label.cmp(&right.label))
                    });
                    Answer::Choice { selected, probs }
                }
                QuestionKind::Score { .. } => Answer::Score {
                    value: 3,
                    confidence: Probability::new(1.0)
                        .map_err(|_| DecisionError::Model("fixture probability".into()))?,
                },
                QuestionKind::Noul { .. } => Answer::Noul {
                    yes: Probability::new(if question.id.to_string() == "parallel" {
                        1.0
                    } else {
                        0.0
                    })
                    .map_err(|_| DecisionError::Model("fixture probability".into()))?,
                },
            };
            answers.push(QuestionAnswer {
                question: question.id.clone(),
                answer,
            });
        }
        Ok(Decision {
            answers,
            latency_us: 1,
        })
    }
}
fn input() -> RouteInput {
    RouteInput {
        text: "masked task".into(),
        root_goal: None,
        repo_facts: RepoFacts::default(),
        directives: DirectiveSnapshot::default(),
    }
}
fn router(
    engine: Engine,
    config: RouterConfig,
) -> Result<Router<Engine, SchemaSet>, Box<dyn Error>> {
    Ok(Router::new(engine, SchemaSet::load_builtin()?, config)?)
}
#[tokio::test]
async fn route_timeout_defaults_to_medium_within_allowlist() -> TestResult {
    let router = router(
        Engine {
            unavailable: true,
            ..Engine::default()
        },
        RouterConfig::default(),
    )?;
    assert_eq!(router.route(&input()).await?.effort, Effort::Medium);
    let mut constrained = input();
    constrained.directives.allowed_efforts = Some(vec![Effort::Low]);
    assert_eq!(router.route(&constrained).await?.effort, Effort::Low);
    Ok(())
}
#[tokio::test]
async fn route_combines_intent_effort_parallel_and_vision_once() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let route = router.route(&input()).await?;
    assert_eq!(route.path, RoutePath::S2);
    assert_eq!(route.effort, Effort::High);
    assert!(route.spawn.parallel_branches);
    assert!(!route.needs_vision);
    assert_eq!(route.source, RouteSource::S1);
    assert_eq!(
        *calls.lock().map_err(|_| "fixture lock")?,
        vec![vec![
            "intent".to_owned(),
            "effort".to_owned(),
            "parallel".to_owned(),
            "vision".to_owned()
        ]]
    );
    Ok(())
}
#[tokio::test]
async fn route_preserves_hard_facts_and_explicit_direct_tool() -> TestResult {
    let tool = ToolId::new("read_file")?;
    let mut config = RouterConfig::default();
    config.direct_tool_allowlist.insert(tool.clone());
    let router = router(Engine::default(), config)?;
    let mut task = input();
    task.repo_facts.needs_vision = true;
    task.directives.direct_tool = Some(tool.clone());
    task.directives.allowed_efforts = Some(vec![Effort::Low]);
    let route = router.route(&task).await?;
    assert_eq!(route.path, RoutePath::DirectTool { tool });
    assert_eq!(route.effort, Effort::Low);
    assert!(route.needs_vision);
    Ok(())
}
#[tokio::test]
async fn route_rejects_oversize_input_before_inference() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let mut task = input();
    task.root_goal = Some("x".repeat(262_145));
    assert!(router.route(&task).await.is_err());
    assert!(calls.lock().map_err(|_| "fixture lock")?.is_empty());
    Ok(())
}
