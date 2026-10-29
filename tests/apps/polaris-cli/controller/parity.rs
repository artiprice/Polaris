//! User-reported dist parity regressions through the actual app controller.
use super::*;
use std::time::Duration;

#[test]
fn demo_commands_open_views_without_starting_response() -> Result<(), Box<dyn std::error::Error>> {
    for command in [
        "/model",
        "/usage",
        "/stats",
        "/providers",
        "/sessions",
        "/diff",
        "/onboarding",
        "/agents",
        "/preview",
    ] {
        let mut state = state()?;
        state.switch_profile(true)?;
        state.composer.insert(command)?;
        let (tx, mut rx) = mpsc::channel(4);
        let _quit = submit_draft(&mut state, &tx)?;
        assert!(
            !state.demo.busy(),
            "{command} was swallowed as an ordinary demo prompt"
        );
        assert!(
            state.composer.is_empty(),
            "{command} did not open its local view"
        );
        assert!(
            rx.try_recv().is_err(),
            "demo command sent a real backend request"
        );
    }
    Ok(())
}

#[test]
fn unknown_demo_slash_is_not_a_simulated_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.composer.insert("/not-a-command")?;
    let (tx, _rx) = mpsc::channel(4);
    let _quit = submit_draft(&mut state, &tx)?;
    assert!(
        !state.demo.busy(),
        "unknown commands must not start a simulation"
    );
    assert_eq!(state.composer.text(), "/not-a-command");
    assert!(state.notice.contains("Unknown"));
    Ok(())
}

#[test]
fn ordinary_demo_prompt_does_not_force_questions() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.demo.submit("show a response")?;
    let _changed = state.demo.tick(Duration::from_secs(3))?;
    assert!(
        state.demo.question().is_none(),
        "ordinary responses unexpectedly steal composer focus"
    );
    Ok(())
}

#[test]
fn plain_delete_keys_edit_draft_and_selected_range() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("a界bc")?;
    state.composer.set_cursor(4);
    let (tx, _rx) = mpsc::channel(4);
    let mut area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions::default();
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(state.composer.text(), "abc");
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(state.composer.text(), "ac");
    state.composer.set_selection(0..2);
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        &tx,
        &mut area,
        &options,
    )?;
    assert!(state.composer.is_empty());
    Ok(())
}

fn press(
    state: &mut UiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<(), crate::CliError> {
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

#[test]
fn question_review_preserves_answers_notes_and_requires_final_submit()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.composer.insert("unsent main draft")?;
    state.demo.start_questionnaire()?;
    super::super::refresh_demo_question(&mut state);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Char(' '), KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE)?;
    for code in [
        KeyCode::Char('a'),
        KeyCode::Char('b'),
        KeyCode::Left,
        KeyCode::Char('界'),
    ] {
        press(&mut state, code, KeyModifiers::NONE)?;
    }
    assert_eq!(state.demo.question().map(|q| q.notes()), Some("a界b"));
    press(&mut state, KeyCode::Backspace, KeyModifiers::NONE)?;
    assert_eq!(state.demo.question().map(|q| q.notes()), Some("ab"));
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Right, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Char(' '), KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(state.question_review);
    assert!(state.demo.question().is_some(), "review must not submit");
    let viewport = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), viewport);
    assert_eq!(
        metadata.composer_count, 0,
        "questions replace the main textarea"
    );
    assert!(metadata.questionnaire_rect.is_some());
    capture_actual(&mut state, "questions-review")?;
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(state.demo.question().is_none());
    assert_eq!(state.composer.text(), "unsent main draft");
    Ok(())
}

#[test]
fn cross_entry_selection_keeps_direction_and_semantic_newlines()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.view_mut().push_assistant("alpha 界")?;
    state.view_mut().push_assistant("e\u{301} omega")?;
    let viewport = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), viewport);
    let first = metadata
        .source_cells
        .iter()
        .find(|s| s.entry_index == 0 && s.source_bytes.start == 6)
        .ok_or("first source")?;
    let second = metadata
        .source_cells
        .iter()
        .find(|s| s.entry_index == 1 && s.source_bytes.start == 0)
        .ok_or("second source")?;
    let points = [
        (
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            (second.rect.x, second.rect.y),
        ),
        (
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            (first.rect.x, first.rect.y),
        ),
    ];
    let (tx, _rx) = mpsc::channel(4);
    let mut area = state.viewport;
    for (kind, (column, row)) in points {
        let _outcome = handle_input(
            &mut state,
            NormalizedInput::Mouse(crossterm::event::MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }),
            &tx,
            &mut area,
            &RenderOptions::default(),
        )?;
    }
    assert_eq!(
        super::super::selected_transcript_text(&state).as_deref(),
        Some("界\n\ne\u{301}")
    );
    assert!(
        state
            .transcript_span
            .is_some_and(|selection| selection.anchor > selection.head)
    );
    Ok(())
}

