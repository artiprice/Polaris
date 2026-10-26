use super::*;

#[test]
fn arriving_demo_questions_do_not_overwrite_an_active_picker()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.demo.submit("questions")?;
    super::super::open_profile_picker(&mut state);
    let _changed = state.demo.tick(std::time::Duration::from_secs(2))?;
    super::super::refresh_demo_question(&mut state);
    assert_eq!(
        state.sheet.as_ref().map(|sheet| sheet.title.as_str()),
        Some("Profile")
    );
    press(&mut state, KeyCode::Esc, KeyModifiers::NONE)?;
    assert!(
        state.demo.question().is_some(),
        "closing the picker must preserve the pending demo question"
    );
    super::super::refresh_demo_question(&mut state);
    assert!(state.sheet.is_none());
    assert!(super::super::question_focused(&state));
    Ok(())
}

#[test]
fn effort_picker_applies_explicitly_and_keeps_the_draft_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("draft")?;
    state.composer.set_cursor(2);
    let effort = state.view().effort;
    super::super::open_effort_picker(&mut state);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    assert_eq!(
        state.view().effort,
        effort,
        "preview cannot apply effort before Enter"
    );
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert_ne!(state.view().effort, effort);
    assert_eq!(state.composer.cursor(), 2);
    assert!(state.sheet.is_none());
    Ok(())
}

#[test]
fn pasted_text_preview_expands_the_exact_chip_payload() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let payload = "界 e\u{301} text\n".repeat(100);
    state.composer.insert(&payload)?;
    let id = state
        .composer
        .chips()
        .first()
        .ok_or("paste chip missing")?
        .id;
    super::super::preview_chip(&mut state, id);
    assert!(state.sheet.is_some());
    press(&mut state, KeyCode::Char('e'), KeyModifiers::NONE)?;
    assert_eq!(state.composer.text(), payload);
    assert!(state.composer.chips().is_empty());
    assert!(state.sheet.is_none());
    Ok(())
}

fn capture_visual(state: &UiState, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let Some(directory) = std::env::var_os("POLARIS_UI_CAPTURE_DIR") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let lines = state.panel_lines();
    let footer = super::super::footer_text(state);
    let dump = polaris_ui_cli::screen::dump_cells(
        &super::super::projection(state, &lines, &footer),
        &RenderOptions {
            colors: ColorMode::TrueColor,
            ascii: false,
        },
        100,
        30,
    )?;
    let cells: Vec<_> = dump.cells.iter().map(|cell| serde_json::json!({ "symbol": cell.symbol, "fg": cell.fg_rgb, "bg": cell.bg_rgb })).collect();
    std::fs::write(
        directory.join(format!("{name}.json")),
        serde_json::to_vec(&serde_json::json!({"width":100,"height":30,"cells":cells}))?,
    )?;
    Ok(())
}

#[test]
fn continuation_visual_scenarios_use_the_interactive_projection()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    assert_eq!(
        state.view().model.to_string(),
        "nvidia/llama-3.3-nemotron-super-49b-v1.5"
    );
    capture_visual(&state, "demo-welcome")?;
    state
        .composer
        .insert("review a safe UI change questionnaire")?;
    let (tx, _rx) = mpsc::channel(4);
    let _quit = submit_draft(&mut state, &tx)?;
    let _changed = state.demo.tick(std::time::Duration::from_secs(2))?;
    super::super::refresh_demo_question(&mut state);
    assert!(super::super::question_focused(&state));
    capture_visual(&state, "demo-questions")?;
    state.demo.cancel()?;
    state.sheet = None;
    state
        .composer
        .insert("one long row with Unicode 界 and e\u{301}\nsecond line\nthird line")?;
    state.composer.set_selection(4..17);
    assert!(state.composer.selection().is_some());
    capture_visual(&state, "textarea-selection")?;
    state
        .composer
        .apply_completion(0..state.composer.text().len(), "")?;
    let directory = tempfile::tempdir()?;
    state.settings = Some(crate::cli_settings::CliSettings::from_snapshot(
        polaris_config::Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        },
        polaris_config::CliPreferences::default(),
        crate::ui_services::PreferenceOverrides::default(),
        crate::cli_settings::PersistencePolicy::MemoryOnly,
    ));
    super::super::open_settings(&mut state);
    capture_visual(&state, "demo-settings")?;
    assert!(!directory.path().join("config.toml").exists());
    Ok(())
}

