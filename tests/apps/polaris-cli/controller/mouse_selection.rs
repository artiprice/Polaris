//! Actual controller gestures for source-only selection.
use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use polaris_ui_cli::screen::{FooterAction, HitTarget};
use polaris_ui_model::questionnaire::QuestionnaireAction as Q;

fn mouse(state: &mut UiState, kind: MouseEventKind, x: u16, y: u16) -> Result<(), crate::CliError> {
    let (tx, _rx) = mpsc::channel(4);
    let mut area = state.viewport;
    let _outcome = handle_input(
        state,
        NormalizedInput::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
        &tx,
        &mut area,
        &RenderOptions::default(),
    )?;
    Ok(())
}
fn click(state: &mut UiState, x: u16, y: u16) -> Result<(), crate::CliError> {
    mouse(state, MouseEventKind::Down(MouseButton::Left), x, y)?;
    mouse(state, MouseEventKind::Up(MouseButton::Left), x, y)
}
fn key(state: &mut UiState, code: KeyCode, modifiers: KeyModifiers) -> Result<(), crate::CliError> {
    let (tx, _rx) = mpsc::channel(4);
    let mut area = state.viewport;
    let _outcome = handle_input(
        state,
        NormalizedInput::Key(KeyEvent::new(code, modifiers)),
        &tx,
        &mut area,
        &RenderOptions::default(),
    )?;
    Ok(())
}
fn editor_origin(state: &mut UiState) -> (u16, u16) {
    let area = state.viewport;
    let metadata = super::super::current_metadata(state, &RenderOptions::default(), area);
    (
        metadata.composer_rect.x.saturating_add(4),
        metadata.composer_rect.y.saturating_add(1),
    )
}
#[test]
fn triple_click_selects_the_logical_editor_line() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("alpha beta\nsecond")?;
    let (x, y) = editor_origin(&mut state);
    for _ in 0..3 {
        click(&mut state, x.saturating_add(2), y)?;
    }
    assert_eq!(
        state.composer.selected_text().as_deref(),
        Some("alpha beta\n")
    );
    Ok(())
}
#[test]
fn double_click_drag_extends_by_words() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("alpha beta gamma")?;
    let (x, y) = editor_origin(&mut state);
    click(&mut state, x.saturating_add(1), y)?;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        x.saturating_add(1),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        x.saturating_add(8),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        x.saturating_add(8),
        y,
    )?;
    assert_eq!(
        state.composer.selected_text().as_deref(),
        Some("alpha beta")
    );
    super::parity::capture_actual(&mut state, "word-drag")?;
    Ok(())
}
#[test]
fn mouse_selection_survives_release_and_delete() -> Result<(), Box<dyn std::error::Error>> {
    for delete in [KeyCode::Delete, KeyCode::Backspace] {
        let mut state = state()?;
        state.composer.insert("abcde")?;
        let (x, y) = editor_origin(&mut state);
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            x.saturating_add(1),
            y,
        )?;
        mouse(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            x.saturating_add(4),
            y,
        )?;
        mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            x.saturating_add(4),
            y,
        )?;
        assert_eq!(state.composer.selected_text().as_deref(), Some("bcd"));
        key(&mut state, delete, KeyModifiers::NONE)?;
        assert_eq!(state.composer.text(), "ae");
        key(&mut state, KeyCode::Char('z'), KeyModifiers::CONTROL)?;
        assert_eq!(state.composer.text(), "abcde");
        assert_eq!(state.composer.selected_text().as_deref(), Some("bcd"));
    }
    Ok(())
}
#[test]
fn blank_transcript_click_clears_selection() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.view_mut().push_assistant("alpha beta")?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let source = metadata.source_cells.first().ok_or("source")?;
    let (x, y) = (source.rect.x, source.rect.y);
    mouse(&mut state, MouseEventKind::Down(MouseButton::Left), x, y)?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        x.saturating_add(3),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        x.saturating_add(3),
        y,
    )?;
    assert!(super::super::selected_transcript_text(&state).is_some());
    click(&mut state, x.saturating_add(25), y)?;
    assert!(
        super::super::selected_transcript_text(&state).is_none(),
        "padding click must not retain a stale range"
    );
    Ok(())
}
#[test]
fn editor_focus_clears_transcript_selection() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.view_mut().push_assistant("alpha beta")?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let source = metadata.source_cells.first().ok_or("source")?;
    let (x, y) = (source.rect.x, source.rect.y);
    mouse(&mut state, MouseEventKind::Down(MouseButton::Left), x, y)?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        x.saturating_add(3),
        y,
    )?;
    state.composer.insert("draft")?;
    let (cx, cy) = editor_origin(&mut state);
    click(&mut state, cx, cy)?;
    assert!(
        super::super::selected_transcript_text(&state).is_none(),
        "editor focus must clear transcript span"
    );
    Ok(())
}
#[test]
fn mouse_selects_demo_from_profile_sheet() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let hit = metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Footer(FooterAction::Profile))
        .ok_or("profile hit")?;
    click(&mut state, hit.rect.x, hit.rect.y)?;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let hit = metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Sheet)
        .ok_or("profile sheet")?;
    click(
        &mut state,
        hit.rect.x.saturating_add(3),
        hit.rect.y.saturating_add(4),
    )?;
    assert!(state.demo_active);
    Ok(())
}
#[test]
fn question_notes_drag_selects_and_deletes() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.demo.start_questionnaire()?;
    let q = state.demo.question_mut().ok_or("question")?;
    q.apply(Q::FocusNotes)?;
    q.apply(Q::InsertText("alpha beta".to_owned()))?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let field = metadata.questionnaire_editor.ok_or("notes geometry")?;
    let (x, y) = (field.rect.x, field.rect.y);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        x.saturating_add(1),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        x.saturating_add(5),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        x.saturating_add(5),
        y,
    )?;
    key(&mut state, KeyCode::Delete, KeyModifiers::NONE)?;
    assert_eq!(state.demo.question().map(|q| q.notes()), Some("a beta"));
    Ok(())
}

