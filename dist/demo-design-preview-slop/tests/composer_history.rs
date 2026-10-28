use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::App,
    interaction::{TextRegionId, UiMap},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, style::Color};
fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn send(app: &mut App, text: &str) {
    app.input.set(text);
    app.submit();
    app.working = false;
}
#[test]
fn sent_prompts_trim_only_trailing_blank_rows_and_keep_paragraphs_and_indentation() {
    for (input, expected, rows) in [
        ("see this?\n\n \t\n", "see this?", 1),
        ("It's fine\n\nSituation\n\n", "It's fine\n\nSituation", 3),
        ("  code  \n\tmore  \n \n", "  code  \n\tmore  ", 2),
    ] {
        let mut app = App::default();
        send(&mut app, input);
        assert_eq!(app.entries[0].draft.as_ref().unwrap().text, expected);
        assert_eq!(app.submissions[0].text, expected);
        let mut terminal = Terminal::new(TestBackend::new(100, 35)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|f| map = ui::render_interactive(f, &app))
            .unwrap();
        assert_eq!(
            map.region(&TextRegionId::Conversation)
                .unwrap()
                .source
                .as_ref(),
            expected
        );
        let buffer = terminal.backend().buffer();
        let gray_rows = (0..35)
            .filter(|&y| (0..100).any(|x| buffer[(x, y)].bg == Color::Rgb(20, 20, 20)))
            .count();
        assert_eq!(gray_rows, rows, "extra gray rows for {input:?}");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.input.text, expected);
    }
}
#[test]
fn trimming_keeps_attachment_payload_and_rejected_send_keeps_entire_draft() {
    let mut app = App::default();
    app.paste(&"x".repeat(501));
    app.paste_image(1, 1, vec![255; 4]);
    let original = app.input.snapshot();
    app.input.insert("\n \n\n");
    app.working = true;
    let unsent = app.input.clone();
    app.submit();
    assert_eq!(app.input, unsent);
    app.working = false;
    app.submit();
    let sent = app.entries[0].draft.as_ref().unwrap();
    assert_eq!(sent.text, original.text);
    assert_eq!(sent.spans, original.spans);
    assert_eq!(sent.expanded_text(), original.expanded_text());
}
#[test]
fn arrows_move_inside_draft_then_recall_history_and_restore_unsent_draft() {
    let mut app = App::default();
    send(&mut app, "older");
    send(&mut app, "newer");
    app.input.set("draft top\ndraft bottom");
    let original = app.input.clone();
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, original.text);
    assert_eq!(app.input.cursor, 9);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "newer");
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "older");
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "older");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input.text, "newer");
    app.input.insert(" edited");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input.text, original.text);
    assert_eq!(app.input.cursor, 9); // Cursor saved at the top boundary before history recall.
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input.cursor, original.cursor);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    assert_eq!(
        app.input.text, "newer",
        "editing recall must not mutate stored history"
    );
}
#[test]
fn history_boundaries_use_wrapped_visual_rows_and_preserve_attachment_draft_undo() {
    let mut app = App::default();
    send(&mut app, "saved");
    app.editor_width = 5;
    app.input.set("abcdefghij");
    press(&mut app, KeyCode::Up); // final empty visual row -> second wrapped row
    assert_eq!(app.input.text, "abcdefghij");
    assert_eq!(app.input.cursor, 5);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.cursor, 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "saved");
    press(&mut app, KeyCode::Down); // history recall at its end -> restore draft
    assert_eq!(app.input.text, "abcdefghij");
    app.input.clear();
    app.editor_width = 100;
    app.paste_image(1, 1, vec![255; 4]);
    app.input.insert(" tail");
    let attachment_draft = app.input.clone();
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "saved");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.input, attachment_draft);
    app.input.undo();
    assert_eq!(app.input.text, "[Image 1]");
    assert_eq!(app.input.spans.len(), 1);
}
#[test]
fn completion_and_modified_arrows_take_priority_over_history() {
    let mut app = App::default();
    send(&mut app, "saved");
    app.input.set("/");
    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.selected, 0);
    assert_eq!(app.input.text, "/");
    app.input.set("draft");
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.input.text, "draft");
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
    assert_eq!(app.input.text, "draft");
    app.command("/clear");
    press(&mut app, KeyCode::Up);
    assert!(app.input.text.is_empty());
}

#[test]
fn cancelling_history_browse_or_loading_a_scenario_discards_stashed_draft() {
    let mut app = App::default();
    send(&mut app, "older");
    send(&mut app, "newer");
    app.input.set("unsent");
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.input.text.is_empty());
    press(&mut app, KeyCode::Up);
    assert_eq!(
        app.input.text, "newer",
        "cancel must restart from newest history"
    );
    press(&mut app, KeyCode::Down);
    assert!(app.input.text.is_empty(), "cancelled draft must not return");
    app.input.set("another draft");
    press(&mut app, KeyCode::Up);
    app.load_scenario("welcome").unwrap();
    press(&mut app, KeyCode::Down);
    assert!(
        app.input.text.is_empty(),
        "scenario reset must not resurrect a draft"
    );
    press(&mut app, KeyCode::Up);
    assert_eq!(app.input.text, "newer");
}
