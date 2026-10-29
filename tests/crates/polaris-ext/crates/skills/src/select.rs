// Unit tests of crates/polaris-ext/crates/skills/src/select.rs (`tests`); compiled into that module via #[path].
use std::future;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use polaris_sdk::registry::{DecisionEngine, DecisionError};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionSchema, DecisionSchemaCalibration, DecisionSchemaCalibrationRecord,
    DecisionSchemaCalibrationState, DecisionSchemaCatalog, DecisionSchemaFailMode,
    DecisionSchemaFailPolicy, DecisionSchemaIntegrity, DecisionSchemaThresholds, QuestionAnswer,
};

use super::*;
use crate::{SkillRoot, SkillSource};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn registry() -> (Self, SkillRegistry) {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "polaris-skill-select-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("git-bisect")).expect("create test skill directory");
        std::fs::create_dir_all(root.join("db-migrate")).expect("create test skill directory");
        std::fs::write(
                root.join("git-bisect/SKILL.md"),
                "---\nname: git-bisect\ndescription: Find the bad commit in git history.\nrequires_tools: [git_read]\n---\nprivate body sentinel: NEVER SEND THIS",
            )
            .expect("write git skill");
        std::fs::write(
                root.join("db-migrate/SKILL.md"),
                "---\nname: db-migrate\ndescription: Plan a safe database schema migration.\n---\nprivate database body",
            )
            .expect("write database skill");
        let registry = SkillRegistry::discover(&[SkillRoot::user_global(&root)])
            .expect("discover test skills");
        (Self(root), registry)
    }

    fn repo_registry() -> (Self, SkillRegistry) {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "polaris-skill-select-repo-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("repo-only")).expect("create repository skill directory");
        std::fs::write(
            root.join("repo-only/SKILL.md"),
            "---\nname: repo-only\ndescription: Repository-only skill.\n---\nprivate body",
        )
        .expect("write repository skill");
        let registry = SkillRegistry::discover(&[SkillRoot::repository(&root)])
            .expect("discover repository skill");
        (Self(root), registry)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestCatalog(DecisionSchema);

impl DecisionSchemaCatalog for TestCatalog {
    fn get(
        &self,
        id: &str,
        version: u32,
    ) -> Result<Option<DecisionSchema>, polaris_sdk::system_one::DecisionSchemaError> {
        Ok((self.0.id() == id && self.0.version() == version).then(|| self.0.clone()))
    }

    fn latest(
        &self,
        id: &str,
    ) -> Result<Option<DecisionSchema>, polaris_sdk::system_one::DecisionSchemaError> {
        Ok((self.0.id() == id).then(|| self.0.clone()))
    }
}

fn test_catalog(mode: DecisionSchemaFailMode) -> TestCatalog {
    let calibration = DecisionSchemaCalibration::try_new(DecisionSchemaCalibrationRecord {
        state: DecisionSchemaCalibrationState::Uncalibrated,
        fixture_set: String::new(),
        fixture_blake3: None,
        sample_count: 0,
        precision_target: None,
        observed_precision: None,
        observed_recall: None,
        expected_calibration_error: None,
        checkpoint_blake3: None,
        date: None,
    })
    .expect("valid test calibration");
    let thresholds = DecisionSchemaThresholds::new(
        polaris_sdk::system_one::Probability::new(0.9).expect("valid probability"),
        polaris_sdk::system_one::Probability::new(0.1).expect("valid probability"),
        polaris_sdk::system_one::Probability::new(0.4).expect("valid probability"),
        polaris_sdk::system_one::Probability::new(0.6).expect("valid probability"),
    )
    .expect("valid test thresholds");
    let fail = DecisionSchemaFailPolicy::new(
        "skill_select".to_owned(),
        mode,
        750,
        DecisionSchemaIntegrity::Normal,
    )
    .expect("valid test failure policy");
    let question = Question {
        id: QuestionId::new("relevant").expect("valid canonical question id"),
        kind: QuestionKind::Noul {
            statement: "The skill applies to the stated task.".to_owned(),
        },
    };
    TestCatalog(
        DecisionSchema::try_new(
            "skill.relevance".to_owned(),
            1,
            65_536,
            vec![question],
            thresholds,
            fail,
            calibration,
        )
        .expect("valid test schema"),
    )
}

type CapturedDecision = Arc<Mutex<Option<(String, Vec<String>)>>>;

struct CaptureEngine(CapturedDecision);

impl DecisionEngine for CaptureEngine {
    fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> impl Future<Output = Result<Decision, DecisionError>> + Send {
        if let Ok(mut captured) = self.0.lock() {
            let statements = schema
                .iter()
                .filter_map(|question| match &question.kind {
                    QuestionKind::Noul { statement } => Some(statement.clone()),
                    _ => None,
                })
                .collect();
            *captured = Some((input.text.clone(), statements));
        }
        let answers = schema
            .iter()
            .map(|question| QuestionAnswer {
                question: question.id.clone(),
                answer: Answer::Noul {
                    yes: polaris_sdk::system_one::Probability::new(
                        if question.id.as_str() == "skill_000001" {
                            0.99
                        } else {
                            0.05
                        },
                    )
                    .expect("valid probability"),
                },
            })
            .collect();
        future::ready(Ok(Decision {
            answers,
            latency_us: 1,
        }))
    }
}

struct ErrorEngine;

impl DecisionEngine for ErrorEngine {
    fn decide(
        &self,
        _schema: &[Question],
        _input: &DecisionInput,
    ) -> impl Future<Output = Result<Decision, DecisionError>> + Send {
        future::ready(Err(DecisionError::Unavailable(
            "test engine down".to_owned(),
        )))
    }
}

struct SlowEngine;

impl DecisionEngine for SlowEngine {
    async fn decide(
        &self,
        _schema: &[Question],
        _input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        tokio::time::sleep(Duration::from_millis(800)).await;
        Err(DecisionError::Timeout)
    }
}

#[tokio::test]
async fn selection_sends_descriptions_but_never_skill_bodies() {
    let (_tree, registry) = TempTree::registry();
    let captured = Arc::new(Mutex::new(None));
    let engine = CaptureEngine(Arc::clone(&captured));
    let catalog = test_catalog(DecisionSchemaFailMode::Open);
    let selected = registry
        .select(
            "find a bad commit",
            &catalog,
            &engine,
            TopK::new(1).expect("valid k"),
        )
        .await
        .expect("selection should succeed");

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].name.as_str(), "git-bisect");
    assert_eq!(selected[0].reason, SelectionReason::SystemOne);
    assert_eq!(registry.list()[0].source, SkillSource::UserGlobal);
    let capture = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("engine called");
    let (prompt, statements) = capture;
    assert!(prompt.contains("Find the bad commit in git history."));
    assert!(!prompt.contains("NEVER SEND THIS"));
    assert!(!prompt.contains("private database body"));
    assert!(statements[1].contains("skill_000001"));
    assert!(statements[1].ends_with("The skill applies to the stated task."));
    assert_eq!(selected[0].relevance, Some(RelevanceVerdict::Accepted));
    assert_eq!(selected[0].schema_version, Some(1));
    assert_eq!(
        selected[0].calibration,
        Some(DecisionSchemaCalibrationState::Uncalibrated)
    );
}

