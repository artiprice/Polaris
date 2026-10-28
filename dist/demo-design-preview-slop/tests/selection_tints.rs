use polaris::{
    app::{App, Entry, EntryKind},
    interaction::{TextRegionId, UiMap, ViewSelection},
    runtime::selection_text,
    theme::{Theme, ThemeMode},
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::{Buffer, Cell},
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;

fn draw(app: &App, width: u16) -> (UiMap, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    let mut map = UiMap::default();
    // Capture the completed frame: Ratatui does not send wide-glyph continuation
    // cells to Backend::draw, since the terminal paints them with the leading glyph.
    let buffer = terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap()
        .buffer
        .clone();
    (map, buffer)
}
fn cell<'a>(map: &UiMap, b: &'a Buffer, region: &TextRegionId, offset: usize) -> &'a Cell {
    let c = map
        .region(region)
        .unwrap()
        .cells
        .iter()
        .find(|c| c.byte_offset == offset)
        .unwrap();
    &b[(c.x, c.y)]
}
fn tint_at(
    map: &UiMap,
    before: &Buffer,
    after: &Buffer,
    id: &TextRegionId,
    offset: usize,
    bg: Color,
) {
    let a = cell(map, before, id, offset);
    let b = cell(map, after, id, offset);
    assert_eq!(b.bg, bg, "source offset {offset}, original {:?}", a.fg);
    assert_eq!(
        a.fg, b.fg,
        "selection changed source foreground at {offset}"
    );
    assert_eq!(
        a.modifier, b.modifier,
        "selection changed source emphasis at {offset}"
    );
    assert_eq!(a.symbol(), b.symbol());
}

#[test]
fn exact_tints_blend_thirty_percent_source_color_and_spaces_keep_neutral_shade() {
    let theme = Theme::default();
    for (fg, bg, expected) in [
        ((232, 232, 232), (0, 0, 0), (70, 70, 70)),
        ((249, 226, 175), (0, 0, 0), (75, 68, 53)),
        ((118, 185, 0), (0, 0, 0), (35, 56, 0)),
        ((137, 180, 250), (0, 0, 0), (41, 54, 75)),
        ((172, 66, 66), (0, 0, 0), (52, 20, 20)),
        ((130, 184, 200), (55, 55, 55), (78, 94, 99)),
        ((232, 232, 232), (55, 55, 55), (108, 108, 108)),
        ((203, 166, 247), (50, 7, 9), (96, 55, 80)),
    ] {
        let rgb = |(r, g, b)| Color::Rgb(r, g, b);
        let style = theme.copy_selection(rgb(fg), rgb(bg), false);
        assert_eq!(style.bg, Some(rgb(expected)));
        // Every fixture must visibly separate the highlight from its original
        // surface; this catches an accidentally reduced tint strength.
        let separation = u16::from(expected.0.abs_diff(bg.0))
            + u16::from(expected.1.abs_diff(bg.1))
            + u16::from(expected.2.abs_diff(bg.2));
        assert!(
            separation >= 60,
            "selection too close to the original background"
        );
        assert_eq!(style.fg, None);
        assert!(style.add_modifier.is_empty() && style.sub_modifier.is_empty());
        assert_eq!(
            theme.copy_selection(rgb(fg), rgb(bg), true).bg,
            Some(match bg {
                (55, 55, 55) => Color::Rgb(108, 108, 108),
                (50, 7, 9) => Color::Rgb(105, 75, 76),
                _ => Color::Rgb(70, 70, 70),
            })
        );
    }
}

#[test]
fn composer_mouse_and_keyboard_selection_preserve_tokens_links_tabs_and_wide_cells() {
    let text = "plain @main.rs /plan $websearch https://example.com\n界\tend e\u{301}";
    for width in [60, 80, 120] {
        for keyboard in [false, true] {
            let mut app = App::default();
            app.input.set(text);
            let (map, before) = draw(&app, width);
            if keyboard {
                app.input.anchor = Some(0);
            } else {
                app.view_selection = Some(ViewSelection {
                    region: TextRegionId::Composer,
                    anchor: text.len(),
                    end: 0,
                });
            }
            let (_, after) = draw(&app, width);
            let id = TextRegionId::Composer;
            for (offset, bg) in [
                (0, Color::Rgb(70, 70, 70)),
                (text.find('@').unwrap(), Color::Rgb(75, 68, 53)),
                (text.find("/plan").unwrap(), Color::Rgb(35, 56, 0)),
                (text.find('$').unwrap(), Color::Rgb(41, 54, 75)),
                (text.find(' ').unwrap(), Color::Rgb(70, 70, 70)),
                (text.find("https").unwrap(), Color::Rgb(70, 70, 70)),
            ] {
                tint_at(&map, &before, &after, &id, offset, bg);
            }
            let c = map
                .region(&id)
                .unwrap()
                .cells
                .iter()
                .find(|c| c.byte_offset == text.find('界').unwrap())
                .unwrap();
            assert_eq!(after[(c.x, c.y)].bg, Color::Rgb(70, 70, 70));
            assert_eq!(after[(c.x + 1, c.y)].bg, after[(c.x, c.y)].bg);
            let tab = map
                .region(&id)
                .unwrap()
                .cells
                .iter()
                .find(|c| c.byte_offset == text.find('\t').unwrap())
                .unwrap();
            for x in tab.x..tab.x + 2 {
                assert_eq!(after[(x, tab.y)].bg, Color::Rgb(70, 70, 70));
            }
            let (_, repeated) = draw(&app, width);
            assert_eq!(repeated, after, "tint accumulated across frames");
            app.input.anchor = None;
            app.view_selection = None;
            assert_eq!(
                draw(&app, width).1,
                before,
                "deselect did not restore exact source styling"
            );
            assert!(
                cell(&map, &after, &id, text.find("https").unwrap())
                    .modifier
                    .contains(Modifier::UNDERLINED)
            );
        }
    }
}

