use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::{App, Entry, EntryKind, Screen},
    interaction::{TextRegionId, UiMap},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (map, t.backend().buffer().clone())
}
#[test]
fn prompt_marker_stays_visible_after_repeated_shift_enter_and_composer_scroll() {
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let mut app = App::default();
        for _ in 0..40 {
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        }
        app.input.insert("Last paragraph");
        for scroll in [0, 4, usize::MAX] {
            app.composer_scroll = scroll;
            let (map, b) = draw(&app, w, h);
            let r = map.region(&TextRegionId::Composer).unwrap();
            assert_eq!(
                b[(r.rect.x - 2, r.rect.y)].symbol(),
                ">",
                "prompt disappeared at {w}x{h}, scroll {scroll}"
            );
            assert_eq!(r.source.as_ref(), app.input.text);
        }
    }
}
#[test]
fn commands_and_completions_remain_available_while_working_without_resubmission() {
    let mut app = App::default();
    app.input.set("First request");
    app.submit();
    app.input.set("/");
    assert!(app.completion_open());
    assert!(!app.suggestions().is_empty());
    app.input.set("/usage");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Usage);
    assert!(app.working);
    assert_eq!(app.submissions.len(), 1);
    app.tick();
    assert!(app.working);
    assert_eq!(app.ticks, 1);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Home);
    app.input.set("Second request");
    app.submit();
    assert_eq!(app.input.text, "Second request");
    assert_eq!(app.submissions.len(), 1);
    app.input.set("Please /pl");
    assert!(!app.suggestions().is_empty());
    app.input.set("@");
    assert!(app.completion_open());
}
#[test]
fn opening_popups_keeps_the_conversation_star_artwork() {
    let mut app = App::default();
    app.entries
        .push(Entry::new(EntryKind::Assistant, "", &["Reply"]));
    let (_, before) = draw(&app, 120, 40);
    for command in ["/model", "/status", "/usage", "/settings"] {
        app.command(command);
        let (_, after) = draw(&app, 120, 40);
        for y in 2..7 {
            for x in 2..13 {
                assert_eq!(
                    after[(x, y)],
                    before[(x, y)],
                    "logo changed for {command} at {x},{y}"
                );
            }
        }
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
}

#[test]
fn title_hover_changes_only_tone_and_click_toggles_one_tool_while_control_o_toggles_all() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use polaris::{interaction::HitAction, runtime::MouseController};
    use ratatui::style::Color;
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Tool,
        "Read first [demo]",
        &["First detail"],
    ));
    app.entries.push(Entry::new(
        EntryKind::Tool,
        "Read second [demo]",
        &["Second detail"],
    ));
    let (map, before) = draw(&app, 100, 30);
    let target = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::ToggleTool(0))
        .unwrap()
        .rect;
    let mut mouse = MouseController::default();
    let event = |kind| MouseEvent {
        kind,
        column: target.x + 2,
        row: target.y,
        modifiers: KeyModifiers::NONE,
    };
    mouse.event(event(MouseEventKind::Moved), &map, &mut app);
    let (_, hover) = draw(&app, 100, 30);
    assert_eq!(
        before[(target.x + 2, target.y)].fg,
        Color::Rgb(249, 226, 175)
    );
    assert_eq!(
        hover[(target.x + 2, target.y)].fg,
        Color::Rgb(255, 253, 230)
    );
    assert!(
        before
            .content
            .iter()
            .zip(&hover.content)
            .all(|(a, b)| a.bg == b.bg)
    );
    mouse.event(
        event(MouseEventKind::Down(MouseButton::Left)),
        &map,
        &mut app,
    );
    mouse.event(event(MouseEventKind::Up(MouseButton::Left)), &map, &mut app);
    assert!(app.tool_expanded(0));
    assert!(!app.tool_expanded(1));
    let (map, expanded) = draw(&app, 100, 30);
    let text = expanded
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("First detail"));
    assert!(!text.contains("Second detail"));
    assert!(
        map.region(&TextRegionId::Entry(0))
            .unwrap()
            .source
            .contains("First detail")
    );
    assert!(
        map.region(&TextRegionId::Entry(1))
            .unwrap()
            .source
            .is_empty()
    );
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(app.tool_expanded(0) && app.tool_expanded(1));
    app.toggle_tool_details(0);
    assert!(!app.tool_expanded(0) && app.tool_expanded(1));
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(!app.tool_expanded(0) && !app.tool_expanded(1));
    app.command("/model");
    let (map, _) = draw(&app, 100, 30);
    let sheet = ui::popup_area(&app, ratatui::layout::Rect::new(0, 0, 100, 30));
    assert!(
        map.targets
            .iter()
            .filter(|t| matches!(t.action, HitAction::ToggleTool(_)))
            .all(|t| t.rect.y < sheet.y)
    );
    app.command("/clear");
    app.entries
        .push(Entry::new(EntryKind::Tool, "New", &["New detail"]));
    assert!(!app.tool_expanded(0));
}
#[test]
fn working_indicator_matches_bullet_elapsed_and_bold_escape_hint_with_visible_marker() {
    use std::time::{Duration, Instant};
    let mut app = App::default();
    app.input.set("Task");
    app.submit();
    assert!(app.working_since.is_some());
    app.working_since = Some(Instant::now() - Duration::from_secs(65));
    app.input.set(&"paragraph\n".repeat(30));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let (map, b) = draw(&app, w, h);
        let text = b.content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(text.contains("• Working (1m 05s · esc to interrupt)"));
        assert!(!text.contains("Ctrl+C to interrupt"));
        assert!(
            b.content.windows(3).any(|cells| cells[0].symbol() == "e"
                && cells[1].symbol() == "s"
                && cells[2].symbol() == "c"
                && cells
                    .iter()
                    .all(|c| c.modifier.contains(ratatui::style::Modifier::BOLD))),
            "escape hint should be bold"
        );
        let r = map.region(&TextRegionId::Composer).unwrap();
        assert_eq!(b[(r.rect.x - 2, r.rect.y)].symbol(), ">");
    }
    app.working_since = Some(Instant::now());
    let (_, b) = draw(&app, 80, 24);
    assert!(
        b.content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("• Working (0s · esc to interrupt)")
    );
    app.working_since = Some(Instant::now() - Duration::from_secs(394));
    app.ascii = true;
    app.theme.color = false;
    let (_, plain) = draw(&app, 80, 24);
    let text = plain.content.iter().map(|c| c.symbol()).collect::<String>();
    assert!(text.contains("* Working (6m 34s . esc to interrupt)"));
    assert!(!text.contains('•'));
    assert!(
        plain
            .content
            .iter()
            .all(|c| c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset)
    );
}
