use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::{App, Editor, SCENARIOS, Screen},
    theme::ThemeMode,
    ui,
};
use ratatui::style::{Color, Modifier};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn editor_moves_and_deletes_on_unicode_boundaries_and_sanitizes_controls() {
    let mut editor = Editor::default();
    editor.set("a🦀é");
    assert_eq!(editor.cursor, "a🦀é".len());

    editor.left();
    assert_eq!(editor.cursor, "a🦀".len());
    editor.backspace();
    assert_eq!(editor.text, "aé");
    assert_eq!(editor.cursor, 1);
    editor.delete();
    assert_eq!(editor.text, "a");
    assert_eq!(editor.cursor, 1);

    editor.set("one\u{1b}two\r\nthree\tend");
    assert_eq!(editor.text, "onetwo\nthreeend");
    assert!(editor.text.is_char_boundary(editor.cursor));
}

#[test]
fn editor_bounds_long_unicode_insertion_without_splitting_a_codepoint() {
    let mut editor = Editor::default();
    editor.insert(&"界".repeat(20_000));
    assert_eq!(editor.text.len(), 32_766);
    assert!(editor.text.is_char_boundary(editor.cursor));
    assert_eq!(editor.cursor, editor.text.len());

    editor.insert("x");
    assert_eq!(editor.text.len(), 32_767);
    editor.insert("x");
    assert_eq!(editor.text.len(), 32_768);
    editor.backspace();
    assert_eq!(editor.text.len(), 32_767);
    editor.left();
    editor.backspace();
    assert_eq!(editor.text.len(), 32_764);
    assert!(editor.text.is_char_boundary(editor.cursor));
}

#[test]
fn command_menu_filters_and_enters_the_selected_command() {
    let mut app = App::new("C:/workspace/demo".into());
    app.input.set("/s");
    assert!(app.menu_open());
    assert_eq!(
        app.filtered_commands()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        vec!["/settings", "/sessions", "/status", "/stats"]
    );

    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Down));
    assert_eq!(app.selected, 2);
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Status);
    assert!(app.input.text.is_empty());
}

#[test]
fn provider_form_validates_then_saves_only_in_memory_and_model_can_be_selected() {
    let mut app = App::new("C:/workspace/demo".into());
    app.start_provider_edit(0);
    assert_eq!(app.screen, Screen::ProviderEdit);

    app.form[0].clear();
    app.form[3].set("nim-preview-model");
    app.save_provider();
    assert_eq!(app.screen, Screen::ProviderEdit);
    assert!(!app.providers[0].configured);
    assert!(app.notice.contains("label is required"));

    app.form[0].set("NVIDIA NIM Preview");
    app.form[1].set("https://example.invalid/v1");
    app.form[2].set("environment variable (label only)");
    app.save_provider();
    assert_eq!(app.screen, Screen::Home);
    assert!(app.providers[0].configured);
    assert_eq!(app.providers[0].models, vec!["nim-preview-model"]);
    assert_eq!(app.active_model.as_deref(), Some("nim-preview-model"));
    assert!(app.notice.contains("not been validated"));

    app.command("/model");
    assert_eq!(app.screen, Screen::Models);
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.active_provider, 0);
    assert_eq!(app.active_model.as_deref(), Some("nim-preview-model"));
}

#[test]
fn navigation_restores_conversation_draft_and_view_state() {
    let mut app = App::new("C:/workspace/demo".into());
    app.input.set("draft survives navigation");
    app.query.set("saved search");
    app.selected = 3;
    app.scroll = 7;
    app.tab = 2;

    app.command("/settings");
    assert_eq!(app.screen, Screen::Settings);
    app.close();

    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.input.text, "draft survives navigation");
    assert_eq!(app.query.text, "saved search");
    assert_eq!(app.selected, 3);
    assert_eq!(app.scroll, 7);
    assert_eq!(app.tab, 2);
}

#[test]
fn approval_defaults_to_decline_and_details_return_to_pending_approval() {
    let mut app = App::new("C:/workspace/demo".into());
    app.load_scenario("approval").unwrap();
    assert!(app.approval);
    assert_eq!(
        app.approval_selected, 1,
        "the safe initial choice should be Decline"
    );

    app.key(key(KeyCode::Right));
    assert_eq!(app.approval_selected, 2);
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Diff);
    assert!(app.approval);

    app.key(key(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Home);
    assert!(app.approval);
    assert_eq!(app.approval_selected, 2);
}

#[test]
fn interrupt_stops_simulation_and_preserves_current_draft() {
    let mut app = App::new("C:/workspace/demo".into());
    app.working = true;
    app.input.set("draft while working");
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(!app.working);
    assert_eq!(app.input.text, "draft while working");
    assert!(
        app.entries
            .iter()
            .any(|entry| entry.title == "Interrupted [demo]")
    );
}

