//! Questionnaire State (descriptor: specs/descriptors/uimodel_questionnaire_state.json).

use polaris_ui_model::questionnaire::{
    AnswerSubmitError, MAX_ANSWER_BYTES, MAX_FIELD_BYTES, MultiChoiceAnswer, QuestionAnswer,
    QuestionKind, QuestionOption, QuestionSpec, QuestionnaireAction as A, QuestionnaireFocus,
    QuestionnaireSpec, QuestionnaireState,
};
use std::error::Error;
type TestResult = Result<(), Box<dyn Error>>;

fn request() -> QuestionnaireSpec {
    QuestionnaireSpec {
        request_id: "req-1".to_owned(),
        revision: 7,
        requester_label: "Frame 3".to_owned(),
        questions: vec![
            QuestionSpec {
                id: "choice".to_owned(),
                prompt: "Pick".to_owned(),
                required: true,
                kind: QuestionKind::MultiChoice {
                    options: vec![
                        QuestionOption {
                            id: "a".to_owned(),
                            label: "A".to_owned(),
                        },
                        QuestionOption {
                            id: "b".to_owned(),
                            label: "B".to_owned(),
                        },
                    ],
                },
            },
            QuestionSpec {
                id: "text".to_owned(),
                prompt: "Explain".to_owned(),
                required: true,
                kind: QuestionKind::FreeText,
            },
            QuestionSpec {
                id: "optional".to_owned(),
                prompt: "Optional".to_owned(),
                required: false,
                kind: QuestionKind::SingleChoice {
                    options: vec![QuestionOption {
                        id: "x".to_owned(),
                        label: "X".to_owned(),
                    }],
                },
            },
        ],
    }
}
fn state() -> Result<QuestionnaireState, Box<dyn Error>> {
    Ok(QuestionnaireState::new(request())?)
}

#[test]
fn questions_start_with_no_selection() -> TestResult {
    let s = state()?;
    assert_eq!(s.question_index(), 0);
    assert_eq!(s.focus(), QuestionnaireFocus::Choices);
    assert_eq!(s.answered_count(), 0);
    assert!(matches!(
        s.prepare_answer(),
        Err(AnswerSubmitError::Incomplete(_))
    ));
    Ok(())
}
#[test]
fn none_of_the_above_is_explicit_and_exclusive() -> TestResult {
    let mut s = state()?;
    s.apply(A::ToggleChoice)?;
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    assert_eq!(s.selected_named_options(), vec!["a", "b"]);
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    assert!(s.selected_named_options().is_empty());
    assert!(s.none_of_the_above_selected());
    s.apply(A::ChoiceUp)?;
    s.apply(A::ToggleChoice)?;
    assert!(!s.none_of_the_above_selected());
    assert_eq!(s.selected_named_options(), vec!["b"]);
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    assert!(s.none_of_the_above_selected());
    assert!(s.selected_named_options().is_empty());
    Ok(())
}
#[test]
fn multi_choice_toggles_named_options() -> TestResult {
    let mut s = state()?;
    s.apply(A::ToggleChoice)?;
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    assert_eq!(s.selected_named_options(), vec!["a", "b"]);
    s.apply(A::ChoiceUp)?;
    s.apply(A::ToggleChoice)?;
    assert_eq!(s.selected_named_options(), vec!["b"]);
    Ok(())
}
#[test]
fn required_answers_block_atomic_submit_and_optional_is_unanswered() -> TestResult {
    let mut s = state()?;
    s.apply(A::ChoiceDown)?;
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    s.navigate(1);
    s.apply(A::InsertText("because".to_owned()))?;
    let batch = s.prepare_answer()?;
    assert_eq!(
        batch
            .answers
            .first()
            .ok_or("missing answer for first question")?
            .answer,
        QuestionAnswer::MultiChoice(MultiChoiceAnswer::NoneOfTheAbove)
    );
    assert_eq!(
        batch
            .answers
            .get(1)
            .ok_or("missing answer for second question")?
            .answer,
        QuestionAnswer::FreeText("because".to_owned())
    );
    assert_eq!(
        batch
            .answers
            .get(2)
            .ok_or("missing answer for third question")?
            .answer,
        QuestionAnswer::Unanswered
    );
    Ok(())
}
#[test]
fn required_free_text_needs_nonblank_answer_and_notes_are_optional() -> TestResult {
    let mut s = state()?;
    s.apply(A::ChoiceDown)?;
    s.apply(A::ToggleChoice)?;
    s.navigate(1);
    s.apply(A::InsertText("   ".to_owned()))?;
    assert!(matches!(
        s.prepare_answer(),
        Err(AnswerSubmitError::Incomplete(_))
    ));
    s.apply(A::InsertText("answer".to_owned()))?;
    assert!(s.prepare_answer().is_ok());
    Ok(())
}
#[test]
fn navigation_preserves_per_question_answers_and_notes() -> TestResult {
    let mut s = state()?;
    s.apply(A::ToggleChoice)?;
    s.apply(A::FocusNotes)?;
    assert_eq!(s.focus(), QuestionnaireFocus::Notes);
    s.apply(A::InsertText("note".to_owned()))?;
    s.navigate(1);
    assert_eq!(s.focus(), QuestionnaireFocus::Choices);
    s.navigate(-1);
    assert_eq!(s.selected_named_options(), vec!["a"]);
    assert_eq!(s.notes(), "note");
    Ok(())
}
#[test]
fn oversized_field_rejects_without_truncating() -> TestResult {
    let mut s = state()?;
    let too_big = "x".repeat(MAX_FIELD_BYTES + 1);
    assert!(s.apply(A::InsertText(too_big)).is_err());
    assert_eq!(s.answer_text(), "");
    Ok(())
}
#[test]
fn invalid_choice_kind_cannot_be_toggled() -> TestResult {
    let mut spec = request();
    spec.questions = vec![QuestionSpec {
        id: "free".to_owned(),
        prompt: "Free".to_owned(),
        required: true,
        kind: QuestionKind::FreeText,
    }];
    let mut s = QuestionnaireState::new(spec)?;
    assert!(s.apply(A::ToggleChoice).is_err());
    Ok(())
}
#[test]
fn questionnaire_plan_caps_reject_excess_questions_and_options() -> TestResult {
    let mut too_many_questions = request();
    too_many_questions.questions = (0..9)
        .map(|index| QuestionSpec {
            id: format!("q{index}"),
            prompt: "Prompt".to_owned(),
            required: false,
            kind: QuestionKind::FreeText,
        })
        .collect();
    assert_eq!(
        QuestionnaireState::new(too_many_questions).err(),
        Some(polaris_ui_model::questionnaire::QuestionnaireError::TooManyQuestions)
    );

    let mut too_many_options = request();
    too_many_options.questions.truncate(1);
    if let Some(question) = too_many_options.questions.first_mut() {
        question.kind = QuestionKind::MultiChoice {
            options: (0..17)
                .map(|index| QuestionOption {
                    id: format!("o{index}"),
                    label: format!("Option {index}"),
                })
                .collect(),
        };
    }
    assert_eq!(
        QuestionnaireState::new(too_many_options).err(),
        Some(polaris_ui_model::questionnaire::QuestionnaireError::TooManyOptions)
    );
    Ok(())
}

