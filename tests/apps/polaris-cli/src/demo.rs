// Unit tests of apps/polaris-cli/src/demo.rs (`tests`); compiled into that module via #[path].
use super::{DemoError, DemoRuntime, MAX_DEMO_STREAM_BYTES};
use polaris_ui_model::questionnaire::QuestionnaireAction;
use polaris_ui_model::view::EntryKind;
use std::{error::Error, time::Duration};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn runtime_is_local_bounded_and_runs_scripted_beats() -> TestResult {
    let mut runtime = DemoRuntime::new()?;
    runtime.submit("show the Unicode stream questionnaire")?;
    assert!(runtime.busy());
    assert!(runtime.tick(Duration::from_millis(1_800))?);
    assert!(!runtime.busy());
    let texts: Vec<&str> = runtime
        .view()
        .conversation
        .iter()
        .map(|entry| entry.text.as_str())
        .collect();
    assert!(texts.iter().all(|text| text.starts_with("[DEMO]")));
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Tool: inspect_workspace"))
    );
    assert!(texts.iter().any(|text| text.contains("Diff preview")));
    assert!(texts.iter().any(|text| text.contains("provider timeout")));
    assert!(texts.iter().any(|text| text.contains("Retrying")));
    assert!(texts.iter().any(|text| text.contains("こんにちは")));
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Stream:") || text.contains("This response"))
            .count(),
        1,
        "incremental output must remain one assistant entry"
    );
    assert!(texts.iter().any(|text| text.contains("👩‍💻")));
    let stream = runtime
        .view()
        .conversation
        .iter()
        .find(|entry| entry.kind == EntryKind::Assistant)
        .ok_or_else(|| std::io::Error::other("assistant stream missing"))?;
    assert!(stream.text.len() <= MAX_DEMO_STREAM_BYTES);
    assert!(runtime.question().is_some());
    assert!(matches!(runtime.submit("another"), Err(DemoError::Busy)));
    Ok(())
}

#[test]
fn questionnaire_acceptance_is_atomic_and_contains_all_questions() -> TestResult {
    let mut runtime = DemoRuntime::new()?;
    runtime.submit("questionnaire practice")?;
    runtime.tick(Duration::from_secs(2))?;
    let question = runtime
        .question()
        .ok_or("questionnaire was not scheduled")?;
    assert_eq!(question.spec().questions.len(), 3);
    assert!(runtime.accept_answer().is_err());
    assert!(
        runtime.question().is_some(),
        "incomplete batch must retain all drafts"
    );

    let question = runtime.question_mut().ok_or("questionnaire missing")?;
    question.apply(QuestionnaireAction::ToggleChoice)?;
    question.navigate(1);
    question.apply(QuestionnaireAction::ToggleChoice)?;
    question.navigate(1);
    question.apply(QuestionnaireAction::FocusNotes)?;
    question.apply(QuestionnaireAction::InsertText("DEMO note".to_owned()))?;

    let batch = runtime.accept_answer()?;
    assert_eq!(batch.answers.len(), 3);
    assert!(runtime.question().is_none());
    assert!(
        runtime
            .view()
            .conversation
            .last()
            .is_some_and(|entry| entry.text.contains("nothing was sent"))
    );
    Ok(())
}

#[test]
fn cancel_reset_and_replay_have_bounded_local_lifecycles() -> TestResult {
    let mut runtime = DemoRuntime::new()?;
    assert!(matches!(runtime.replay(), Err(DemoError::NoReplay)));
    runtime.submit("retryable demo")?;
    runtime.tick(Duration::from_secs(10))?;
    runtime.cancel()?;
    assert!(!runtime.busy());
    assert!(
        runtime
            .view()
            .conversation
            .last()
            .is_some_and(|entry| entry.text.starts_with("[DEMO]"))
    );
    runtime.replay()?;
    assert!(runtime.busy());
    assert!(!runtime.tick(Duration::from_secs(10))?);
    assert!(runtime.tick(Duration::from_millis(10_180))?);
    runtime.reset()?;
    assert!(!runtime.busy());
    assert!(runtime.view().conversation.is_empty());
    assert!(matches!(runtime.replay(), Err(DemoError::NoReplay)));
    Ok(())
}

#[test]
fn long_stream_is_incremental_in_one_bounded_entry() -> TestResult {
    let mut runtime = DemoRuntime::new()?;
    runtime.submit("please run the long demo questionnaire")?;
    assert!(runtime.tick(Duration::from_millis(1_260))?);
    let assistant_count = runtime
        .view()
        .conversation
        .iter()
        .filter(|entry| entry.kind == EntryKind::Assistant)
        .count();
    assert_eq!(assistant_count, 1);
    let partial_len = runtime
        .view()
        .conversation
        .last()
        .ok_or("stream entry missing")?
        .text
        .len();
    assert!(partial_len > 7);

    runtime.tick(Duration::from_millis(4_200))?;
    let streams: Vec<&str> = runtime
        .view()
        .conversation
        .iter()
        .filter(|entry| entry.kind == EntryKind::Assistant)
        .map(|entry| entry.text.as_str())
        .collect();
    assert_eq!(streams.len(), 1);
    let stream = streams.first().ok_or("long assistant stream missing")?;
    assert!(stream.len() <= MAX_DEMO_STREAM_BYTES);
    assert!(stream.len() > 4_000);
    assert!(runtime.question().is_some());
    Ok(())
}
