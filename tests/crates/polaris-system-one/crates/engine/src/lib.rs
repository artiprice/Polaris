// Unit tests of crates/polaris-system-one/crates/engine/src/lib.rs (`tests`); compiled into that module via #[path].
use super::{
    EngineError, TextNormalization, build_task_plans, checked_pii_spans, combine_input,
    hash_engine_inputs, hash_pii_engine_inputs, prepare_text, validate_cpu_device,
};
use gliner2_rs::Entity;
use polaris_sdk::system_one::{DecisionInput, Label, Question, QuestionId, QuestionKind};

#[test]
fn engine_identity_binds_model_runtime_and_normalization_inputs()
-> Result<(), Box<dyn std::error::Error>> {
    const MODEL_FILES: [&str; 9] = [
        "classifier_fp32.onnx",
        "count_lstm_fixed_fp32.onnx",
        "count_pred_argmax_fp32.onnx",
        "encoder_fp32.onnx",
        "schema_gather_fp32.onnx",
        "scorer_fp32.onnx",
        "span_rep_fp32.onnx",
        "token_gather_fp32.onnx",
        "tokenizer.json",
    ];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "polaris-s1-identity-{}-{timestamp}",
        std::process::id()
    ));
    let model = root.join("model");
    std::fs::create_dir_all(&model)?;
    let runtime = root.join("onnxruntime.dll");
    std::fs::write(&runtime, b"runtime-v1")?;
    for filename in MODEL_FILES {
        std::fs::write(model.join(filename), b"model-v1")?;
    }
    let baseline = hash_engine_inputs(&model, &runtime, TextNormalization::Raw, 4)?;
    let changed_normalization =
        hash_engine_inputs(&model, &runtime, TextNormalization::Gliner2Python21, 4)?;
    let changed_threads = hash_engine_inputs(&model, &runtime, TextNormalization::Raw, 2)?;
    std::fs::write(model.join("tokenizer.json"), b"tokenizer-v2")?;
    let changed_model = hash_engine_inputs(&model, &runtime, TextNormalization::Raw, 4)?;
    assert_ne!(baseline, changed_normalization);
    assert_ne!(baseline, changed_threads);
    assert_ne!(baseline, changed_model);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn pii_companion_identity_is_domain_separated_and_span_offsets_are_raw_utf8_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    const MODEL_FILES: [&str; 9] = [
        "classifier_fp32.onnx",
        "count_lstm_fixed_fp32.onnx",
        "count_pred_argmax_fp32.onnx",
        "encoder_fp32.onnx",
        "schema_gather_fp32.onnx",
        "scorer_fp32.onnx",
        "span_rep_fp32.onnx",
        "token_gather_fp32.onnx",
        "tokenizer.json",
    ];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "polaris-s1-pii-identity-{}-{timestamp}",
        std::process::id()
    ));
    let model = root.join("model");
    std::fs::create_dir_all(&model)?;
    let runtime = root.join("onnxruntime.dll");
    std::fs::write(&runtime, b"runtime-v1")?;
    for filename in MODEL_FILES {
        std::fs::write(model.join(filename), b"pii-model-v1")?;
    }
    let pii_identity = hash_pii_engine_inputs(&model, &runtime, 2)?;
    let decision_identity = hash_engine_inputs(&model, &runtime, TextNormalization::Raw, 2)?;
    assert_ne!(pii_identity, decision_identity);
    let raw = "Привет Alice";
    let start = raw.find("Alice").ok_or("missing raw fixture")?;
    let end = start.checked_add("Alice".len()).ok_or("span overflow")?;
    let entity = Entity {
        text: "Alice".to_owned(),
        label: "person".to_owned(),
        score: 0.95,
        char_start: start,
        char_end: end,
        word_start: 0,
        word_end: 0,
        slot: 0,
        task: "entities".to_owned(),
    };
    let overlapping = Entity {
        text: "lice".to_owned(),
        label: "last_name".to_owned(),
        score: 0.8,
        char_start: end.checked_sub(4).ok_or("overlap test span underflow")?,
        char_end: end,
        word_start: 0,
        word_end: 0,
        slot: 0,
        task: "entities".to_owned(),
    };
    let spans = checked_pii_spans(raw, &[overlapping, entity], 32)?;
    assert_eq!(spans.len(), 1);
    let span = spans.first().ok_or("missing checked span")?;
    assert_eq!(usize::try_from(span.start())?, start);
    assert_eq!(usize::try_from(span.end())?, end);
    assert_eq!(raw.get(start..end), Some("Alice"));
    let invalid_utf8 = Entity {
        text: String::new(),
        label: "person".to_owned(),
        score: 0.95,
        char_start: 1,
        char_end: 2,
        word_start: 0,
        word_end: 0,
        slot: 0,
        task: "entities".to_owned(),
    };
    assert!(checked_pii_spans(raw, &[invalid_utf8], 32).is_err());
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cpu_provider_must_be_explicit_and_rejects_device_fallbacks() {
    assert!(matches!(
        validate_cpu_device(None),
        Err(EngineError::InvalidConfig(_))
    ));
    assert!(matches!(
        validate_cpu_device(Some("cuda")),
        Err(EngineError::InvalidConfig(_))
    ));
    assert!(validate_cpu_device(Some("cpu")).is_ok());
}

