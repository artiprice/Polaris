#[path = "support/selection.rs"]
mod selection;
use polaris::{
    app::{App, Entry, EntryKind},
    completion::FileIndex,
    interaction::{HitAction, TextRegionId, UiMap},
    theme::ThemeMode,
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};
use std::time::{Duration, Instant};

fn draw(terminal: &mut Terminal<TestBackend>, app: &App) -> (UiMap, Buffer) {
    let mut map = UiMap::default();
    let mut rendered = terminal.backend().buffer().clone();
    terminal
        .draw(|f| {
            map = ui::render_interactive(f, app);
            // Inspect widget output before Terminal::diff normalizes invisible wide-cell tails.
            rendered = f.buffer_mut().clone();
        })
        .unwrap();
    (map, rendered)
}

#[test]
fn completions_erase_the_entire_width_and_restore_history_after_dismissal() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("overlay-files-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for name in ["selection.txt", "selection2.txt", "selection3.txt"] {
        std::fs::write(root.join(name), "fixture").unwrap();
    }
    let mut index = FileIndex::start(root.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !index.is_ready() {
        index.poll();
        assert!(Instant::now() < deadline, "file index timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut app = App::default();
    app.file_index = Some(index);
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        app.theme.mode = mode;
        for (width, height) in [(60, 20), (80, 30), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for input in ["/", "Please /", "@sel"] {
                app.entries.clear();
                app.entries.push(Entry::new(EntryKind::Assistant, "", &[]));
                app.input.set(input);
                app.menu_dismissed = false;
                let (clean_map, clean) = draw(&mut terminal, &app);
                let rows: Vec<_> = clean_map
                    .targets
                    .iter()
                    .filter(|t| matches!(t.action, HitAction::Suggestion(_)))
                    .map(|t| t.rect.y)
                    .collect();
                assert!(!rows.is_empty(), "no completion rows for {input}");
                let noise = "BACKGROUND_SENTINEL ".repeat(300);
                app.entries[0] = Entry::new(EntryKind::Assistant, "", &[&noise]);
                let (map, covered) = draw(&mut terminal, &app);
                let margin_y = rows[0] - 1;
                assert!(
                    (0..width).all(|x| covered[(x, margin_y)].symbol() == " "),
                    "completion top margin is not blank"
                );
                assert!(
                    map.text_regions
                        .iter()
                        .all(|r| r.cells.iter().all(|c| c.y != margin_y)),
                    "margin source is still selectable"
                );
                for y in &rows {
                    for x in 0..width {
                        assert_eq!(
                            covered[(x, *y)],
                            clean[(x, *y)],
                            "transcript leaked through {input} at {x},{y}, width {width}"
                        );
                    }
                    assert!(
                        map.text_regions
                            .iter()
                            .all(|r| r.cells.iter().all(|c| c.y != *y)),
                        "hidden source still selectable"
                    );
                }
                app.menu_dismissed = true;
                let (restored, b) = draw(&mut terminal, &app);
                assert!(
                    !restored
                        .targets
                        .iter()
                        .any(|t| matches!(t.action, HitAction::Suggestion(_)))
                );
                assert!(
                    restored
                        .region(&TextRegionId::Conversation)
                        .unwrap()
                        .cells
                        .iter()
                        .any(|c| rows.contains(&c.y)),
                    "missing restored cells for {input} at {width}x{height}, rows {rows:?}"
                );
                assert!(rows.iter().any(|y| (0..width).any(|x| b[(x,*y)].symbol() != clean[(x,*y)].symbol())), "history was not restored");
            }
        }
    }
    for name in ["selection.txt", "selection2.txt", "selection3.txt"] {
        std::fs::remove_file(root.join(name)).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn collapsed_tools_show_only_gold_title_and_expand_hint() {
    let app = App::default();
    let tool = Entry::new(
        EntryKind::Tool,
        "Read · src/validation.rs · complete [demo]",
        &["private detail one", "private detail two"],
    );
    let collapsed = ui::entry_lines(&tool, &app, false);
    let visible = collapsed
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(visible.contains("Read · src/validation.rs"));
    assert!(visible.contains("Ctrl+O to expand"));
    assert!(!visible.contains("private detail"));
    assert_eq!(collapsed.len(), 3, "title, hint, and entry spacer only");
    assert!(
        collapsed[0]
            .spans
            .iter()
            .all(|s| s.style.fg == Some(Color::Rgb(249, 226, 175)))
    );
    let expanded = ui::entry_lines(&tool, &app, true)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded.contains("private detail one\n  private detail two"));
    assert!(!expanded.contains("Ctrl+O to expand"));
    let mut app = app;
    app.entries.push(tool);
    app.entries
        .push(Entry::new(EntryKind::Assistant, "", &["Following reply."]));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    let (map, b) = draw(&mut terminal, &app);
    assert!(
        map.region(&TextRegionId::Entry(0))
            .unwrap()
            .source
            .is_empty()
    );
    let reply = map.region(&TextRegionId::Entry(1)).unwrap();
    let p = reply.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    assert_eq!(
        b[(p.x, p.y)].symbol(),
        "F",
        "collapsed tool shifted reply mapping"
    );
    app.key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('o'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert_eq!(app.screen, polaris::app::Screen::Home);
    let (_, expanded_buffer) = draw(&mut terminal, &app);
    assert!(
        expanded_buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("private detail one")
    );
}

#[test]
fn copy_selection_shades_only_selected_glyphs_in_all_text_surfaces() {
    use polaris::interaction::ViewSelection;
    use ratatui::style::Modifier;
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for rgb in [true, false] {
            for color in [true, false] {
                for surface in 0..3 {
                    let mut app = App::default();
                    app.theme.mode = mode;
                    app.theme.rgb = rgb;
                    app.theme.color = color;
                    let text = "alpha 界 https://host.com omega";
                    let id = match surface {
                        0 => {
                            app.input.set(text);
                            TextRegionId::Composer
                        }
                        1 => {
                            app.entries
                                .push(Entry::new(EntryKind::Assistant, "", &[text]));
                            TextRegionId::Conversation
                        }
                        _ => {
                            app.paste(&format!("{text}\n").repeat(30));
                            let id = app.input.spans[0].attachment.id;
                            app.open_attachment(id);
                            TextRegionId::AttachmentPreview(id)
                        }
                    };
                    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
                    let (map, before) = draw(&mut terminal, &app);
                    let region = map.region(&id).unwrap();
                    let end = text.find(" omega").unwrap();
                    app.view_selection = Some(ViewSelection {
                        region: id.clone(),
                        anchor: 0,
                        end,
                    });
                    let (_, after) = draw(&mut terminal, &app);
                    let mut expected = std::collections::HashSet::new();
                    for c in region.cells.iter().filter(|c| c.byte_offset < end) {
                        // Insertion markers duplicate the next row's first source byte.
                        if before[(c.x, c.y)].symbol().trim().is_empty()
                            && !region.source[c.byte_offset..].starts_with(' ')
                        {
                            continue;
                        }
                        let width =
                            unicode_width::UnicodeWidthStr::width(before[(c.x, c.y)].symbol())
                                .max(1) as u16;
                        for x in c.x..c.x + width {
                            expected.insert((x, c.y));
                            assert_eq!(
                                after[(x, c.y)].bg,
                                selection::background(app.theme, &before[(c.x, c.y)]),
                                "surface {surface}, byte {}, rgb {rgb}, color {color}",
                                c.byte_offset
                            );
                        }
                    }
                    assert!(!expected.is_empty());
                    for y in 0..30 {
                        for x in 0..100 {
                            if !expected.contains(&(x, y)) {
                                assert_eq!(
                                    after[(x, y)].bg,
                                    before[(x, y)].bg,
                                    "unselected cell changed at {x},{y}, surface {surface}"
                                );
                            }
                            assert!(!after[(x, y)].modifier.contains(Modifier::REVERSED));
                        }
                    }
                    if surface < 2 {
                        let link = region
                            .cells
                            .iter()
                            .find(|c| c.byte_offset == text.find("https").unwrap())
                            .unwrap();
                        assert!(
                            after[(link.x, link.y)]
                                .modifier
                                .contains(Modifier::UNDERLINED)
                        );
                    }
                }
            }
        }
    }
}