#[test]
fn history_tints_follow_message_severity_syntax_and_existing_prompt_and_diff_surfaces() {
    let mut app = App::default();
    app.input.set("sent @file.rs /plan $websearch");
    app.submit();
    app.working = false;
    app.entries.push(Entry::new(
        EntryKind::Warning,
        "Notice",
        &["warning message"],
    ));
    app.entries
        .push(Entry::new(EntryKind::Error, "Error", &["error message"]));
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "",
        &[
            "ordinary @file.rs /plan $websearch https://example.com",
            "\x60\x60\x60diff",
            "-let old = 2;",
            "+let new = 3;",
            "\x60\x60\x60",
        ],
    ));
    let (map, before) = draw(&app, 120);
    let id = TextRegionId::Conversation;
    let source = map.region(&id).unwrap().source.clone();
    app.view_selection = Some(ViewSelection {
        region: id.clone(),
        anchor: 0,
        end: source.len(),
    });
    let (selected_map, after) = draw(&app, 120);
    assert_eq!(selection_text(&app, &selected_map), source.as_ref());
    for (needle, bg) in [
        ("sent", Color::Rgb(84, 84, 84)),
        ("@file.rs", Color::Rgb(89, 82, 67)),
        ("warning message", Color::Rgb(69, 54, 25)),
        ("error message", Color::Rgb(52, 20, 20)),
        ("ordinary", Color::Rgb(70, 70, 70)),
        ("/plan", Color::Rgb(49, 70, 14)),
        ("$websearch", Color::Rgb(55, 68, 89)),
        ("-let", Color::Rgb(109, 50, 51)),
        ("+let", Color::Rgb(48, 86, 39)),
    ] {
        tint_at(&map, &before, &after, &id, source.find(needle).unwrap(), bg);
    }
    // Syntax takes priority within the diff: the purple keyword keeps its hue.
    tint_at(
        &map,
        &before,
        &after,
        &id,
        source.find("-let").unwrap() + 1,
        Color::Rgb(96, 55, 80),
    );
    for (a, b) in before.content.iter().zip(&after.content) {
        assert_eq!(a.fg, b.fg);
        assert_eq!(a.modifier, b.modifier);
    }
    app.view_selection = None;
    assert_eq!(draw(&app, 120).1, before);
}

#[test]
fn attachment_preview_whitespace_and_unicode_copy_remain_exact() {
    let mut app = App::default();
    let payload = "white 界\ttext\n".repeat(40);
    app.paste(&payload);
    let attachment = app.input.spans[0].attachment.id;
    app.open_attachment(attachment);
    let id = TextRegionId::AttachmentPreview(attachment);
    let (map, before) = draw(&app, 80);
    let r = map.region(&id).unwrap();
    app.view_selection = Some(ViewSelection {
        region: id.clone(),
        anchor: 0,
        end: r.source.len(),
    });
    let (selected_map, after) = draw(&app, 80);
    assert_eq!(selection_text(&app, &selected_map), payload);
    for c in r.cells.iter().filter(|c| c.byte_offset < r.source.len()) {
        // Row-end insertion markers may overlap the next line's source offset.
        if before[(c.x, c.y)].symbol().trim().is_empty()
            && !r.source[c.byte_offset..].starts_with([' ', '\t'])
        {
            continue;
        }
        let expected = Color::Rgb(70, 70, 70);
        assert_eq!(after[(c.x, c.y)].bg, expected);
        let width = UnicodeWidthStr::width(before[(c.x, c.y)].symbol()).max(1) as u16;
        for x in c.x..c.x + width {
            assert_eq!(after[(x, c.y)].bg, expected);
        }
        assert_eq!(after[(c.x, c.y)].fg, before[(c.x, c.y)].fg);
    }
}

#[test]
fn ansi_terminal_and_no_color_fallbacks_do_not_guess_host_foregrounds() {
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for rgb in [false, true] {
            for color in [false, true] {
                if mode == ThemeMode::Dark && rgb && color {
                    continue;
                }
                let mut app = App::default();
                app.theme = Theme { mode, rgb, color };
                app.input.set("white @file.rs /plan $websearch");
                let (map, before) = draw(&app, 80);
                app.input.anchor = Some(0);
                let (_, after) = draw(&app, 80);
                let r = map.region(&TextRegionId::Composer).unwrap();
                for c in r
                    .cells
                    .iter()
                    .filter(|c| c.byte_offset < app.input.text.len())
                {
                    let a = &before[(c.x, c.y)];
                    let b = &after[(c.x, c.y)];
                    assert_eq!(a.fg, b.fg);
                    assert_eq!(b.bg, if color { Color::Indexed(237) } else { a.bg });
                    assert_eq!(b.modifier.contains(Modifier::BOLD), !color);
                    assert!(!b.modifier.contains(Modifier::REVERSED));
                }
            }
        }
    }
}