#[tokio::test]
async fn engine_failure_uses_stable_lexical_fallback_and_bounds_k() {
    let (_tree, registry) = TempTree::registry();
    let catalog = test_catalog(DecisionSchemaFailMode::Open);
    let first = registry.select_lexical("database migration", TopK::new(2).expect("valid k"));
    let second = registry
        .select(
            "database migration",
            &catalog,
            &ErrorEngine,
            TopK::new(2).expect("valid k"),
        )
        .await
        .expect("fallback should succeed");
    assert_eq!(
        first.iter().map(|item| &item.name).collect::<Vec<_>>(),
        second.iter().map(|item| &item.name).collect::<Vec<_>>()
    );
    assert_eq!(second[0].name.as_str(), "db-migrate");
    assert!(
        first
            .iter()
            .all(|item| item.reason == SelectionReason::LexicalFallback)
    );
    assert!(TopK::new(0).is_err());
    assert!(TopK::new(33).is_err());
}

#[tokio::test]
async fn closed_schema_policy_never_uses_lexical_fallback() {
    let (_tree, registry) = TempTree::registry();
    let catalog = test_catalog(DecisionSchemaFailMode::Closed);
    assert!(matches!(
        registry
            .select(
                "database migration",
                &catalog,
                &ErrorEngine,
                TopK::new(2).expect("valid k")
            )
            .await,
        Err(SkillError::ClosedSelection(_))
    ));
}

#[tokio::test]
async fn selection_rejects_noncanonical_multi_question_schema() {
    let (_tree, registry) = TempTree::registry();
    let base = test_catalog(DecisionSchemaFailMode::Open).0;
    let mut questions = base.questions().to_vec();
    questions.push(Question {
        id: QuestionId::new("unexpected").expect("valid test question id"),
        kind: QuestionKind::Noul {
            statement: "An extra question must not be accepted.".to_owned(),
        },
    });
    let schema = DecisionSchema::try_new(
        base.id().to_owned(),
        base.version(),
        base.max_chars(),
        questions,
        base.thresholds(),
        base.fail_policy().clone(),
        base.calibration().clone(),
    )
    .expect("multi-question schema is otherwise valid");
    let catalog = TestCatalog(schema);

    assert!(matches!(
        registry
            .select(
                "database migration",
                &catalog,
                &ErrorEngine,
                TopK::new(2).expect("valid k"),
            )
            .await,
        Err(SkillError::InvalidSelectionSchema)
    ));
}

#[tokio::test]
async fn schema_deadline_uses_lexical_fallback_only_when_open() {
    let (_tree, registry) = TempTree::registry();
    let catalog = test_catalog(DecisionSchemaFailMode::Open);
    let selected = registry
        .select(
            "database migration",
            &catalog,
            &SlowEngine,
            TopK::new(2).expect("valid k"),
        )
        .await
        .expect("open timeout falls back");
    assert_eq!(selected[0].name.as_str(), "db-migrate");
    assert!(
        selected
            .iter()
            .all(|item| item.reason == SelectionReason::LexicalFallback)
    );
    assert!(
        selected
            .iter()
            .all(|item| { item.calibration == Some(DecisionSchemaCalibrationState::Uncalibrated) })
    );
}

#[test]
fn lexical_and_system_one_candidates_exclude_unapproved_repository_skills() {
    let (_tree, registry) = TempTree::repo_registry();
    assert_eq!(registry.list()[0].source, SkillSource::RepoUnapproved);
    assert!(
        registry
            .select_lexical("repository skill", TopK::new(2).expect("valid k"))
            .is_empty()
    );
}

#[test]
fn required_tools_are_reported_without_being_granted() {
    let (_tree, registry) = TempTree::registry();
    let mut ranked = registry.select_lexical("git history", TopK::new(2).expect("valid k"));
    registry.with_missing_tools(&mut ranked, &BTreeSet::new());
    let git = ranked
        .iter()
        .find(|item| item.name.as_str() == "git-bisect")
        .expect("git skill is ranked");
    assert_eq!(git.missing_tools, ["git_read"]);
    assert!(BTreeSet::<String>::new().is_empty());
}