#[test]
fn configured_python_normalization_changes_actual_model_text() -> Result<(), EngineError> {
    let input = DecisionInput {
        text: "hello".to_owned(),
        context: None,
    };
    let raw = prepare_text(&input, 16, TextNormalization::Raw)?;
    let normalized = prepare_text(&input, 16, TextNormalization::Gliner2Python21)?;
    assert_eq!(raw, "hello");
    assert_eq!(normalized, "hello.");
    let punctuated = DecisionInput {
        text: "hello!".to_owned(),
        context: None,
    };
    assert_eq!(
        prepare_text(&punctuated, 16, TextNormalization::Gliner2Python21)?,
        "hello!"
    );
    Ok(())
}

#[test]
fn input_context_is_included_and_checked_before_inference() {
    let input = DecisionInput {
        text: "primary".to_owned(),
        context: Some("context".to_owned()),
    };
    let combined = combine_input(&input, 16);
    assert!(matches!(combined, Ok(value) if value == "primary\n\ncontext"));
    let too_small = combine_input(&input, 15);
    assert!(matches!(too_small, Err(EngineError::InputTooLarge { .. })));
}

#[test]
fn duplicate_model_prompts_are_rejected_before_inference() -> Result<(), EngineError> {
    let first = Question {
        id: QuestionId::new("first")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Choice {
            options: vec![
                Label::new("a").map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
            ],
        },
    };
    let second = Question {
        id: QuestionId::new("first")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Choice {
            options: vec![
                Label::new("b").map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
            ],
        },
    };
    let result = build_task_plans(&[first, second]);
    assert!(
        matches!(result, Err(EngineError::InvalidSchema(_))),
        "duplicate task names cannot be distinguished in GLiNER results"
    );
    Ok(())
}

#[test]
fn question_ids_are_unique_independently_of_noul_prompts() -> Result<(), EngineError> {
    let make = |statement: &str| -> Result<Question, EngineError> {
        Ok(Question {
            id: QuestionId::new("same_id")
                .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
            kind: QuestionKind::Noul {
                statement: statement.to_owned(),
            },
        })
    };
    let result = build_task_plans(&[make("first statement")?, make("different statement")?]);
    assert!(matches!(result, Err(EngineError::InvalidSchema(_))));
    Ok(())
}

#[test]
fn duplicate_choice_labels_are_rejected_but_reused_labels_are_valid_across_prompts()
-> Result<(), EngineError> {
    let label =
        || Label::new("same").map_err(|error| EngineError::InvalidSchema(error.to_string()));
    let duplicate = Question {
        id: QuestionId::new("dup")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Choice {
            options: vec![label()?, label()?],
        },
    };
    assert!(matches!(
        build_task_plans(&[duplicate]),
        Err(EngineError::InvalidSchema(_))
    ));
    let first = Question {
        id: QuestionId::new("first")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Choice {
            options: vec![label()?],
        },
    };
    let second = Question {
        id: QuestionId::new("second")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Choice {
            options: vec![label()?],
        },
    };
    assert!(build_task_plans(&[first, second]).is_ok());
    Ok(())
}

#[test]
fn oversized_noul_statement_and_aggregate_schema_fail_before_inference() -> Result<(), EngineError>
{
    let too_long = Question {
        id: QuestionId::new("noul")
            .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
        kind: QuestionKind::Noul {
            statement: "x".repeat(4_097),
        },
    };
    assert!(matches!(
        build_task_plans(&[too_long]),
        Err(EngineError::InvalidSchema(_))
    ));
    let large_labels = (0..128)
        .map(|index| {
            Label::new(&format!("{}-{index:03}", "x".repeat(116)))
                .map_err(|error| EngineError::InvalidSchema(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let large_schema = (0..5)
        .map(|index| -> Result<Question, EngineError> {
            Ok(Question {
                id: QuestionId::new(&format!("q{index}"))
                    .map_err(|error| EngineError::InvalidSchema(error.to_string()))?,
                kind: QuestionKind::Choice {
                    options: large_labels.clone(),
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert!(matches!(
        build_task_plans(&large_schema),
        Err(EngineError::InvalidSchema(_))
    ));
    Ok(())
}