#[test]
fn interactive_projection_keeps_dollar_in_prefill_and_literal_paste()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("$websearch(on)")?;
    state.refresh_completion();
    let lines = state.panel_lines();
    let footer = super::super::footer_text(&state);
    let text = polaris_ui_cli::screen::dump_text(
        &super::super::projection(&state, &lines, &footer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    assert!(
        text.contains("$websearch(on)"),
        "interactive projection lost sigil: {text}"
    );
    Ok(())
}

#[test]
fn late_clipboard_read_cannot_paste_into_a_changed_draft_or_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("draft")?;
    state.clipboard_generation = 3;
    state.paste_target = Some(super::super::PasteTarget {
        session: state.id,
        demo: false,
        revision: state.composer.revision(),
    });
    state.composer.move_left();
    assert!(super::super::receive_clipboard_result(
        &mut state,
        crate::ui_services::ClipboardResult::TextRead {
            generation: 3,
            result: Ok("wrong target".to_owned())
        }
    ));
    assert_eq!(state.composer.text(), "draft");
    assert!(state.notice.contains("cancelled"));
    state.paste_target = Some(super::super::PasteTarget {
        session: state.id,
        demo: false,
        revision: state.composer.revision(),
    });
    assert!(super::super::receive_clipboard_result(
        &mut state,
        crate::ui_services::ClipboardResult::TextRead {
            generation: 3,
            result: Ok("$literal\n/quit".to_owned())
        }
    ));
    assert_eq!(state.composer.text(), "draf$literal\n/quitt");
    assert!(!super::super::receive_clipboard_result(
        &mut state,
        crate::ui_services::ClipboardResult::TextRead {
            generation: 2,
            result: Ok("stale".to_owned())
        }
    ));
    assert_eq!(state.composer.text(), "draf$literal\n/quitt");
    Ok(())
}

#[test]
fn profiles_keep_drafts_separate_and_demo_never_dispatches_kernel_controls()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("offline draft")?;
    state.switch_profile(true)?;
    assert!(state.composer.is_empty());
    state.composer.insert("$websearch(on)")?;
    let (tx, mut rx) = mpsc::channel(4);
    assert!(!submit_draft(&mut state, &tx)?);
    assert!(state.demo.busy());
    assert!(
        rx.try_recv().is_err(),
        "demo must not dispatch signed directives"
    );
    assert!(state.pending.is_none());
    state.composer.insert("demo draft")?;
    state.switch_profile(false)?;
    assert_eq!(state.composer.text(), "offline draft");
    state.switch_profile(true)?;
    assert_eq!(state.composer.text(), "demo draft");
    assert_eq!(
        state.view().model.to_string(),
        "nvidia/llama-3.3-nemotron-super-49b-v1.5"
    );
    Ok(())
}

#[test]
fn settings_enter_persists_one_field_off_thread_and_demo_cannot_save()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::cli_settings::{CliSettings, PersistencePolicy, SettingsService};
    use crate::ui_services::PreferenceOverrides;
    let temp = tempfile::tempdir()?;
    let dirs = polaris_config::Dirs {
        global: temp.path().to_path_buf(),
        repo: None,
    };
    let mut state = state()?;
    state.settings = Some(CliSettings::from_snapshot(
        dirs,
        polaris_config::CliPreferences::default(),
        PreferenceOverrides::default(),
        PersistencePolicy::Global,
    ));
    state.settings_worker = Some(SettingsService::new()?);
    super::super::open_settings(&mut state);
    press(&mut state, KeyCode::Right, KeyModifiers::NONE)?;
    let file = temp.path().join("config.toml");
    assert!(!file.exists(), "preview must not save");
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(state.settings_pending);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while state.settings_pending && std::time::Instant::now() < deadline {
        let _changed = super::super::poll_services(&mut state)?;
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!state.settings_pending, "worker save deadline exceeded");
    let saved = std::fs::read_to_string(&file)?;
    assert!(saved.contains("theme = \"terminal\""));
    state.settings_open = false;
    state.sheet = None;
    state.switch_profile(true)?;
    super::super::open_settings(&mut state);
    press(&mut state, KeyCode::Right, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert_eq!(
        std::fs::read_to_string(&file)?,
        saved,
        "demo must never write preferences"
    );
    assert!(!state.settings_pending);
    Ok(())
}

#[test]
fn demo_question_notes_none_and_atomic_submit_are_connected()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.switch_profile(true)?;
    state.composer.insert("questions")?;
    let (tx, mut rx) = mpsc::channel(4);
    let _quit = submit_draft(&mut state, &tx)?;
    let _changed = state.demo.tick(std::time::Duration::from_secs(2))?;
    super::super::refresh_demo_question(&mut state);
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(
        state.demo.question().is_some(),
        "Enter cannot invent required answers"
    );
    assert!(state.question_review);
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(!state.question_review);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Char(' '), KeyModifiers::NONE)?;
    assert!(
        state
            .demo
            .question()
            .is_some_and(|q| q.none_of_the_above_selected())
    );
    press(&mut state, KeyCode::Tab, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Char('n'), KeyModifiers::NONE)?;
    assert_eq!(state.demo.question().map(|q| q.notes()), Some("n"));
    press(&mut state, KeyCode::BackTab, KeyModifiers::SHIFT)?;
    press(&mut state, KeyCode::Right, KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Char(' '), KeyModifiers::NONE)?;
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(state.question_review);
    assert!(state.demo.question().is_some());
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE)?;
    assert!(
        state.demo.question().is_none(),
        "all valid answers should submit atomically"
    );
    assert!(rx.try_recv().is_err(), "demo answers must remain local");
    Ok(())
}

