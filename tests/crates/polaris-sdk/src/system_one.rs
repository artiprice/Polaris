// Unit tests of crates/polaris-sdk/src/system_one.rs (`decision_schema_tests`); compiled into that module via #[path].
use super::*;

fn uncalibrated() -> DecisionSchemaCalibration {
    DecisionSchemaCalibration::try_new(DecisionSchemaCalibrationRecord {
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
    .expect("valid uncalibrated provenance")
}

fn schema() -> DecisionSchema {
    DecisionSchema::try_new(
        "skill.relevance".to_owned(),
        1,
        65_536,
        vec![Question {
            id: QuestionId::new("relevant").expect("valid id"),
            kind: QuestionKind::Noul {
                statement: "The skill applies to the stated task.".to_owned(),
            },
        }],
        DecisionSchemaThresholds::new(
            Probability::new(0.9).expect("valid probability"),
            Probability::new(0.1).expect("valid probability"),
            Probability::new(0.4).expect("valid probability"),
            Probability::new(0.6).expect("valid probability"),
        )
        .expect("valid thresholds"),
        DecisionSchemaFailPolicy::new(
            "skill_select".to_owned(),
            DecisionSchemaFailMode::Open,
            750,
            DecisionSchemaIntegrity::Normal,
        )
        .expect("valid fail policy"),
        uncalibrated(),
    )
    .expect("valid schema snapshot")
}

#[test]
fn validated_schema_snapshot_preserves_version_policy_and_calibration_state() {
    let schema = schema();
    assert_eq!(schema.id(), "skill.relevance");
    assert_eq!(schema.version(), 1);
    assert_eq!(schema.questions()[0].id.as_str(), "relevant");
    assert_eq!(schema.fail_policy().deadline_ms(), 750);
    assert_eq!(
        schema.calibration().state(),
        DecisionSchemaCalibrationState::Uncalibrated
    );
    assert_eq!(schema.thresholds().reject().get(), 0.1);
}

#[test]
fn schema_snapshot_rejects_duplicate_question_ids_and_bad_calibration() {
    let mut duplicate = schema();
    duplicate.questions.push(duplicate.questions[0].clone());
    assert!(matches!(
        DecisionSchema::try_new(
            duplicate.id,
            duplicate.version,
            duplicate.max_chars,
            duplicate.questions,
            duplicate.thresholds,
            duplicate.fail,
            duplicate.calibration,
        ),
        Err(DecisionSchemaError::InvalidQuestion)
    ));

    assert!(matches!(
        DecisionSchemaCalibration::try_new(DecisionSchemaCalibrationRecord {
            state: DecisionSchemaCalibrationState::Calibrated,
            fixture_set: "fixtures".to_owned(),
            fixture_blake3: Some("bad".to_owned()),
            sample_count: 10,
            precision_target: Some(0.9),
            observed_precision: Some(0.95),
            observed_recall: Some(0.9),
            expected_calibration_error: Some(0.05),
            checkpoint_blake3: Some("a".repeat(64)),
            date: Some("2026-01-01T00:00:00Z".to_owned()),
        }),
        Err(DecisionSchemaError::InvalidCalibration)
    ));
}
