#[path = "support/selection.rs"]
mod selection;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, ClipboardAction, Entry, EntryKind},
    interaction::{HitAction, TextRegionId, UiMap, ViewSelection},
    runtime::{MouseController, selection_text},
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};

fn draw(app: &App, width: u16, height: u16) -> (UiMap, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap();
    (map, terminal.backend().buffer().clone())
}
fn mouse(
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

#[test]
fn paste_label_counts_unicode_characters_not_bytes_or_sequence() {
    let mut app = App::default();
    app.paste(&"é".repeat(501));
    assert_eq!(app.input.text, "[Pasted Content 501 chars]");
    assert_eq!(app.input.expanded_text(), "é".repeat(501));
    app.input.clear();
    app.paste("a\nb\nc\nd\ne\nf");
    assert_eq!(app.input.text, "[Pasted Content 11 chars]");
    app.input.backspace();
    assert!(app.input.text.is_empty());
    app.input.undo();
    assert_eq!(app.input.text, "[Pasted Content 11 chars]");
}

#[test]
fn drag_selects_across_prompts_answers_and_paragraphs_in_both_directions() {
    let mut app = App::default();
    app.input.set("first prompt\nsecond line");
    app.submit();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris",
        &["A paragraph.", "", "Another paragraph."],
    ));
    app.working = false;
    app.input.set("last prompt");
    app.submit();
    app.working = false;
    let (map, _) = draw(&app, 100, 40);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    let expected = "first prompt\nsecond line\n\nA paragraph.\n\nAnother paragraph.\n\nlast prompt";
    assert_eq!(region.source.as_ref(), expected);
    let start = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let end = region
        .cells
        .iter()
        .find(|c| c.byte_offset == expected.len())
        .unwrap();
    for (a, b) in [(start, end), (end, start)] {
        let mut controller = MouseController::default();
        mouse(
            &mut controller,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            a.x,
            a.y,
        );
        mouse(
            &mut controller,
            &mut app,
            &map,
            MouseEventKind::Drag(MouseButton::Left),
            b.x,
            b.y,
        );
        mouse(
            &mut controller,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            b.x,
            b.y,
        );
        assert_eq!(selection_text(&app, &map), expected);
        let Some(ClipboardAction::Copy(_, text)) = app.clipboard_actions.pop() else {
            panic!("selection must request a copy")
        };
        assert_eq!(text, expected);
    }
}

#[test]
fn conversation_copy_expands_pasted_content_across_entry_boundaries() {
    let mut app = App::default();
    app.input.set("read ");
    app.paste(&"λ".repeat(501));
    app.submit();
    app.entries
        .push(Entry::new(EntryKind::Assistant, "polaris", &["Received."]));
    let (map, _) = draw(&app, 120, 40);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: region.source.len(),
    });
    assert_eq!(
        selection_text(&app, &map),
        format!("read {}\n\nReceived.", "λ".repeat(501))
    );
}

#[test]
fn sent_prompt_has_sampled_gray_without_you_heading_and_dark_copy_selection() {
    let mut app = App::default();
    app.input.set("plain prompt\nnext line");
    app.submit();
    app.working = false;
    let (map, before) = draw(&app, 80, 24);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    let text = before
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!text.contains("> you"));
    for cell in &region.cells {
        assert_eq!(before[(cell.x, cell.y)].bg, Color::Rgb(20, 20, 20));
    }
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: region.source.len(),
    });
    let (_, after) = draw(&app, 80, 24);
    for (index, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
        let (x, y) = ((index % 80) as u16, (index / 80) as u16);
        let selected = region.cells.iter().any(|c| {
            c.x == x
                && c.y == y
                && c.byte_offset < region.source.len()
                && !region.source[c.byte_offset..].starts_with('\n')
        });
        assert_eq!(
            b.bg,
            if selected {
                selection::background(app.theme, a)
            } else {
                a.bg
            }
        );
        assert!(!b.modifier.contains(Modifier::REVERSED));
    }
    let first = region.cells.first().unwrap();
    assert_eq!(after[(first.x, first.y)].fg, before[(first.x, first.y)].fg);
}