#[test]
fn usage_and_stats_distinguish_unavailable_metrics_from_labeled_demo_data() {
    let mut usage = App::new("C:/workspace/demo".into());
    usage.load_scenario("usage").unwrap();
    let usage_empty = ui::dump(&usage, 80, 24).unwrap();
    assert!(usage_empty.contains("Unavailable"));
    assert!(!usage_empty.contains("DEMO DATA"));
    usage.key(key(KeyCode::Char('d')));
    let usage_demo = ui::dump(&usage, 80, 24).unwrap();
    assert!(usage_demo.contains("DEMO DATA"));

    let mut stats = App::new("C:/workspace/demo".into());
    stats.load_scenario("stats").unwrap();
    let stats_empty = ui::dump(&stats, 80, 24).unwrap();
    assert!(stats_empty.contains("Statistics unavailable"));
    assert!(!stats_empty.contains("0 tokens"));
    assert!(!stats_empty.contains("DEMO DATA"));
    stats.load_scenario("stats-demo").unwrap();
    let stats_demo = ui::dump(&stats, 80, 24).unwrap();
    assert!(stats_demo.contains("DEMO DATA"));
}

#[test]
fn all_scenarios_dump_deterministically_at_supported_terminal_sizes() {
    for (name, _) in SCENARIOS {
        let mut app = App::new("C:/workspace/demo".into());
        app.load_scenario(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        for (width, height) in [(120, 40), (80, 24), (60, 20)] {
            let first = ui::dump(&app, width, height)
                .unwrap_or_else(|error| panic!("{name} at {width}x{height}: {error}"));
            let second = ui::dump(&app, width, height)
                .unwrap_or_else(|error| panic!("{name} at {width}x{height}: {error}"));
            assert_eq!(
                first, second,
                "nondeterministic dump for {name} at {width}x{height}"
            );
            assert!(
                first.contains("Polaris v0.1.0alpha"),
                "missing persistent header for {name} at {width}x{height}"
            );
        }
    }
}

#[test]
fn models_scenario_opens_model_selection_and_loaded_conversation_keeps_full_history() {
    let mut app = App::new("C:/workspace/demo".into());
    app.load_scenario("models").unwrap();
    assert_eq!(app.screen, Screen::Models);

    app.load_scenario("conversation").unwrap();
    assert_eq!(app.screen, Screen::Home);
    assert!(!app.entries.is_empty());
    assert_eq!(app.entries.len(), 4);
    assert!(app.entries.last().unwrap().title.contains("demo response"));
}

#[test]
fn undersized_dump_preserves_draft_and_navigation_state() {
    let mut app = App::new("C:/workspace/demo".into());
    app.input.set("preserve this draft");
    app.scroll = 9;
    app.selected = 2;
    app.command("/settings");
    let screen = app.screen;
    let input = app.input.text.clone();
    let scroll = app.scroll;
    let selected = app.selected;

    let rendered = ui::dump(&app, 59, 19).unwrap();
    assert!(rendered.contains("Resize to at least 60 columns x 20 rows."));
    assert!(rendered.contains("Your draft and navigation state are preserved."));
    assert_eq!(app.screen, screen);
    assert_eq!(app.input.text, input);
    assert_eq!(app.scroll, scroll);
    assert_eq!(app.selected, selected);
}

#[test]
fn ascii_and_no_color_modes_use_plain_symbols_and_terminal_safe_styles() {
    let mut app = App::new("C:/workspace/demo".into());
    app.ascii = true;
    app.theme.color = false;
    app.load_scenario("welcome").unwrap();
    let rendered = ui::dump(&app, 80, 24).unwrap();
    assert!(rendered.is_ascii(), "ASCII mode emitted a non-ASCII glyph");
    assert_eq!(app.theme.accent().fg, Some(Color::Reset));
    assert!(
        !app.theme
            .selected()
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert_eq!(app.theme.selected().bg, None);

    app.theme.mode = ThemeMode::Terminal;
    assert_eq!(app.theme.accent().fg, Some(Color::Reset));
}

#[test]
fn long_unicode_input_cursor_wraps_on_display_cells() {
    let text = "界🙂é".repeat(160);
    let editor = {
        let mut e = Editor::default();
        e.set(&text);
        e
    };
    let (lines, cursor_row, cursor_col) = ui::wrap_editor(&editor.text, editor.cursor, 20);
    assert!(lines.len() > 20);
    assert!(cursor_row < lines.len());
    assert!(cursor_col <= 20);
    assert!(editor.text.is_char_boundary(editor.cursor));
    assert_eq!(
        cursor_col, 0,
        "cursor at end after a full display-cell row should wrap to column zero"
    );
}