#[test]
fn request_payload_cap_counts_aggregate_metadata() -> TestResult {
    let questions = (0..8)
        .map(|question| QuestionSpec {
            id: format!("q{question}"),
            prompt: "P".to_owned(),
            required: false,
            kind: QuestionKind::MultiChoice {
                options: (0..16)
                    .map(|option| QuestionOption {
                        id: format!("o{question}-{option}"),
                        label: "x".repeat(2_048),
                    })
                    .collect(),
            },
        })
        .collect();
    let result = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "bounded".to_owned(),
        revision: 1,
        requester_label: "Frame".to_owned(),
        questions,
    });
    assert_eq!(
        result.err(),
        Some(polaris_ui_model::questionnaire::QuestionnaireError::FieldTooLarge)
    );
    Ok(())
}

#[test]
fn oversized_selected_option_ids_block_batch_without_losing_selections() -> TestResult {
    let mut spec = request();
    spec.questions.truncate(1);
    if let Some(question) = spec.questions.first_mut() {
        question.kind = QuestionKind::MultiChoice {
            options: (0..16)
                .map(|index| QuestionOption {
                    id: format!("{index}-{}", "i".repeat(8 * 1024)),
                    label: format!("Option {index}"),
                })
                .collect(),
        };
    }
    let mut state = QuestionnaireState::new(spec)?;
    assert!(state.remaining_answer_bytes() <= MAX_ANSWER_BYTES);
    for index in 0..16 {
        state.apply(A::ToggleChoice)?;
        if index < 15 {
            state.apply(A::ChoiceDown)?;
        }
    }
    let selected = state.selected_named_options();
    assert_eq!(selected.len(), 16);
    assert_eq!(state.prepare_answer(), Err(AnswerSubmitError::TooLarge));
    assert_eq!(state.selected_named_options(), selected);
    assert_eq!(state.remaining_answer_bytes(), 0);
    Ok(())
}

#[test]
fn answer_budget_rejects_without_truncating_notes_or_answer() -> TestResult {
    let questions = (0..8)
        .map(|index| QuestionSpec {
            id: format!("q{index}"),
            prompt: "Prompt".to_owned(),
            required: false,
            kind: QuestionKind::FreeText,
        })
        .collect();
    let mut state = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "bounded".to_owned(),
        revision: 1,
        requester_label: "Frame".to_owned(),
        questions,
    })?;
    let text = "x".repeat(16 * 1024);
    for _ in 0..3 {
        state.apply(A::InsertText(text.clone()))?;
        state.apply(A::FocusNotes)?;
        state.apply(A::InsertText(text.clone()))?;
        state.navigate(1);
    }
    state.apply(A::InsertText(text.clone()))?;
    state.apply(A::FocusNotes)?;
    let before = state.answer_text().to_owned();
    assert_eq!(
        state.apply(A::InsertText(text)),
        Err(polaris_ui_model::questionnaire::QuestionnaireError::FieldTooLarge)
    );
    assert_eq!(state.answer_text(), before);
    Ok(())
}

#[test]
fn single_choice_and_none_are_mutually_exclusive() -> TestResult {
    let mut spec = request();
    spec.questions.truncate(1);
    if let Some(question) = spec.questions.first_mut() {
        question.kind = QuestionKind::SingleChoice {
            options: vec![
                QuestionOption {
                    id: "a".to_owned(),
                    label: "A".to_owned(),
                },
                QuestionOption {
                    id: "b".to_owned(),
                    label: "B".to_owned(),
                },
            ],
        };
    }
    let mut state = QuestionnaireState::new(spec)?;
    state.apply(A::ToggleChoice)?;
    state.apply(A::ChoiceDown)?;
    state.apply(A::ToggleChoice)?;
    assert_eq!(state.selected_named_options(), vec!["b"]);
    state.apply(A::ChoiceDown)?;
    state.apply(A::ToggleChoice)?;
    assert!(state.selected_named_options().is_empty());
    assert!(state.none_of_the_above_selected());
    Ok(())
}
