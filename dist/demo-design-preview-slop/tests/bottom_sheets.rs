use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::{App, Entry, EntryKind, Screen},
    models::NVIDIA_MODELS,
    theme::ThemeMode,
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};
fn capture(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn commands_open_bottom_sheets_over_the_existing_conversation() {
    for command in [
        "/model",
        "/status",
        "/stats",
        "/usage",
        "/settings",
        "/help",
        "/providers",
        "/sessions",
        "/diff",
        "/preview",
    ] {
        let mut app = App::default();
        app.input.set("draft survives the popup");
        app.entries.push(Entry::new(
            EntryKind::Note,
            "Conversation context",
            &["Keep this conversation visible above the sheet."],
        ));
        let before = capture(&app, 120, 40);
        app.command(command);
        let sheet = ui::popup_area(&app, Rect::new(0, 0, 120, 40));
        if command == "/usage" {
            assert_eq!(sheet.y, 8, "Usage reserves its approved 30 rows");
        } else if command == "/stats" {
            assert_eq!(sheet.y, 6, "Stats reserves its approved 32 rows");
        } else if command == "/settings" {
            assert_eq!(sheet.y, 7, "Settings reserves the protected header rows");
        } else {
            assert!(sheet.y >= 10, "{command} must leave room for conversation");
        }
        assert_eq!(sheet.bottom(), 38, "global footer belongs below the popup");
        let after = capture(&app, 120, 40);
        // Opening a popup preserves the app bar, metadata and visible conversation.
        for y in 0..sheet.y {
            for x in 0..120 {
                assert_eq!(before[(x, y)], after[(x, y)]);
            }
        }
        assert_eq!(
            after[(
                ui::content_column(ratatui::layout::Rect::new(0, 0, 120, 40)).x + 2,
                2
            )]
                .symbol(),
            "✦"
        );
        for y in 38..40 {
            for x in 0..120 {
                assert_eq!(before[(x, y)], after[(x, y)]);
            }
        }
        let text = ui::dump(&app, 120, 40).unwrap();
        if command != "/settings" {
            assert!(text.contains("Keep this conversation visible"));
        }
        assert!(text.contains(app.screen.title()));
        assert!(!text.contains("UI PREVIEW"));
        app.key(key(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.input.text, "draft survives the popup");
    }
}

#[test]
fn nvidia_models_are_visible_before_connection_and_provider_control_is_below_them() {
    let mut app = App::default();
    assert!(!app.provider().configured);
    app.command("/model");
    let text = ui::dump(&app, 100, 35).unwrap();
    for model in NVIDIA_MODELS {
        assert!(text.contains(model.label));
    }
    let model_end = text.find("Nemotron Nano 8B v1").unwrap();
    assert!(text.find("Provider: NVIDIA NIM").unwrap() > model_end);
    assert!(!text.contains("Not connected"));
    assert!(!text.contains("reference/configured"));
    assert!(!text.contains("connection not verified"));
    assert!(!text.contains("< change >"));
    assert!(text.contains("Provider: NVIDIA NIM · Change"));
    assert!(!text.contains("Default ("));
    assert_eq!(text.matches('✓').count(), 1);
    app.key(key(KeyCode::Tab));
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::ProviderPicker);
    app.providers[1].models.push("user-configured-model".into());
    app.selected = 1;
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Models);
    assert_eq!(app.model_provider, 1);
    assert!(
        ui::dump(&app, 100, 35)
            .unwrap()
            .contains("user-configured-model")
    );
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.active_provider, 1);
    assert_eq!(app.active_model.as_deref(), Some("user-configured-model"));
    assert!(
        !app.provider().configured,
        "choosing a model must not imply a verified connection"
    );
}

#[test]
fn shift_enter_and_alt_enter_insert_newlines_without_submitting() {
    for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
        let mut app = App::default();
        app.input.set("first paragraph");
        app.key(KeyEvent::new(KeyCode::Enter, modifier));
        assert_eq!(app.input.text, "first paragraph\n");
        assert!(!app.working);
        assert!(app.entries.is_empty());
        app.paste("second paragraph");
        app.key(key(KeyCode::Enter));
        assert!(app.working);
        assert_eq!(
            app.entries[0].lines,
            ["first paragraph", "second paragraph"]
        );
    }
}

