use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{Answer, Decision, DecisionInput, Probability, Question, QuestionAnswer},
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
    calls: Arc<Mutex<usize>>,
    unavailable: bool,
    timeout: bool,
}

impl DecisionEngine for Engine {
    async fn decide(
        &self,
        questions: &[Question],
        _: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        {
            let mut calls = self
                .calls
                .lock()
                .map_err(|_| DecisionError::Model("fixture lock".into()))?;
            *calls = calls.saturating_add(1);
        }
        if self.timeout {
            return std::future::pending::<Result<Decision, DecisionError>>().await;
        }
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
    config: RouterConfig,
) -> Result<Router<Engine, SchemaSet>, Box<dyn Error>> {
    Ok(Router::new(engine, SchemaSet::load_builtin()?, config)?)
}

fn manifest(
    commit: Option<&str>,
    lockfile_hash: Option<&str>,
) -> Result<RepoManifestFacts, Box<dyn Error>> {
    Ok(RepoManifestFacts::new(
        "owner/repo",
        commit,
        lockfile_hash,
        "rust tokio linux",
    )?)
}

fn image(
    id: &str,
    repo: &str,
    commit: Option<&str>,
    lockfile_hash: Option<&str>,
    metadata: &str,
) -> Result<EnvImageCandidate, Box<dyn Error>> {
    Ok(EnvImageCandidate::new(
        ImageId::new(id)?,
        Some(repo),
        commit,
        lockfile_hash,
        metadata,
    )?)
}

#[tokio::test]
async fn exact_repo_commit_or_lockfile_match_skips_engine() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let commit = "a".repeat(40);
    let images = vec![image(
        "img-exact",
        "owner/repo",
        Some(&commit),
        None,
        "rust",
    )?];
    let result = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert_eq!(result.len(), 1);
    assert_eq!(
        result.first().map(|ranked| ranked.item.as_str()),
        Some("img-exact")
    );
    assert_eq!(
        result.first().map(|ranked| ranked.reason),
        Some(RankReason::ExactIdentity)
    );
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 0);
    let hash = "c".repeat(64);
    let images = vec![image("img-hash", "owner/repo", None, Some(&hash), "rust")?];
    let result = router
        .rank_env_images(&manifest(None, Some(&hash))?, &images)
        .await?;
    assert_eq!(
        result.first().map(|ranked| ranked.item.as_str()),
        Some("img-hash")
    );
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 0);
    Ok(())
}

#[tokio::test]
async fn foreign_repo_same_lockfile_hash_is_not_exact() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let hash = "d".repeat(64);
    let images = vec![image(
        "img-foreign",
        "foreign/repo",
        None,
        Some(&hash),
        "rust tokio linux",
    )?];
    let result = router
        .rank_env_images(&manifest(None, Some(&hash))?, &images)
        .await?;
    assert_eq!(
        result.first().map(|ranked| ranked.reason),
        Some(RankReason::Semantic)
    );
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 1);
    Ok(())
}

#[tokio::test]
async fn near_miss_is_scored_and_not_treated_as_exact() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let commit = "b".repeat(40);
    let images = vec![image(
        "img-near",
        "owner/repo",
        Some(&commit),
        None,
        "rust tokio linux",
    )?];
    let result = router
        .rank_env_images(&manifest(Some(&"a".repeat(40)), None)?, &images)
        .await?;
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 1);
    assert_eq!(
        result.first().map(|ranked| ranked.reason),
        Some(RankReason::Semantic)
    );
    let images = vec![image(
        "img-prefix",
        "owner/repo",
        Some(&format!("{}{}", "a".repeat(39), "b")),
        None,
        "rust tokio linux",
    )?];
    let commit = format!("{}{}", "a".repeat(39), "c");
    let result = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 2);
    assert_eq!(
        result.first().map(|ranked| ranked.reason),
        Some(RankReason::Semantic)
    );
    Ok(())
}