#[test]
fn transcript_word_drag_reverses_by_whole_words() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.view_mut().push_assistant("alpha beta gamma")?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let first = metadata
        .source_cells
        .iter()
        .find(|s| s.source_bytes.start == 1)
        .ok_or("alpha")?;
    let second = metadata
        .source_cells
        .iter()
        .find(|s| s.source_bytes.start == 7)
        .ok_or("beta")?;
    let target = (first.rect.x, first.rect.y);
    let origin = (second.rect.x, second.rect.y);
    click(&mut state, origin.0, origin.1)?;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        origin.0,
        origin.1,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        target.0,
        target.1,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        target.0,
        target.1,
    )?;
    assert_eq!(
        super::super::selected_transcript_text(&state).as_deref(),
        Some("alpha beta")
    );
    Ok(())
}
#[test]
fn transcript_triple_click_selects_source_line_across_soft_wrap()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let line = format!("{}tail\n", "alpha beta ".repeat(12));
    state.view_mut().push_assistant(format!("{line}second"))?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let source = metadata
        .source_cells
        .iter()
        .find(|s| s.source_bytes.start == 6)
        .ok_or("source")?;
    let (x, y) = (source.rect.x, source.rect.y);
    for _ in 0..3 {
        click(&mut state, x, y)?;
    }
    assert_eq!(
        super::super::selected_transcript_text(&state).as_deref(),
        Some(line.as_str())
    );
    super::parity::capture_actual(&mut state, "transcript-line")?;
    Ok(())
}
#[test]
fn double_click_editor_padding_keeps_no_selection() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("alpha")?;
    let (x, y) = editor_origin(&mut state);
    click(&mut state, x.saturating_add(25), y)?;
    click(&mut state, x.saturating_add(25), y)?;
    assert!(state.composer.selection().is_none());
    assert_eq!(state.composer.cursor(), 5);
    key(&mut state, KeyCode::Delete, KeyModifiers::NONE)?;
    assert_eq!(state.composer.text(), "alpha");
    Ok(())
}
#[test]
fn reverse_unicode_drag_delete_and_late_copy_keep_the_exact_range()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("A界e\u{301} Z")?;
    let (x, y) = editor_origin(&mut state);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        x.saturating_add(4),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        x.saturating_add(1),
        y,
    )?;
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        x.saturating_add(1),
        y,
    )?;
    assert_eq!(
        state.composer.selected_text().as_deref(),
        Some("界e\u{301}")
    );
    super::parity::capture_actual(&mut state, "unicode-selection")?;
    let generation = state.clipboard_generation;
    assert!(super::super::receive_clipboard_result(
        &mut state,
        crate::ui_services::ClipboardResult::TextCopied {
            generation,
            result: Ok(())
        }
    ));
    assert_eq!(
        state.composer.selected_text().as_deref(),
        Some("界e\u{301}")
    );
    key(&mut state, KeyCode::Delete, KeyModifiers::NONE)?;
    assert_eq!(state.composer.text(), "A Z");
    key(&mut state, KeyCode::Char('z'), KeyModifiers::CONTROL)?;
    assert_eq!(state.composer.text(), "A界e\u{301} Z");
    assert_eq!(
        state.composer.selected_text().as_deref(),
        Some("界e\u{301}")
    );
    Ok(())
}
#[test]
fn selected_word_is_replaced_by_typing_and_paste() -> Result<(), Box<dyn std::error::Error>> {
    for paste in [false, true] {
        let mut state = state()?;
        state.composer.insert("alpha beta")?;
        let (x, y) = editor_origin(&mut state);
        click(&mut state, x.saturating_add(2), y)?;
        click(&mut state, x.saturating_add(2), y)?;
        if paste {
            let (tx, _rx) = mpsc::channel(4);
            let mut area = state.viewport;
            let _outcome = handle_input(
                &mut state,
                NormalizedInput::Paste("X".to_owned()),
                &tx,
                &mut area,
                &RenderOptions::default(),
            )?;
        } else {
            key(&mut state, KeyCode::Char('X'), KeyModifiers::NONE)?;
        }
        assert_eq!(state.composer.text(), "X beta");
    }
    Ok(())
}
#[test]
fn notes_double_and_triple_click_keep_the_main_draft_intact()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.composer.insert("hidden main draft")?;
    state.demo.start_questionnaire()?;
    let q = state.demo.question_mut().ok_or("question")?;
    q.apply(Q::FocusNotes)?;
    q.apply(Q::InsertText("alpha beta\nsecond".to_owned()))?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let field = metadata.questionnaire_editor.ok_or("notes geometry")?;
    let (x, y) = (field.rect.x.saturating_add(2), field.rect.y);
    click(&mut state, x, y)?;
    click(&mut state, x, y)?;
    assert_eq!(
        state
            .demo
            .question()
            .and_then(|q| q.active_editor())
            .and_then(|e| e.selected_text())
            .as_deref(),
        Some("alpha")
    );
    click(&mut state, x, y)?;
    assert_eq!(
        state
            .demo
            .question()
            .and_then(|q| q.active_editor())
            .and_then(|e| e.selected_text())
            .as_deref(),
        Some("alpha beta\n")
    );
    super::parity::capture_actual(&mut state, "notes-line")?;
    key(&mut state, KeyCode::Backspace, KeyModifiers::NONE)?;
    assert_eq!(state.demo.question().map(|q| q.notes()), Some("second"));
    assert_eq!(state.composer.text(), "hidden main draft");
    Ok(())
}