#[test]
fn header_footer_modes_and_default_background_match_the_requested_style() {
    let mut app = App::default();
    app.account_email = Some("display-only@example.invalid".into());
    let text = ui::dump(&app, 100, 35).unwrap();
    assert!(text.starts_with("\n\n"));
    assert!(text.contains("Polaris v0.1.0alpha"));
    assert!(text.contains("display-only@example.invalid"));
    assert!(!text.contains("UI PREVIEW"));
    let rows = text.lines().collect::<Vec<_>>();
    assert!(rows[31].contains("> What's on your mind"));
    assert!(rows[33].is_empty());
    assert!(rows[34].contains("[ auto mode on ]"));
    assert!(rows[34].ends_with("[ high ]"));
    assert!(!text.contains("Shift+Tab mode"));
    let auto = capture(&app, 100, 35);
    let col = ui::content_column(auto.area);
    assert!(!auto[(col.x + 4, 2)].modifier.contains(Modifier::BOLD));
    assert_eq!(auto[(col.x + 12, 2)].fg, Color::Rgb(133, 133, 133));
    assert!(!auto[(col.x + 12, 2)].modifier.contains(Modifier::BOLD));
    assert!(
        auto.content
            .iter()
            .all(|c| c.bg == Color::Rgb(0, 0, 0) && !c.modifier.contains(Modifier::REVERSED))
    );
    let mode_x = rows[34].find("auto mode on").unwrap() as u16;
    assert_eq!(auto[(mode_x, 34)].fg, Color::Rgb(245, 194, 231));
    app.key(key(KeyCode::BackTab));
    let plan = capture(&app, 100, 35);
    assert_eq!(plan[(mode_x, 34)].fg, Color::Rgb(249, 226, 175));
    assert!(
        ui::dump(&app, 100, 35)
            .unwrap()
            .contains("[ plan mode on ]")
    );
    app.key(key(KeyCode::F(2)));
    assert_eq!(app.screen, Screen::Status);
    assert!(
        ui::dump(&app, 100, 35)
            .unwrap()
            .contains("Provider is not configured")
    );
    app.theme.mode = ThemeMode::Terminal;
    assert_eq!(app.theme.base().bg, None);
}

#[test]
fn provider_form_remains_reachable_in_the_smallest_bottom_sheet() {
    let mut app = App::default();
    app.start_provider_edit(0);
    for _ in 0..4 {
        app.key(key(KeyCode::Tab));
    }
    assert_eq!(app.selected, 4);
    assert!(
        ui::dump(&app, 60, 20)
            .unwrap()
            .contains("Save for this session only")
    );
}

#[test]
fn compact_footer_and_model_picker_remain_clean_at_all_supported_sizes() {
    for (width, height) in [(120, 40), (80, 24), (60, 20)] {
        let mut app = App::default();
        app.command("/model");
        let text = ui::dump(&app, width, height).unwrap();
        let rows: Vec<_> = text.lines().collect();
        let title = rows
            .iter()
            .position(|row| row.contains("Choose a model"))
            .unwrap();
        assert!(rows[title + 1].trim().trim_matches('│').trim().is_empty());
        assert!(rows[title + 2].contains("Nemotron Super"));
        assert!(text.contains("Nemotron Nano 8B v1"));
        assert!(text.contains("Provider: NVIDIA NIM · Change"));
        assert!(text.contains("Reasoning and"));
        assert!(text.contains("Effort"));
        assert!(text.contains("[ high ]"));
        assert!(
            text.contains("low")
                && text.contains("medium")
                && text.contains("xhigh")
                && text.contains("max")
        );
        assert!(!text.contains("connection"));
        assert!(!text.contains("warning"));
        let footer = rows[height as usize - 1];
        assert!(footer.chars().count() <= width as usize - 2);
        assert!(footer.ends_with("[ high ]"));
        assert!(footer.contains("Nemotron Super"));
        let buffer = capture(&app, width, height);
        let provider_y = rows
            .iter()
            .position(|row| row.contains("Provider:"))
            .unwrap();
        assert_eq!(buffer[(6, provider_y as u16)].fg, Color::Rgb(232, 232, 232));
        app.key(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Home);
        assert!(
            app.notice.is_empty(),
            "model selection should leave the composer clean"
        );
    }
}