#[tokio::test]
async fn tied_candidates_have_stable_order_and_duplicate_ids_are_rejected() -> TestResult {
    let router = router(
        Engine {
            unavailable: true,
            ..Engine::default()
        },
        RouterConfig::default(),
    )?;
    let images = vec![
        image("img-b", "other/repo", None, None, "rust")?,
        image("img-a", "other/repo", None, None, "rust")?,
    ];
    let commit = "a".repeat(40);
    let first = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    let second = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert_eq!(first, second);
    assert_eq!(
        first.first().map(|ranked| ranked.item.as_str()),
        Some("img-a")
    );
    let duplicate = vec![
        image("same", "other/repo", None, None, "rust")?,
        image("same", "owner/repo", Some(&"a".repeat(40)), None, "rust")?,
    ];
    assert!(
        router
            .rank_env_images(&manifest(Some(&commit), None)?, &duplicate)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn candidate_count_and_text_sizes_obey_hard_bounds() -> TestResult {
    let config = RouterConfig {
        max_candidates: 257,
        ..RouterConfig::default()
    };
    assert!(Router::new(Engine::default(), SchemaSet::load_builtin()?, config).is_err());
    let router = router(Engine::default(), RouterConfig::default())?;
    let images = vec![image("img", "other/repo", None, None, "rust")?];
    let commit = "a".repeat(40);
    assert!(RepoManifestFacts::new("owner/repo", Some(&commit), None, "q".repeat(16_385)).is_err());
    assert!(RepoManifestFacts::new("owner/repo", Some("abc"), None, "rust").is_err());
    let too_many = (0..4_097)
        .map(|index| {
            EnvImageCandidate::new(
                ImageId::new(format!("img-{index}"))?,
                None,
                None,
                None,
                "rust",
            )
        })
        .collect::<Result<Vec<_>, RouterError>>()?;
    assert!(
        router
            .rank_env_images(&manifest(Some(&commit), None)?, &too_many)
            .await
            .is_err()
    );
    assert!(
        router
            .rank_env_images(&manifest(Some(&commit), None)?, &images)
            .await
            .is_ok()
    );
    Ok(())
}

#[tokio::test]
async fn engine_outage_uses_deterministic_near_miss_fallback_and_candidate_cap() -> TestResult {
    let engine = Engine {
        unavailable: true,
        ..Engine::default()
    };
    let calls = Arc::clone(&engine.calls);
    let config = RouterConfig {
        max_candidates: 2,
        ..RouterConfig::default()
    };
    let router = router(engine, config)?;
    let images = vec![
        image("img-c", "other/repo", None, None, "rust")?,
        image("img-b", "other/repo", None, None, "rust")?,
        image("img-a", "owner/repo", None, None, "rust tokio linux")?,
    ];
    let commit = "a".repeat(40);
    let result = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert!(result.len() <= 2);
    assert!(
        result
            .iter()
            .all(|ranked| ranked.reason == RankReason::LexicalFallback)
    );
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 1);
    Ok(())
}

#[tokio::test]
async fn engine_timeout_uses_deterministic_fallback() -> TestResult {
    let engine = Engine {
        timeout: true,
        ..Engine::default()
    };
    let calls = Arc::clone(&engine.calls);
    let router = router(engine, RouterConfig::default())?;
    let images = vec![image(
        "img-near",
        "other/repo",
        None,
        None,
        "rust tokio linux",
    )?];
    let commit = "a".repeat(40);
    let result = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert_eq!(
        result.first().map(|ranked| ranked.reason),
        Some(RankReason::LexicalFallback)
    );
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 1);
    Ok(())
}

#[tokio::test]
async fn semantic_scoring_respects_candidate_cap() -> TestResult {
    let engine = Engine::default();
    let calls = Arc::clone(&engine.calls);
    let config = RouterConfig {
        max_candidates: 2,
        ..RouterConfig::default()
    };
    let router = router(engine, config)?;
    let images = vec![
        image("img-c", "other/repo", None, None, "rust")?,
        image("img-b", "other/repo", None, None, "rust")?,
        image("img-a", "owner/repo", None, None, "rust tokio linux")?,
    ];
    let commit = "a".repeat(40);
    let result = router
        .rank_env_images(&manifest(Some(&commit), None)?, &images)
        .await?;
    assert_eq!(result.len(), 2);
    assert_eq!(*calls.lock().map_err(|_| "fixture lock")?, 2);
    Ok(())
}