#[test]
fn to_latest_only_appears_away_from_bottom_and_click_returns_immediately() {
    let mut app = App::default();
    app.entries
        .push(Entry::new(EntryKind::Assistant, "polaris", &["short"]));
    assert!(
        !draw(&app, 80, 24)
            .0
            .targets
            .iter()
            .any(|t| t.action == HitAction::ToLatest)
    );
    app.entries.clear();
    let lines = (0..80).map(|i| format!("line {i}")).collect::<Vec<_>>();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris",
        &lines.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    app.scroll = 20;
    let (map, _) = draw(&app, 80, 24);
    let target = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::ToLatest)
        .unwrap();
    let mut controller = MouseController::default();
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Down(MouseButton::Left),
        target.rect.x,
        target.rect.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Up(MouseButton::Left),
        target.rect.x,
        target.rect.y,
    );
    assert_eq!(app.scroll, 0);
    let (map, buffer) = draw(&app, 80, 24);
    assert!(!map.targets.iter().any(|t| t.action == HitAction::ToLatest));
    assert!(
        buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("line 79")
    );
}

#[test]
fn slash_typing_colors_only_known_tokens_and_unselected_suggestions_stay_neutral() {
    let mut app = App::default();
    app.input
        .set("/not-a-command /pl /plan /goal https://x/path");
    let (map, buffer) = draw(&app, 120, 40);
    let region = map.region(&TextRegionId::Composer).unwrap();
    for token in ["/not-a-command", "/pl ", "https://"] {
        let offset = app.input.text.find(token).unwrap();
        let cell = region
            .cells
            .iter()
            .find(|c| c.byte_offset == offset)
            .unwrap();
        assert_eq!(
            buffer[(cell.x, cell.y)].fg,
            Color::Rgb(232, 232, 232),
            "{token}"
        );
    }
    for token in ["/plan", "/goal"] {
        let offset = app.input.text.find(token).unwrap();
        let cell = region
            .cells
            .iter()
            .find(|c| c.byte_offset == offset)
            .unwrap();
        assert_eq!(buffer[(cell.x, cell.y)].fg, Color::Rgb(118, 185, 0));
    }
    app.input.set("/");
    let (map, buffer) = draw(&app, 120, 40);
    for target in map
        .targets
        .iter()
        .filter(|t| matches!(t.action,HitAction::Suggestion(i) if i>0))
    {
        assert_eq!(
            buffer[(target.rect.x, target.rect.y)].fg,
            Color::Rgb(133, 133, 133)
        );
        assert_eq!(
            buffer[(target.rect.x, target.rect.y)].bg,
            Color::Rgb(0, 0, 0)
        );
    }
}

#[test]
fn conversation_selection_survives_scroll_with_global_source_offsets() {
    let mut app = App::default();
    let lines = (0..60).map(|i| format!("line {i}")).collect::<Vec<_>>();
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "polaris",
        &lines.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    app.scroll = usize::MAX;
    let (first, _) = draw(&app, 80, 24);
    let region = first.region(&TextRegionId::Conversation).unwrap();
    let start = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let mut controller = MouseController::default();
    mouse(
        &mut controller,
        &mut app,
        &first,
        MouseEventKind::Down(MouseButton::Left),
        start.x,
        start.y,
    );
    app.scroll = 0;
    let (last, _) = draw(&app, 80, 24);
    let region = last.region(&TextRegionId::Conversation).unwrap();
    let end = region
        .cells
        .iter()
        .find(|c| c.byte_offset == region.source.len())
        .unwrap();
    mouse(
        &mut controller,
        &mut app,
        &last,
        MouseEventKind::Drag(MouseButton::Left),
        end.x,
        end.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &last,
        MouseEventKind::Up(MouseButton::Left),
        end.x,
        end.y,
    );
    assert_eq!(selection_text(&app, &last), lines.join("\n"));
}

