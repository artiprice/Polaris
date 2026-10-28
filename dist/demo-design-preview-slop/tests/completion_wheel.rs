use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::App,
    completion::FileIndex,
    interaction::{HitAction, UiMap},
    runtime::MouseController,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
use std::time::{Duration, Instant};
fn draw(app: &App, width: u16, height: u16) -> UiMap {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap();
    map
}
fn event(
    mouse: &mut MouseController,
    app: &mut App,
    map: &UiMap,
    kind: MouseEventKind,
    x: u16,
    y: u16,
) {
    mouse.event(
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
        map,
        app,
    );
}
fn suggestion(map: &UiMap, index: usize) -> ratatui::layout::Rect {
    map.targets
        .iter()
        .find(|t| t.action == HitAction::Suggestion(index))
        .unwrap()
        .rect
}
#[test]
fn command_wheel_reaches_hidden_rows_clamps_and_keeps_draft_and_conversation_still() {
    for (width, height) in [(60, 20), (120, 40)] {
        let mut app = App::default();
        app.load_scenario("conversation").unwrap();
        app.input.set("/");
        app.scroll = 7;
        let draft = app.input.clone();
        let count = app.suggestions().len();
        assert!(count > 8);
        let mut mouse = MouseController::default();
        for expected in 1..count {
            let map = draw(&app, width, height);
            let r = map
                .targets
                .iter()
                .find(|t| matches!(t.action, HitAction::Suggestion(_)))
                .unwrap()
                .rect;
            event(
                &mut mouse,
                &mut app,
                &map,
                MouseEventKind::ScrollDown,
                r.x,
                r.y,
            );
            assert_eq!(app.selected, expected);
            assert_eq!(app.scroll, 7);
            assert_eq!(app.input, draft);
        }
        let map = draw(&app, width, height);
        let r = suggestion(&map, count - 1);
        assert!(
            !map.targets
                .iter()
                .any(|t| t.action == HitAction::Suggestion(0)),
            "viewport must reveal later commands"
        );
        event(
            &mut mouse,
            &mut app,
            &map,
            MouseEventKind::ScrollDown,
            r.x,
            r.y,
        );
        assert_eq!(app.selected, count - 1);
        event(
            &mut mouse,
            &mut app,
            &map,
            MouseEventKind::ScrollUp,
            r.x,
            r.y,
        );
        assert_eq!(app.selected, count - 2);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.selected, count - 1);
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.selected, count - 2);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.selected, count - 1);
        let map = draw(&app, width, height);
        let r = suggestion(&map, count - 1);
        event(
            &mut mouse,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            r.x + 3,
            r.y,
        );
        event(
            &mut mouse,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            r.x + 3,
            r.y,
        );
        assert_eq!(app.input.text, "/goal ");
        assert!(!app.working);
    }
}
#[test]
fn file_wheel_captures_full_width_padding_and_top_margin_then_accepts_visible_file() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("wheel-files-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..24 {
        std::fs::write(root.join(format!("file{i:02}.rs")), "").unwrap();
    }
    let mut index = FileIndex::start(root.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !index.is_ready() {
        index.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut app = App::default();
    app.file_index = Some(index);
    app.input.set("@");
    app.scroll = 11;
    let mut mouse = MouseController::default();
    for expected in 1..24 {
        let map = draw(&app, 100, 30);
        let first = map
            .targets
            .iter()
            .find(|t| matches!(t.action, HitAction::Suggestion(_)))
            .unwrap()
            .rect;
        // Full-width blank top margin, including far-right padding, belongs to the menu.
        event(
            &mut mouse,
            &mut app,
            &map,
            MouseEventKind::ScrollDown,
            99,
            first.y - 1,
        );
        assert_eq!(app.selected, expected);
        assert_eq!(app.scroll, 11);
        assert_eq!(app.composer_scroll, 0);
        assert_eq!(app.input.text, "@");
    }
    let map = draw(&app, 100, 30);
    let r = suggestion(&map, 23);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollDown,
        0,
        r.y,
    );
    assert_eq!(app.selected, 23);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::Down(MouseButton::Left),
        r.x + 3,
        r.y,
    );
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::Up(MouseButton::Left),
        r.x + 3,
        r.y,
    );
    assert_eq!(app.input.text, "@file23.rs ");
    assert!(!app.working);
    for i in 0..24 {
        std::fs::remove_file(root.join(format!("file{i:02}.rs"))).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}
#[test]
fn directive_wheel_clamps_but_dismissed_or_empty_menus_keep_normal_scrolling() {
    let mut app = App::default();
    app.input.set("Please /");
    let mut mouse = MouseController::default();
    let map = draw(&app, 100, 30);
    let r = suggestion(&map, 0);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollUp,
        r.x,
        r.y,
    );
    assert_eq!(app.selected, 0);
    assert_eq!(app.scroll, 0);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollDown,
        r.x,
        r.y,
    );
    assert_eq!(app.selected, 1);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollDown,
        r.x,
        r.y,
    );
    assert_eq!(app.selected, 1);
    app.menu_dismissed = true;
    let map = draw(&app, 100, 30);
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollUp,
        r.x,
        r.y,
    );
    assert_eq!(app.scroll, 3);
    app.input.set("/no-such-command");
    app.menu_dismissed = false;
    let map = draw(&app, 100, 30);
    assert!(
        !map.targets
            .iter()
            .any(|t| matches!(t.action, HitAction::Suggestion(_)))
    );
    event(
        &mut mouse,
        &mut app,
        &map,
        MouseEventKind::ScrollUp,
        r.x,
        r.y,
    );
    assert_eq!(app.scroll, 6);
}
