use polaris_sdk::system_one::{
    DecisionSchemaCalibrationState, DecisionSchemaCatalog, DecisionSchemaFailMode,
    DecisionSchemaIntegrity, QuestionKind,
};
use polaris_system_one_schemas::{SchemaSet, SdkSchemaCatalog};

#[test]
fn sdk_adapter_returns_the_canonical_versioned_skill_schema() {
    let set = SchemaSet::load_builtin().expect("embedded schemas validate");
    let catalog = SdkSchemaCatalog::new(&set);
    let schema = catalog
        .latest("skill.relevance")
        .expect("conversion succeeds")
        .expect("skill relevance schema exists");

    assert_eq!(schema.id(), "skill.relevance");
    assert_eq!(schema.version(), 1);
    assert_eq!(schema.max_chars(), 65_536);
    assert_eq!(schema.fail_policy().consumer(), "skill_select");
    assert_eq!(schema.fail_policy().deadline_ms(), 750);
    assert_eq!(
        schema.fail_policy().on_timeout(),
        DecisionSchemaFailMode::Open
    );
    assert_eq!(
        schema.fail_policy().integrity(),
        DecisionSchemaIntegrity::Normal
    );
    assert_eq!(
        schema.calibration().state(),
        DecisionSchemaCalibrationState::Uncalibrated
    );
    assert_eq!(schema.questions().len(), 1);
    assert!(matches!(
        &schema.questions()[0].kind,
        QuestionKind::Noul { statement } if statement == "The skill applies to the stated task."
    ));
    assert_eq!(schema.thresholds().reject().get(), 0.1);
    assert_eq!(schema.thresholds().accept().get(), 0.9);
}

#[test]
fn sdk_adapter_distinguishes_absence_from_invalid_conversion() {
    let set = SchemaSet::load_builtin().expect("embedded schemas validate");
    let catalog = SdkSchemaCatalog::new(&set);
    assert!(
        catalog
            .latest("missing.schema")
            .expect("valid lookup")
            .is_none()
    );
    assert!(
        catalog
            .get("skill.relevance", 99)
            .expect("valid lookup")
            .is_none()
    );
}

#[test]
fn sdk_adapter_preserves_calibration_provenance() {
    let fixture_digest = "a".repeat(64);
    let checkpoint_digest = "b".repeat(64);
    let source = format!(
        r#"
[[schemas]]
id = "skill.relevance"
version = 1
kind = "skill_relevance"
max_chars = 4096
questions = [{{ id = "relevant", kind = {{ type = "noul", statement = "The skill applies to the stated task." }} }}]
thresholds = {{ accept = 0.9, reject = 0.1, abstain_band = [0.4, 0.6] }}
fail = {{ consumer = "skill_select", on_timeout = "open", deadline_ms = 750, integrity = "normal" }}
calibration = {{ status = "calibrated", fixture_set = "skill-fixtures-v1", fixture_blake3 = "{fixture_digest}", n = 20, precision_target = 0.8, observed_precision = 0.9, observed_recall = 0.85, ece = 0.04, checkpoint_blake3 = "{checkpoint_digest}", date = "2026-10-01T00:00:00Z" }}
"#
    );
    let set = SchemaSet::from_toml(&source).expect("calibrated schema validates");
    let schema = SdkSchemaCatalog::new(&set)
        .latest("skill.relevance")
        .expect("conversion succeeds")
        .expect("schema exists");
    let calibration = schema.calibration();
    assert_eq!(calibration.fixture_set(), "skill-fixtures-v1");
    assert_eq!(calibration.fixture_blake3(), Some(fixture_digest.as_str()));
    assert_eq!(
        calibration.checkpoint_blake3(),
        Some(checkpoint_digest.as_str())
    );
    assert_eq!(calibration.sample_count(), 20);
    assert_eq!(calibration.precision_target(), Some(0.8));
    assert_eq!(calibration.observed_precision(), Some(0.9));
    assert_eq!(calibration.observed_recall(), Some(0.85));
    assert_eq!(calibration.expected_calibration_error(), Some(0.04));
    assert_eq!(calibration.date(), Some("2026-10-01T00:00:00Z"));
}