#[test]
fn wrapped_transcript_cells_match_real_text_with_tabs_and_wide_unicode() {
    let mut app = App::default();
    let prompt = "Long words near the wrapping boundary should stay aligned. \t漢字 and emoji 🙂 with another sentence crossing a line.";
    app.input.set(prompt);
    app.submit();
    app.working = false;
    let (map, buffer) = draw(&app, 60, 30);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    assert_eq!(region.source.as_ref(), prompt);
    for cell in &region.cells {
        if cell.byte_offset >= prompt.len() || cell.x >= region.rect.right() {
            continue;
        }
        let source = prompt[cell.byte_offset..].chars().next().unwrap();
        let shown = buffer[(cell.x, cell.y)].symbol();
        // End-insertion markers occupy blank padding at the right of a row.
        if shown.trim().is_empty() {
            continue;
        }
        assert_eq!(
            shown.chars().next().unwrap(),
            source,
            "cell ({},{}) mapped wrong source byte {}",
            cell.x,
            cell.y,
            cell.byte_offset
        );
    }
    let start = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let end = region
        .cells
        .iter()
        .find(|c| c.byte_offset == prompt.len())
        .unwrap();
    let mut controller = MouseController::default();
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Down(MouseButton::Left),
        start.x,
        start.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Drag(MouseButton::Left),
        end.x,
        end.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Up(MouseButton::Left),
        end.x,
        end.y,
    );
    assert_eq!(selection_text(&app, &map), prompt);
}

#[test]
fn exact_width_prompt_can_be_selected_through_its_last_character() {
    let mut app = App::default();
    let prompt = "x".repeat(54); // Two display padding columns + 54 source columns.
    app.input.set(&prompt);
    app.submit();
    app.working = false;
    let (map, _) = draw(&app, 60, 24);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    let start = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let end = region
        .cells
        .iter()
        .find(|c| c.byte_offset == prompt.len())
        .unwrap();
    let mut controller = MouseController::default();
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Down(MouseButton::Left),
        start.x,
        start.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Drag(MouseButton::Left),
        end.x,
        end.y,
    );
    mouse(
        &mut controller,
        &mut app,
        &map,
        MouseEventKind::Up(MouseButton::Left),
        end.x,
        end.y,
    );
    assert_eq!(selection_text(&app, &map), prompt);
}

#[test]
fn selected_sent_attachment_preserves_cyan_with_matching_copy_tint() {
    let mut app = App::default();
    app.paste(&"x".repeat(501));
    app.submit();
    app.working = false;
    let (map, before) = draw(&app, 80, 24);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: region.source.len(),
    });
    let (_, after) = draw(&app, 80, 24);
    for cell in region
        .cells
        .iter()
        .filter(|c| c.byte_offset < region.source.len())
    {
        assert_eq!(after[(cell.x, cell.y)].fg, before[(cell.x, cell.y)].fg);
        assert_eq!(before[(cell.x, cell.y)].bg, Color::Rgb(20, 20, 20));
        assert_eq!(
            after[(cell.x, cell.y)].bg,
            selection::background(app.theme, &before[(cell.x, cell.y)])
        );
    }
    for mode in [
        polaris::theme::ThemeMode::Dark,
        polaris::theme::ThemeMode::Terminal,
    ] {
        app.theme.mode = mode;
        app.theme.color = false;
        let (_, buffer) = draw(&app, 80, 24);
        assert!(
            buffer
                .content
                .iter()
                .all(|c| c.bg == Color::Reset && c.fg == Color::Reset)
        );
    }
}

