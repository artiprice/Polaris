use std::error::Error;
use std::path::PathBuf;

use polaris_sdk::system_one::{Answer, DecisionInput, Label, Question, QuestionId, QuestionKind};
use polaris_system_one_engine::{EngineConfig, GlinerEngine, TextNormalization};

/// Runs against the locally installed fp32 checkpoint and ORT DLL when explicitly configured.
#[test]
#[ignore = "requires explicit local GLiNER model and ORT DLL paths"]
fn real_model_returns_bounded_choice_probabilities() -> Result<(), Box<dyn Error>> {
    let model_dir = std::env::var_os("POLARIS_S1_MODEL_DIR")
        .map(PathBuf::from)
        .ok_or("POLARIS_S1_MODEL_DIR is required for the real-model check")?;
    let ort_dylib = std::env::var_os("POLARIS_ORT_DYLIB")
        .map(PathBuf::from)
        .ok_or("POLARIS_ORT_DYLIB is required for the real-model check")?;
    let config = EngineConfig::new(model_dir, ort_dylib, TextNormalization::Gliner2Python21);
    let engine = GlinerEngine::load(config)?;
    let question = Question {
        id: QuestionId::new("triage_kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?, Label::new("feature")?],
        },
    };
    let input = DecisionInput {
        text: "Fix the parser when an empty file is provided".to_owned(),
        context: None,
    };
    let decision = engine.decide(&[question], &input)?;
    let answer = decision.answers.first().ok_or("model returned no answer")?;
    let Answer::Choice { selected, probs } = &answer.answer else {
        return Err("model returned the wrong answer kind".into());
    };

    assert_eq!(probs.len(), 2, "one probability per configured choice");
    assert!(
        probs
            .iter()
            .all(|score| (0.0..=1.0).contains(&score.probability.get())),
        "each score must be a model probability"
    );
    assert!(
        probs.iter().any(|score| score.label == *selected),
        "selected label must come from the model result"
    );
    Ok(())
}