#[test]
fn line_feed_and_control_j_fallbacks_insert_newlines_and_shortcuts_are_real() {
    for key in [
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
    ] {
        let mut app = App::default();
        app.input.set("first");
        app.key(key);
        assert_eq!(app.input.text, "first\n");
        assert!(!app.working);
    }
    let mut app = App::default();
    app.key(key(KeyCode::Char('?')));
    assert_eq!(app.screen, Screen::Help);
    app.key(key(KeyCode::Esc));
    app.input.set("question");
    app.key(key(KeyCode::Char('?')));
    assert_eq!(app.input.text, "question?");
}

#[test]
fn effort_is_adjustable_bounded_and_remembered_per_model() {
    let mut app = App::default();
    app.command("/model");
    assert_eq!(app.picker_effort(), 2);
    for _ in 0..5 {
        app.key(key(KeyCode::Left));
    }
    assert_eq!(app.picker_effort(), 0);
    assert!(ui::dump(&app, 80, 24).unwrap().contains("[ low ]"));
    for _ in 0..5 {
        app.key(key(KeyCode::Right));
    }
    assert_eq!(app.picker_effort(), 4);
    app.key(key(KeyCode::Enter));
    assert_eq!(app.effort_label(), "max");
    app.command("/model");
    app.key(key(KeyCode::Down));
    assert_eq!(
        app.picker_effort(),
        2,
        "new model starts with high preference"
    );
    app.key(key(KeyCode::Left));
    app.key(key(KeyCode::Enter));
    assert_eq!(app.effort_label(), "medium");
    app.command("/model");
    app.key(key(KeyCode::Up));
    assert_eq!(app.picker_effort(), 4, "previous model remembers max");
    app.key(key(KeyCode::Esc));
    assert_eq!(
        app.effort_label(),
        "medium",
        "cancel must not apply a different model's effort"
    );
}

#[test]
fn provider_form_separates_each_label_value_group_with_a_blank_row() {
    let mut app = App::default();
    app.start_provider_edit(0);
    let text = ui::dump(&app, 100, 35).unwrap();
    let rows: Vec<_> = text.lines().collect();
    let names = [
        "Display label",
        "Endpoint (optional)",
        "Credential reference (env name only)",
        "Model label or identifier",
    ];
    let mut previous = None;
    for name in names {
        let y = rows.iter().position(|row| row.contains(name)).unwrap();
        assert!(
            rows[y + 2].trim().trim_matches('│').trim().is_empty(),
            "blank row after {name}"
        );
        if let Some(prev) = previous {
            assert_eq!(y - prev, 3);
        }
        previous = Some(y);
    }
    assert!(!text.contains("never an API key"));
    assert!(text.contains("env name only"));
}

#[test]
fn composer_has_exactly_one_blank_row_before_footer_for_single_and_multiline_drafts() {
    for (w, h) in [(120, 40), (80, 24), (60, 20)] {
        for draft in [
            "",
            "hello",
            "first\nsecond",
            "one\ntwo\nthree\nfour\nfive\nsix\nseven",
        ] {
            let mut app = App::default();
            app.input.set(draft);
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|frame| ui::render(frame, &app)).unwrap();
            assert_eq!(terminal.get_cursor_position().unwrap().y, h - 4);
            let text = ui::dump(&app, w, h).unwrap();
            let rows: Vec<_> = text.lines().collect();
            assert!(rows[h as usize - 2].is_empty(), "exactly one spacer row");
            assert!(rows.last().unwrap().contains("[ auto mode on ]"));
        }
    }
}