#[test]
fn command_suggestions_have_one_slash_and_a_selection_pointer() {
    let mut app = App::default();
    app.input.set("/");
    let (map, buffer) = draw(&app, 120, 40);
    for target in map
        .targets
        .iter()
        .filter(|t| matches!(t.action, HitAction::Suggestion(_)))
    {
        let HitAction::Suggestion(index) = target.action else {
            unreachable!()
        };
        let text = (target.rect.x..target.rect.right())
            .map(|x| buffer[(x, target.rect.y)].symbol())
            .collect::<String>();
        let prefix = if index == app.selected { "> /" } else { "  /" };
        assert!(text.starts_with(prefix), "{text:?}");
        assert!(!text.contains("/ /"), "{text:?}");
    }
}

#[test]
fn to_latest_is_centered_with_dark_gray_normal_hover_and_leave_backgrounds() {
    for width in [60, 80, 120] {
        let mut app = App::default();
        let lines = (0..80).map(|i| format!("line {i}")).collect::<Vec<_>>();
        app.entries.push(Entry::new(
            EntryKind::Assistant,
            "polaris",
            &lines.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
        app.scroll = 20;
        let (map, before) = draw(&app, width, 30);
        let rect = map
            .targets
            .iter()
            .find(|t| t.action == HitAction::ToLatest)
            .unwrap()
            .rect;
        assert!((rect.x as i32 * 2 + rect.width as i32 - width as i32).abs() <= 1);
        assert!((rect.x..rect.right()).all(|x| before[(x, rect.y)].bg == Color::Rgb(38, 38, 38)));
        let mut controller = MouseController::default();
        // Feed the actual SGR all-motion transport form into the decoder.
        let mut decoder = polaris::input::KeyboardDecoder::default();
        for ch in format!("\x1b[<35;{};{}M", rect.x + 1, rect.y + 1).chars() {
            decoder.feed(crossterm::event::KeyEvent::new(
                if ch == '\x1b' {
                    crossterm::event::KeyCode::Esc
                } else {
                    crossterm::event::KeyCode::Char(ch)
                },
                KeyModifiers::NONE,
            ));
        }
        let events = decoder.take_mouse();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, MouseEventKind::Moved);
        controller.event(events[0], &map, &mut app);
        let (_, hover) = draw(&app, width, 30);
        assert!((rect.x..rect.right()).all(|x| hover[(x, rect.y)].bg == Color::Rgb(55, 55, 55)));
        assert_eq!(app.scroll, 20);
        mouse(&mut controller, &mut app, &map, MouseEventKind::Moved, 0, 0);
        let (_, leave) = draw(&app, width, 30);
        assert!((rect.x..rect.right()).all(|x| leave[(x, rect.y)].bg == Color::Rgb(38, 38, 38)));
        app.theme.color = false;
        let (_, plain) = draw(&app, width, 30);
        assert!(plain.content.iter().all(|c| c.bg == Color::Reset));
    }
}

#[test]
fn mode_on_footer_labels_preserve_colors_and_click_targets() {
    for width in [60, 80, 120] {
        let mut app = App::default();
        for (mode, label, color) in [
            (0, "auto mode on", Color::Rgb(245, 194, 231)),
            (1, "plan mode on", Color::Rgb(249, 226, 175)),
        ] {
            app.mode = mode;
            let (map, buffer) = draw(&app, width, 30);
            let rect = map
                .targets
                .iter()
                .find(|t| t.action == HitAction::FooterMode)
                .unwrap()
                .rect;
            let text = (rect.x..rect.right())
                .map(|x| buffer[(x, rect.y)].symbol())
                .collect::<String>();
            assert_eq!(text, format!("[ {label} ]"));
            assert_eq!(buffer[(rect.x + 2, rect.y)].fg, color);
            let mut controller = MouseController::default();
            mouse(
                &mut controller,
                &mut app,
                &map,
                MouseEventKind::Down(MouseButton::Left),
                rect.x,
                rect.y,
            );
            mouse(
                &mut controller,
                &mut app,
                &map,
                MouseEventKind::Up(MouseButton::Left),
                rect.x,
                rect.y,
            );
            assert_eq!(app.mode, 1 - mode);
        }
    }
}