#[test]
fn transcript_drag_copies_canonical_unicode_bytes_without_repainting_for_hit_test()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.view_mut().push_assistant("A界e\u{301}Z")?;
    let options = RenderOptions::default();
    let mut area = Rect::new(0, 0, 80, 24);
    let metadata = super::super::current_metadata(&mut state, &options, area);
    let first = metadata
        .source_cells
        .iter()
        .find(|cell| cell.source_bytes.start == 1)
        .ok_or("wide source cell missing")?;
    let last = metadata
        .source_cells
        .iter()
        .find(|cell| cell.source_bytes.end == "A界e\u{301}".len())
        .ok_or("combining source cell missing")?;
    let start = (first.rect.x, first.rect.y);
    let end = (last.rect.x, last.rect.y);
    let (tx, _rx) = mpsc::channel(4);
    for (kind, point) in [
        (
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            start,
        ),
        (
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            end,
        ),
    ] {
        let _outcome = handle_input(
            &mut state,
            NormalizedInput::Mouse(crossterm::event::MouseEvent {
                kind,
                column: point.0,
                row: point.1,
                modifiers: KeyModifiers::NONE,
            }),
            &tx,
            &mut area,
            &options,
        )?;
    }
    assert_eq!(
        super::super::selected_transcript_text(&state).as_deref(),
        Some("界e\u{301}")
    );
    assert!(
        state.last_metadata.is_none(),
        "source mapping must not paint to resolve a click"
    );
    Ok(())
}

#[test]
fn pointer_hover_and_click_use_current_completion_geometry()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("/hel")?;
    state.refresh_completion();
    let mut area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions::default();
    let lines = state.panel_lines();
    let footer = super::super::footer_text(&state);
    let metadata = polaris_ui_cli::screen::prepare_metadata(
        &super::super::projection(&state, &lines, &footer),
        &options,
        area,
    );
    let region = metadata
        .hits
        .iter()
        .find(|hit| matches!(hit.target, polaris_ui_cli::screen::HitTarget::Completion(_)))
        .ok_or("completion hit absent")?;
    let target = region.target;
    let point = (region.rect.x, region.rect.y);
    let (tx, _rx) = mpsc::channel(4);
    let pointer = |kind| {
        NormalizedInput::Mouse(crossterm::event::MouseEvent {
            kind,
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::NONE,
        })
    };
    let _outcome = handle_input(
        &mut state,
        pointer(crossterm::event::MouseEventKind::Moved),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(
        state.hover,
        Some(target),
        "mouse movement must reach hover projection"
    );
    assert!(
        state.last_metadata.is_none(),
        "geometry preparation must not present a frame"
    );
    let _outcome = handle_input(
        &mut state,
        pointer(crossterm::event::MouseEventKind::Down(
            crossterm::event::MouseButton::Left,
        )),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(
        state.composer.text(),
        "/help",
        "click should accept the visible suggestion"
    );
    assert!(state.completion.is_empty());
    Ok(())
}

fn press(
    state: &mut UiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<(), crate::CliError> {
    let (tx, _rx) = mpsc::channel(4);
    let mut area = Rect::new(0, 0, 80, 24);
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
fn shift_enter_is_newline_and_control_enter_is_not() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("first")?;
    press(&mut state, KeyCode::Enter, KeyModifiers::SHIFT)?;
    assert_eq!(state.composer.text(), "first\n");
    press(&mut state, KeyCode::Enter, KeyModifiers::CONTROL)?;
    assert_eq!(
        state.composer.text(),
        "first\n",
        "Ctrl+Enter must not insert another newline"
    );
    Ok(())
}

#[test]
fn word_selection_replacement_and_line_home_are_connected() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = state()?;
    state.composer.insert("alpha beta\nsecond line")?;
    press(&mut state, KeyCode::Home, KeyModifiers::NONE)?;
    assert_eq!(
        state.composer.cursor(),
        11,
        "Home goes to the logical line start"
    );
    press(&mut state, KeyCode::Right, KeyModifiers::CONTROL)?;
    press(
        &mut state,
        KeyCode::Right,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    )?;
    assert_eq!(state.composer.selected_text().as_deref(), Some("line"));
    press(&mut state, KeyCode::Char('X'), KeyModifiers::NONE)?;
    assert_eq!(state.composer.text(), "alpha beta\nsecond X");
    press(&mut state, KeyCode::Backspace, KeyModifiers::CONTROL)?;
    assert_eq!(state.composer.text(), "alpha beta\nsecond ");
    Ok(())
}

#[test]
fn vertical_navigation_keeps_column_and_effort_does_not_move_caret()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("abcdef\nx\nabcdef")?;
    state.composer.set_cursor(5);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    assert_eq!(state.composer.cursor(), 8);
    press(&mut state, KeyCode::Down, KeyModifiers::NONE)?;
    assert_eq!(
        state.composer.cursor(),
        14,
        "short middle row must retain desired column five"
    );
    let before = state.composer.cursor();
    let effort = state.view().effort;
    press(&mut state, KeyCode::Up, KeyModifiers::SHIFT)?;
    assert_eq!(state.composer.cursor(), before);
    assert_ne!(state.view().effort, effort);
    Ok(())
}
