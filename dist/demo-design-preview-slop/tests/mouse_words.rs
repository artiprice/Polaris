#[path = "support/selection.rs"]
mod selection;
use polaris::{
    app::{App, Entry, EntryKind},
    interaction::{TextRegionId, UiMap, ViewSelection},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    term.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (map, term.backend().buffer().clone())
}
#[test]
fn assistant_selection_does_not_underline_empty_rows_or_padding() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris · demo response",
        &[
            "First paragraph.",
            "",
            "Suggested approach:",
            "1. Extract the request guard.",
            "",
            "Final paragraph.",
        ],
    ));
    let (map, _) = draw(&app, 80, 30);
    let r = map.region(&TextRegionId::Conversation).unwrap();
    let first = r.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let next = r
        .cells
        .iter()
        .find(|c| c.byte_offset == r.source.find("Suggested").unwrap())
        .unwrap();
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: r.source.len(),
    });
    let (_, selected) = draw(&app, 80, 30);
    for y in first.y + 1..next.y {
        for x in r.rect.x..r.rect.right() {
            assert!(
                !selected[(x, y)].modifier.contains(Modifier::UNDERLINED),
                "empty row underlined at {x},{y}"
            );
        }
    }
    let end_x = first.x + "First paragraph.".len() as u16;
    for x in end_x..r.rect.right() {
        assert!(
            !selected[(x, first.y)]
                .modifier
                .contains(Modifier::UNDERLINED),
            "padding underlined at {x},{}",
            first.y
        );
    }
}
#[test]
fn assistant_body_has_no_role_heading_and_starts_at_history_top() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris · demo response",
        &["Response body."],
    ));
    let (map, b) = draw(&app, 80, 24);
    let text = b.content.iter().map(|c| c.symbol()).collect::<String>();
    assert!(!text.contains("demo response"));
    assert!(!text.contains("◆ polaris"));
    let r = map.region(&TextRegionId::Conversation).unwrap();
    let first = r.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    assert_eq!(first.y, r.rect.y);
    assert_eq!(r.source.as_ref(), "Response body.");
}

fn send(
    c: &mut polaris::runtime::MouseController,
    app: &mut App,
    map: &UiMap,
    kind: crossterm::event::MouseEventKind,
    p: (u16, u16),
) {
    c.event(
        crossterm::event::MouseEvent {
            kind,
            column: p.0,
            row: p.1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
        map,
        app,
    );
}
fn point(map: &UiMap, id: &TextRegionId, byte: usize) -> (u16, u16) {
    let r = map.region(id).unwrap();
    let c = r
        .cells
        .iter()
        .find(|c| c.byte_offset == byte && c.x < r.rect.right())
        .unwrap();
    (c.x, c.y)
}
fn double_down(
    c: &mut polaris::runtime::MouseController,
    app: &mut App,
    map: &UiMap,
    p: (u16, u16),
) {
    use crossterm::event::{MouseButton as B, MouseEventKind as M};
    send(c, app, map, M::Down(B::Left), p);
    send(c, app, map, M::Up(B::Left), p);
    send(c, app, map, M::Down(B::Left), p);
}
#[test]
fn word_hold_drag_crosses_wrapped_responses_and_message_boundaries_in_both_directions() {
    use crossterm::event::{MouseButton as B, MouseEventKind as M};
    use polaris::runtime::{MouseController, selection_text};
    let mut app = App::default();
    app.entries
        .push(Entry::new(EntryKind::User, "", &["The first café prompt."]));
    app.entries.push(Entry::new(EntryKind::Assistant,"polaris · demo response", &["A long response with ordinary words near a wrapping boundary. More words continue on another visual row.","","Another naïve paragraph closes the response."]));
    app.input
        .set("draft selection must clear when clicking the transcript");
    let (map, before) = draw(&app, 60, 40);
    app.input.anchor = Some(0);
    let r = map.region(&TextRegionId::Conversation).unwrap();
    let start = r.source.find("café").unwrap();
    let end = r.source.find("naïve").unwrap();
    let expected = &r.source[start..end + "naïve".len()];
    for (a, b, initial) in [(start + 1, end + 2, "café"), (end + 2, start + 1, "naïve")] {
        let mut c = MouseController::default();
        double_down(
            &mut c,
            &mut app,
            &map,
            point(&map, &TextRegionId::Conversation, a),
        );
        assert!(app.input.selection().is_none());
        assert_eq!(selection_text(&app, &map), initial);
        send(
            &mut c,
            &mut app,
            &map,
            M::Drag(B::Left),
            point(&map, &TextRegionId::Conversation, b),
        );
        send(
            &mut c,
            &mut app,
            &map,
            M::Up(B::Left),
            point(&map, &TextRegionId::Conversation, b),
        );
        assert_eq!(selection_text(&app, &map), expected);
        let (_, selected) = draw(&app, 60, 40);
        assert!(
            before
                .content
                .iter()
                .zip(&selected.content)
                .all(|(a, b)| a.bg == b.bg || b.bg == selection::background(app.theme, a))
        );
        assert!(
            matches!(app.clipboard_actions.last(),Some(polaris::app::ClipboardAction::Copy(_,text)) if text==expected)
        );
    }
}
#[test]
fn composer_and_attachment_preview_double_clicks_select_and_copy_unicode_words() {
    use crossterm::event::{MouseButton as B, MouseEventKind as M};
    use polaris::runtime::{MouseController, selection_text};
    let mut app = App::default();
    app.input.set("alpha café gamma");
    let (map, _) = draw(&app, 80, 24);
    let mut c = MouseController::default();
    let p = point(&map, &TextRegionId::Composer, 8);
    double_down(&mut c, &mut app, &map, p);
    assert_eq!(app.input.selected_text(), "café");
    send(&mut c, &mut app, &map, M::Up(B::Left), p);
    assert!(
        matches!(app.clipboard_actions.last(),Some(polaris::app::ClipboardAction::Copy(_,text)) if text=="café")
    );
    app.input.clear();
    let pasted = "naïve café paragraph\n".repeat(50);
    app.paste(&pasted);
    let id = app.input.spans[0].attachment.id;
    app.open_attachment(id);
    let (map, _) = draw(&app, 80, 24);
    let id = TextRegionId::AttachmentPreview(id);
    let p = point(&map, &id, 8);
    double_down(&mut c, &mut app, &map, p);
    assert_eq!(selection_text(&app, &map), "café");
    send(&mut c, &mut app, &map, M::Up(B::Left), p);
    assert!(
        matches!(app.clipboard_actions.last(),Some(polaris::app::ClipboardAction::Copy(_,text)) if text=="café")
    );
}
#[test]
fn repeated_selection_frames_shade_selected_text_and_monochrome_uses_bold() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris · demo response",
        &["An ordinary paragraph.", "", "Another ordinary paragraph."],
    ));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, &app))
        .unwrap();
    let before = terminal.backend().buffer().clone();
    let r = map.region(&TextRegionId::Conversation).unwrap().clone();
    for color in [true, false] {
        app.theme.color = color;
        for (anchor, end) in [(0, r.source.len()), (4, 8), (0, 0), (0, r.source.len())] {
            app.view_selection = Some(ViewSelection {
                region: TextRegionId::Conversation,
                anchor,
                end,
            });
            terminal
                .draw(|f| map = ui::render_interactive(f, &app))
                .unwrap();
            let b = terminal.backend().buffer();
            for y in r.rect.y..r.rect.bottom() {
                for x in r.rect.x..r.rect.right() {
                    assert!(
                        !b[(x, y)].modifier.contains(Modifier::UNDERLINED),
                        "selection underlined {x},{y}"
                    );
                    if color {
                        let selected_cell = r.cells.iter().any(|c| {
                            c.x == x
                                && c.y == y
                                && c.byte_offset >= anchor.min(end)
                                && c.byte_offset < anchor.max(end)
                                && !r.source[c.byte_offset..].starts_with('\n')
                        });
                        assert_eq!(
                            b[(x, y)].bg,
                            if selected_cell {
                                selection::background(app.theme, &before[(x, y)])
                            } else {
                                before[(x, y)].bg
                            }
                        );
                    }
                }
            }
            if anchor != end {
                let p = point(&map, &TextRegionId::Conversation, anchor);
                assert_eq!(b[p].modifier.contains(Modifier::BOLD), !color);
            }
        }
    }
}