#[test]
fn all_reference_scenarios_use_the_actual_app_frame_path() -> Result<(), Box<dyn std::error::Error>>
{
    for (name, _description) in crate::demo_gallery::SCENARIOS {
        for (width, height) in [(60, 20), (80, 24), (100, 30), (120, 40), (160, 60)] {
            let mut state = state()?;
            state.switch_profile(true)?;
            state.viewport = Rect::new(0, 0, width, height);
            super::super::load_demo_scenario(&mut state, name)?;
            if *name == "working" {
                let _changed = state.demo.tick(Duration::from_millis(640))?;
            }
            if *name == "questions" {
                assert_eq!(
                    state.gallery.page(),
                    crate::demo_gallery::GalleryPage::Welcome,
                    "question form must not be replaced by a gallery screen"
                );
                assert!(
                    super::super::question_focused(&state),
                    "question scenario must expose the actual form"
                );
            }
            capture_actual(&mut state, &format!("{name}-{width}x{height}"))?;
            assert!(
                state.pending.is_none(),
                "fixture must never dispatch a kernel command"
            );
        }
    }
    Ok(())
}

pub(super) fn capture_actual(
    state: &mut UiState,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        state.viewport.width,
        state.viewport.height,
    ))?;
    terminal.draw(|frame| {
        let _metadata = super::super::paint_ui_frame(
            frame,
            state,
            &RenderOptions {
                colors: ColorMode::TrueColor,
                ascii: false,
            },
        );
    })?;
    let Some(directory) = std::env::var_os("POLARIS_UI_CAPTURE_DIR") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let rgb = |color: ratatui::style::Color| match color {
        ratatui::style::Color::Rgb(r, g, b) => Some([r, g, b]),
        _ => None,
    };
    let cells: Vec<_> = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":rgb(cell.fg),"bg":rgb(cell.bg)}))
        .collect();
    std::fs::write(
        directory.join(format!("{name}.json")),
        serde_json::to_vec(
            &serde_json::json!({"width":state.viewport.width,"height":state.viewport.height,"cells":cells}),
        )?,
    )?;
    Ok(())
}

#[test]
fn gallery_paste_cannot_edit_the_hidden_main_draft() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.composer.insert("keep main draft")?;
    state.gallery.open(crate::demo_gallery::GalleryPage::Usage);
    let (tx, _rx) = mpsc::channel(4);
    let mut area = state.viewport;
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Paste("private paste".to_owned()),
        &tx,
        &mut area,
        &RenderOptions::default(),
    )?;
    assert_eq!(state.composer.text(), "keep main draft");
    Ok(())
}

#[test]
fn model_gallery_is_anchored_above_global_footer() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.viewport = Rect::new(0, 0, 120, 40);
    state.gallery.open(crate::demo_gallery::GalleryPage::Model);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40))?;
    terminal.draw(|frame| {
        let _metadata = super::super::paint_ui_frame(frame, &mut state, &RenderOptions::default());
    })?;
    let mut title_row = None;
    for y in 0..40 {
        let line = (0..120)
            .filter_map(|x| terminal.backend().buffer().cell((x, y)))
            .map(|cell| cell.symbol())
            .collect::<String>();
        if line.contains("Choose a model") {
            title_row = Some(y);
        }
    }
    assert_eq!(
        title_row,
        Some(24),
        "reference model sheet uses 14 rows above the two global footer rows"
    );
    Ok(())
}

#[test]
fn free_text_question_click_and_vertical_edit_use_visible_geometry()
-> Result<(), Box<dyn std::error::Error>> {
    use polaris_ui_model::questionnaire::QuestionnaireAction as A;
    let mut state = state()?;
    state.switch_profile(true)?;
    state.demo.start_questionnaire()?;
    let question = state.demo.question_mut().ok_or("missing question")?;
    assert!(question.select_question(2));
    question.apply(A::InsertText("alpha\nβ界gamma".to_owned()))?;
    let area = state.viewport;
    let metadata = super::super::current_metadata(&mut state, &RenderOptions::default(), area);
    let field = metadata
        .questionnaire_editor
        .ok_or("free text editor geometry missing")?;
    let (tx, _rx) = mpsc::channel(4);
    let mut viewport = area;
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: field.rect.x.saturating_add(1),
            row: field.rect.y,
            modifiers: KeyModifiers::NONE,
        }),
        &tx,
        &mut viewport,
        &RenderOptions::default(),
    )?;
    assert_eq!(
        state.demo.question().map(|q| q.active_text_cursor()),
        Some(1)
    );
    press(&mut state, KeyCode::Backspace, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Delete, KeyModifiers::NONE)?;
    assert_eq!(
        state.demo.question().map(|q| q.answer_text()),
        Some("lpha\n界gamma")
    );
    capture_actual(&mut state, "questions-free-text")?;
    Ok(())
}
