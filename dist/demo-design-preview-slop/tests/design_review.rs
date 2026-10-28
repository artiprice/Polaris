//! Behavior-level design checks; optional SVGs are exact Ratatui buffer exports.
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, Entry, EntryKind, SCENARIOS, Screen},
    interaction::{HitAction, TextRegionId, UiMap},
    runtime::MouseController,
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
fn draw(app: &App, w: u16, h: u16) -> (Buffer, UiMap) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (t.backend().buffer().clone(), map)
}
#[path = "support/capture.rs"]
mod capture;
use capture::export;
#[test]
fn every_screen_fits_the_shared_column_and_exposes_only_visible_hit_targets() {
    for (name, _) in SCENARIOS {
        let mut app = App::new("C:/workspace/polaris".into());
        app.load_scenario(name).unwrap();
        for (w, h) in [(60, 20), (80, 24), (100, 30), (120, 40), (160, 60)] {
            let (b, map) = draw(&app, w, h);
            assert_eq!(b.area.width, w);
            assert_eq!(b.area.height, h);
            for target in &map.targets {
                assert!(target.rect.width > 0 && target.rect.height > 0);
                assert!(
                    target.rect.right() <= w && target.rect.bottom() <= h,
                    "{name} target {:?} outside {w}x{h}",
                    target
                );
            }
            for r in &map.text_regions {
                for cell in &r.cells {
                    assert!(cell.x <= w && cell.y < h);
                    assert!(r.source.is_char_boundary(cell.byte_offset));
                }
            }
            if app.screen != Screen::Home {
                let sheet = ui::popup_area(&app, b.area);
                assert!(
                    map.text_regions
                        .iter()
                        .all(|r| r.cells.iter().all(|c| !sheet.contains((c.x, c.y).into())))
                );
            }
            assert!(
                b.content
                    .iter()
                    .all(|c| !c.modifier.contains(Modifier::REVERSED))
            );
            export(name, &b);
        }
    }
}
#[test]
fn long_editor_resizes_in_both_directions_without_losing_text_or_cursor() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "",
        &["Keep history available."],
    ));
    app.input.set(&"界é words\n".repeat(100));
    let original = app.input.clone();
    for (w, h) in [
        (80, 24),
        (160, 60),
        (60, 20),
        (100, 30),
        (120, 100),
        (80, 24),
    ] {
        app.editor_width = ui::editor_width(w);
        let (b, map) = draw(&app, w, h);
        let r = map.region(&TextRegionId::Composer).unwrap();
        assert_eq!(r.rect.height, h * 3 / 8);
        assert_eq!(r.rect.width, app.editor_width as u16);
        assert_eq!(r.source.as_ref(), original.text);
        assert_eq!(app.input.cursor, original.cursor);
        assert!(map.region(&TextRegionId::Conversation).unwrap().rect.height >= 4);
        export("long-draft", &b);
    }
}
fn click(app: &mut App, map: &UiMap, action: HitAction) {
    let rect = map
        .targets
        .iter()
        .find(|t| t.action == action)
        .expect("control has a real target")
        .rect;
    let mut mouse = MouseController::default();
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        mouse.event(
            MouseEvent {
                kind,
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            map,
            app,
        );
    }
}
#[test]
fn footer_and_approval_controls_execute_their_labels() {
    let mut app = App::default();
    let (_, map) = draw(&app, 120, 40);
    click(&mut app, &map, HitAction::FooterHelp);
    assert_eq!(app.screen, Screen::Help);
    app.close();
    let (_, map) = draw(&app, 120, 40);
    click(&mut app, &map, HitAction::FooterMode);
    assert_eq!(app.mode, 1);
    let (_, map) = draw(&app, 120, 40);
    click(&mut app, &map, HitAction::FooterModel);
    assert_eq!(app.screen, Screen::Models);
    app.close();
    app.load_scenario("approval").unwrap();
    let (_, map) = draw(&app, 60, 20);
    click(&mut app, &map, HitAction::Approval(1));
    assert!(!app.approval);
    assert_eq!(app.screen, Screen::Home);
}
#[test]
fn status_rows_and_pending_approval_leave_a_real_bounded_editor_in_short_windows() {
    let mut app = App::default();
    app.load_scenario("approval").unwrap();
    app.input.set("/goal keep this draft");
    app.approval = false;
    app.submit();
    app.approval = true;
    app.notice = "Local preview notice".into();
    app.input.set(&"draft\n".repeat(80));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let (b, map) = draw(&app, w, h);
        let r = map.region(&TextRegionId::Composer).unwrap();
        assert!(r.rect.height >= 1);
        assert!(r.rect.bottom() <= h - 3);
        assert!(r.rect.y > 2);
        assert_eq!(r.source.as_ref(), app.input.text);
        for target in &map.targets {
            assert!(target.rect.right() <= w && target.rect.bottom() <= h);
        }
        export("goal-approval", &b);
    }
}
