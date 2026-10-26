//! Question notes use the same Unicode editing rules as the composer.
use polaris_ui_model::questionnaire::{
    QuestionKind, QuestionOption, QuestionSpec, QuestionnaireAction as A, QuestionnaireSpec,
    QuestionnaireState,
};

fn state() -> Result<QuestionnaireState, Box<dyn std::error::Error>> {
    Ok(QuestionnaireState::new(QuestionnaireSpec {
        request_id: "editor".to_owned(),
        revision: 1,
        requester_label: "demo".to_owned(),
        questions: vec![QuestionSpec {
            id: "q".to_owned(),
            prompt: "Choose".to_owned(),
            required: true,
            kind: QuestionKind::SingleChoice {
                options: vec![QuestionOption {
                    id: "a".to_owned(),
                    label: "A".to_owned(),
                }],
            },
        }],
    })?)
}

#[test]
fn notes_backspace_removes_one_grapheme_not_one_scalar() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.apply(A::FocusNotes)?;
    state.apply(A::InsertText("a e\u{301}".to_owned()))?;
    state.apply(A::Backspace)?;
    assert_eq!(
        state.notes(),
        "a ",
        "combining sequence must be deleted atomically"
    );
    Ok(())
}

#[test]
fn notes_support_middle_edit_selection_and_undo() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.apply(A::FocusNotes)?;
    state.apply(A::InsertText("a界z".to_owned()))?;
    state.apply(A::MoveLeft {
        extend: false,
        word: false,
    })?;
    state.apply(A::InsertText("X".to_owned()))?;
    assert_eq!(state.notes(), "a界Xz");
    state.apply(A::MoveLeft {
        extend: true,
        word: false,
    })?;
    state.apply(A::Delete)?;
    assert_eq!(state.notes(), "a界z");
    state.apply(A::Undo)?;
    assert_eq!(state.notes(), "a界Xz");
    state.apply(A::SelectAll)?;
    state.apply(A::InsertText("new".to_owned()))?;
    assert_eq!(state.notes(), "new");
    assert_eq!(state.active_text_cursor(), 3);
    Ok(())
}

#[test]
fn replacing_full_notes_uses_resulting_size() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.apply(A::FocusNotes)?;
    state.apply(A::InsertText(
        "a".repeat(polaris_ui_model::questionnaire::MAX_FIELD_BYTES),
    ))?;
    state.apply(A::SelectAll)?;
    state.apply(A::InsertText("界".to_owned()))?;
    assert_eq!(state.notes(), "界");
    Ok(())
}

#[test]
fn undo_cannot_restore_over_aggregate_budget() -> Result<(), Box<dyn std::error::Error>> {
    let questions = (0..8)
        .map(|i| QuestionSpec {
            id: format!("q{i}"),
            prompt: "Text".to_owned(),
            required: false,
            kind: QuestionKind::FreeText,
        })
        .collect();
    let mut state = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "budget".to_owned(),
        revision: 1,
        requester_label: "demo".to_owned(),
        questions,
    })?;
    state.apply(A::InsertText("界".repeat(2000)))?;
    state.apply(A::SelectAll)?;
    state.apply(A::Delete)?;
    for _ in 1..8 {
        state.navigate(1);
        state.apply(A::InsertText("界".repeat(3000)))?;
    }
    state.navigate(-7);
    let before = state.clone();
    assert!(
        state.apply(A::Undo).is_err(),
        "undo must respect the shared encoded-size cap"
    );
    assert_eq!(state, before, "rejected undo must retain draft and history");
    Ok(())
}