#[test]
fn multiline_assistant_source_cells_match_visible_glyphs_after_every_blank_and_wrap() {
    let mut app = App::default();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris · demo response",
        &[
            "The repeated validation can move into a shared helper.",
            "",
            "Suggested approach:",
            "1. Extract the request guard.",
            "2. Keep error messages at the handler boundary.",
            "3. Add cases for missing and malformed input.",
            "",
            "This is a scripted UI demonstration. No files were read or changed.",
            "Open /diff to review the sample patch.",
        ],
    ));
    for width in [60, 80, 120] {
        let (map, b) = draw(&app, width, 40);
        let r = map.region(&TextRegionId::Conversation).unwrap();
        for c in &r.cells {
            let Some(ch) = r.source[c.byte_offset..]
                .chars()
                .next()
                .filter(|c| !c.is_whitespace())
            else {
                continue;
            };
            // This fixture has no tabs, wide glyphs or inline markup; only true row-end
            // insertion markers may point at the next wrapped word.
            if r.cells
                .iter()
                .any(|next| next.byte_offset == c.byte_offset && next.y > c.y)
            {
                continue;
            }
            assert_eq!(
                b[(c.x, c.y)].symbol(),
                ch.to_string(),
                "source mapping shifted at {width} columns, byte {}, cell {},{}",
                c.byte_offset,
                c.x,
                c.y
            );
        }
    }
}

#[test]
fn wrapped_attachment_preview_source_cells_match_text_at_all_scroll_positions() {
    use unicode_segmentation::UnicodeSegmentation;
    let mut app = App::default();
    let text="A preview paragraph with café, emoji 👩‍💻 and several words that must wrap naturally before the next sentence.\n\n\tIndented source follows with more words and whitespace.\n".repeat(12);
    app.paste(&text);
    let id = app.input.spans[0].attachment.id;
    app.open_attachment(id);
    for scroll in [0, 5, usize::MAX] {
        app.preview_scroll = scroll;
        let (map, b) = draw(&app, 60, 24);
        let r = map.region(&TextRegionId::AttachmentPreview(id)).unwrap();
        assert!(!r.cells.is_empty());
        for c in &r.cells {
            let Some(g) = r.source[c.byte_offset..]
                .graphemes(true)
                .next()
                .filter(|s| !s.chars().all(char::is_whitespace))
            else {
                continue;
            };
            if r.cells
                .iter()
                .any(|next| next.byte_offset == c.byte_offset && next.y > c.y)
            {
                continue;
            }
            assert_eq!(
                b[(c.x, c.y)].symbol(),
                g,
                "preview source shifted at scroll {scroll}, byte {}",
                c.byte_offset
            );
        }
    }
}
