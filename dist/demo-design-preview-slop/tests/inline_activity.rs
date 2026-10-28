use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::{App, Entry, EntryKind, Screen},
    interaction::{TextRegionId, UiMap, ViewSelection},
    runtime::selection_text,
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap();
    (map, terminal.backend().buffer().clone())
}
fn toggle(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
}
fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|c| c.symbol()).collect()
}
#[test]
fn control_o_expands_all_existing_and_future_tools_inline_and_preserves_draft() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Tool,
        "Read one",
        &["First private detail"],
    ));
    app.entries
        .push(Entry::new(EntryKind::Assistant, "", &["First reply."]));
    app.entries.push(Entry::new(
        EntryKind::Tool,
        "Read two",
        &["Second private detail"],
    ));
    app.entries
        .push(Entry::new(EntryKind::Assistant, "", &["Second reply."]));
    app.input.set("Unsent draft");
    app.input.anchor = Some(2);
    let cursor = app.input.cursor;
    let (map, collapsed) = draw(&app, 120, 45);
    assert!(!text(&collapsed).contains("private detail"));
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: map
            .region(&TextRegionId::Conversation)
            .unwrap()
            .source
            .len(),
    });
    toggle(&mut app);
    assert_eq!(
        app.screen,
        Screen::Home,
        "Ctrl+O must not open a transcript popup"
    );
    assert_eq!(app.input.text, "Unsent draft");
    assert_eq!((app.input.anchor, app.input.cursor), (Some(2), cursor));
    assert!(
        app.view_selection.is_none(),
        "changed source offsets must invalidate old conversation selection"
    );
    let (_, expanded) = draw(&app, 120, 45);
    assert!(text(&expanded).contains("First private detail"));
    assert!(text(&expanded).contains("Second private detail"));
    assert!(!text(&expanded).contains("Transcript"));
    app.entries.push(Entry::new(
        EntryKind::Tool,
        "Read three",
        &["Third private detail"],
    ));
    let (_, expanded) = draw(&app, 120, 45);
    assert!(text(&expanded).contains("Third private detail"));
    toggle(&mut app);
    assert_eq!(app.screen, Screen::Home);
    let (_, collapsed) = draw(&app, 120, 45);
    assert!(!text(&collapsed).contains("private detail"));
    assert_eq!(text(&collapsed).matches("Ctrl+O to expand").count(), 3);
    app.command("/status");
    toggle(&mut app);
    assert_eq!(
        app.screen,
        Screen::Status,
        "shortcut must not replace unrelated popups"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Home);
}
#[test]
fn expanded_tool_copy_maps_follow_wrapping_resize_and_scroll() {
    let mut app = App::default();
    let details = (0..40)
        .map(|i| {
            format!("Detail {i:02}: several words with café and ordinary source text that wraps.")
        })
        .collect::<Vec<_>>();
    app.entries.push(Entry {
        kind: EntryKind::Tool,
        title: "Read validation".into(),
        lines: details,
        draft: None,
    });
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "",
        &["Following reply remains mapped."],
    ));
    toggle(&mut app);
    assert_eq!(app.screen, Screen::Home);
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        for scroll in [0, 10, usize::MAX] {
            app.scroll = scroll;
            let (map, b) = draw(&app, w, h);
            let source = map.region(&TextRegionId::Conversation).unwrap();
            assert!(source.source.contains("Detail 00:"));
            assert!(source.source.contains("Detail 39:"));
            for c in &source.cells {
                let Some(ch) = source.source[c.byte_offset..]
                    .chars()
                    .next()
                    .filter(|c| !c.is_whitespace())
                else {
                    continue;
                };
                if source
                    .cells
                    .iter()
                    .any(|other| other.byte_offset == c.byte_offset && other.y > c.y)
                {
                    continue;
                }
                assert_eq!(
                    b[(c.x, c.y)].symbol(),
                    ch.to_string(),
                    "source shifted at {w}x{h} scroll {scroll}"
                );
            }
            app.view_selection = Some(ViewSelection {
                region: TextRegionId::Conversation,
                anchor: 0,
                end: source.source.len(),
            });
            assert_eq!(selection_text(&app, &map), source.source.as_ref());
            app.view_selection = None;
        }
    }
    toggle(&mut app);
    let (map, _) = draw(&app, 80, 24);
    assert!(
        !map.region(&TextRegionId::Conversation)
            .unwrap()
            .source
            .contains("Detail")
    );
}

#[test]
fn incomplete_completion_tokens_and_unfocused_rows_are_gray() {
    use polaris::interaction::HitAction;
    use ratatui::style::Color;
    let mut app = App::default();
    for input in ["/", "/pl", "@"] {
        app.input.set(input);
        let (map, b) = draw(&app, 100, 30);
        let composer = map.region(&TextRegionId::Composer).unwrap();
        let first = composer.cells.iter().find(|c| c.byte_offset == 0).unwrap();
        assert_eq!(
            b[(first.x, first.y)].fg,
            Color::Rgb(133, 133, 133),
            "unfinished token {input} should be gray"
        );
        for target in map
            .targets
            .iter()
            .filter(|t| matches!(t.action,HitAction::Suggestion(i) if i>0))
        {
            assert_eq!(
                b[(target.rect.x, target.rect.y)].fg,
                Color::Rgb(133, 133, 133)
            );
        }
    }
    app.input.set("/model");
    let (map, b) = draw(&app, 100, 30);
    let c = map
        .region(&TextRegionId::Composer)
        .unwrap()
        .cells
        .first()
        .unwrap();
    assert_eq!(
        b[(c.x, c.y)].fg,
        Color::Rgb(118, 185, 0),
        "recognized command keeps its accent"
    );
}
