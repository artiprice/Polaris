use polaris::{
    app::{App, Entry, EntryKind},
    code_view::CodeBlock,
    interaction::{TextRegionId, UiMap, ViewSelection},
    runtime::selection_text,
    theme::{Theme, ThemeMode},
    ui, work_summary,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use std::time::{Duration, Instant};
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (map, t.backend().buffer().clone())
}
#[test]
fn completed_response_records_measured_duration_and_real_clock_without_repeating() {
    let mut app = App::default();
    app.input.set("Task");
    app.submit();
    app.working_since = Some(Instant::now() - Duration::from_secs(216));
    app.ticks = 17;
    app.tick();
    assert!(!app.working);
    let summary = app.entries.last().unwrap();
    assert_eq!(summary.kind, EntryKind::Completion);
    assert!(summary.lines[0].starts_with("Worked for 3m 36s · "));
    let clock = summary.lines[0].split(" · ").last().unwrap();
    let parts = clock.split(':').collect::<Vec<_>>();
    assert_eq!(parts.len(), 2);
    assert!(parts[0].parse::<u8>().unwrap() < 24);
    assert!(
        parts[1]
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<u8>()
            .unwrap()
            < 60
    );
    let count = app.entries.len();
    app.tick();
    assert_eq!(app.entries.len(), count);
    let (_, b) = draw(&app, 120, 40);
    let p = b
        .content
        .iter()
        .find(|c| c.symbol() == "W" && c.fg == Color::Rgb(133, 133, 133));
    assert!(p.is_some());
    let mut app = App::default();
    app.working = true;
    app.ticks = 17;
    app.tick();
    assert!(
        !app.entries.iter().any(|e| e.kind == EntryKind::Completion),
        "unknown start time must not invent a duration"
    );
    assert_eq!(work_summary::elapsed_label(Duration::from_secs(0)), "0s");
    assert_eq!(
        work_summary::elapsed_label(Duration::from_secs(3661)),
        "1h 01m 01s"
    );
}
#[test]
fn severity_is_explicit_in_titles_and_body_color_and_survives_monochrome() {
    let mut app = App::default();
    app.load_scenario("messages").unwrap();
    let warning = app
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::Warning)
        .unwrap();
    let error = app
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::Error)
        .unwrap();
    for (entry, word, fg) in [
        (warning, "Warning", Color::Rgb(229, 180, 84)),
        (error, "Error", Color::Rgb(172, 66, 66)),
    ] {
        let lines = ui::entry_lines(entry, &app, false);
        assert!(lines[0].to_string().contains(word));
        assert_eq!(lines[1].style.fg, Some(fg));
    }
    let ordinary = ui::entry_lines(&app.entries[0], &app, false);
    assert!(!ordinary[0].to_string().contains("Warning"));
    app.theme.color = false;
    let (_, b) = draw(&app, 100, 35);
    let text = b.content.iter().map(|c| c.symbol()).collect::<String>();
    assert!(text.contains("Warning ·") && text.contains("Error ·"));
    assert!(
        b.content
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
}
#[test]
fn numbered_code_keeps_unicode_tabs_and_exact_source_copy_through_wrap_and_scroll() {
    use unicode_segmentation::UnicodeSegmentation;
    let mut app = App::default();
    let code = [
        "pub fn café(input: &str) -> Result<(), Error> {",
        "\tlet message = \"界 👩‍💻 and a long string with several ordinary words that wraps naturally\";",
        "    // keep original code indentation",
        "    Ok(())",
        "}",
    ];
    let mut lines = vec!["\x60\x60\x60rust"];
    lines.extend(code);
    lines.push("\x60\x60\x60");
    app.entries
        .push(Entry::new(EntryKind::Assistant, "", &lines));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        for scroll in [0, usize::MAX] {
            app.scroll = scroll;
            let (map, b) = draw(&app, w, h);
            let r = map.region(&TextRegionId::Conversation).unwrap();
            assert_eq!(r.source.as_ref(), code.join("\n"));
            for c in &r.cells {
                let Some(g) = r.source[c.byte_offset..]
                    .graphemes(true)
                    .next()
                    .filter(|g| !g.chars().all(char::is_whitespace))
                else {
                    continue;
                };
                if r.cells
                    .iter()
                    .any(|other| other.byte_offset == c.byte_offset && other.y > c.y)
                {
                    continue;
                }
                assert_eq!(
                    b[(c.x, c.y)].symbol(),
                    g,
                    "bad source glyph at {w} columns byte {}",
                    c.byte_offset
                );
            }
            app.view_selection = Some(ViewSelection {
                region: TextRegionId::Conversation,
                anchor: 0,
                end: r.source.len(),
            });
            assert_eq!(selection_text(&app, &map), code.join("\n"));
            app.view_selection = None;
        }
    }
    let mut block = CodeBlock::new("rust");
    let row = block.row("pub fn example() { let value = 42; }", app.theme);
    assert_eq!(row.prefix, "     1 ");
    assert!(
        row.line
            .spans
            .iter()
            .any(|s| s.content == "pub" && s.style.fg == Some(Color::Rgb(203, 166, 247)))
    );
    assert!(
        row.line
            .spans
            .iter()
            .any(|s| s.content == "42" && s.style.fg == Some(Color::Rgb(249, 226, 175)))
    );
}
#[test]
fn diff_backgrounds_fill_wrapped_rows_and_selected_source_preserves_unselected_padding() {
    let mut app = App::default();
    let removed = "-let previous_value = \"a deliberately long removed line with café and several wrapping words\";";
    let added = "+let current_value = \"a deliberately long added line with café and several wrapping words\";";
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "",
        &[
            "\x60\x60\x60diff",
            "@@ -4,1 +4,1 @@",
            removed,
            added,
            "\x60\x60\x60",
        ],
    ));
    for w in [60, 120] {
        let (map, b) = draw(&app, w, 40);
        let r = map.region(&TextRegionId::Conversation).unwrap();
        for (needle, bg) in [
            (removed, Color::Rgb(50, 7, 9)),
            (added, Color::Rgb(7, 35, 12)),
        ] {
            let start = r.source.find(needle).unwrap();
            let end = start + needle.len();
            let rows = r
                .cells
                .iter()
                .filter(|c| c.byte_offset >= start && c.byte_offset < end)
                .map(|c| c.y)
                .collect::<std::collections::HashSet<_>>();
            assert!(!rows.is_empty());
            for y in rows {
                assert_eq!(
                    b[(r.rect.right() - 1, y)].bg,
                    bg,
                    "row fill missing at width {w}"
                );
            }
        }
        let start = r.source.find(removed).unwrap();
        app.view_selection = Some(ViewSelection {
            region: TextRegionId::Conversation,
            anchor: start,
            end: start + removed.len(),
        });
        let (_, b) = draw(&app, w, 40);
        let cell = r.cells.iter().find(|c| c.byte_offset == start + 1).unwrap();
        assert_eq!(b[(cell.x, cell.y)].bg, Color::Rgb(96, 55, 80));
        assert_eq!(selection_text(&app, &map), removed);
        app.view_selection = None;
    }
}
#[test]
fn unified_diff_numbers_use_old_and_new_positions_with_color_safe_fallbacks() {
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for rgb in [true, false] {
            for color in [true, false] {
                let theme = Theme { mode, rgb, color };
                let mut code = CodeBlock::new("diff");
                let header = code.row("--- a/source.rs", theme);
                assert_eq!(header.line.style.bg, theme.base().bg);
                code.row("@@ -10,2 +20,2 @@", theme);
                assert_eq!(code.row("-let old = 1;", theme).prefix, "    10 ");
                assert_eq!(code.row("+let new = 2;", theme).prefix, "    20 ");
                assert_eq!(code.row(" context", theme).prefix, "    21 ");
                if !color {
                    assert_eq!(theme.diff_added().bg, None);
                    assert_eq!(theme.diff_removed().bg, None);
                } else {
                    assert_ne!(theme.diff_added().bg, theme.diff_removed().bg);
                }
            }
        }
    }
}
#[test]
fn code_strings_comments_and_unknown_languages_preserve_content_without_reverse_video() {
    let theme = Theme::default();
    let mut code = CodeBlock::new("rust");
    for raw in [
        "let value = \"café \\\"quoted\\\"\"; // note",
        "let lifetime: &'a str = text;",
        "\t// 界 👩‍💻",
        "",
    ] {
        let row = code.row(raw, theme);
        assert!(
            row.line
                .spans
                .iter()
                .all(|s| !s.style.add_modifier.contains(Modifier::REVERSED))
        );
    }
    let mut diff = CodeBlock::new("diff");
    let row = diff.row("+// comment", theme);
    let comment = row.line.spans.last().unwrap();
    assert_eq!(
        comment.style.bg, None,
        "comment styling must inherit the diff surface"
    );
    let mut unknown = CodeBlock::new("unknown");
    let row = unknown.row("raw **not markdown**", theme);
    assert!(row.line.to_string().ends_with("raw **not markdown**"));
}

#[test]
fn errors_use_reference_muted_red_without_bold_in_titles_bodies_and_fallbacks() {
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for rgb in [true, false] {
            for color in [true, false] {
                let mut app = App::default();
                app.theme.mode = mode;
                app.theme.rgb = rgb;
                app.theme.color = color;
                app.entries.push(Entry::new(
                    EntryKind::Error,
                    "Capacity",
                    &["Selected model is at capacity. Please try a different model."],
                ));
                let expected = if !color {
                    Color::Reset
                } else if rgb && mode == ThemeMode::Dark {
                    Color::Rgb(172, 66, 66)
                } else {
                    Color::Red
                };
                let style = app.theme.error();
                assert_eq!(style.fg, Some(expected));
                assert!(!style.add_modifier.contains(Modifier::BOLD));
                let lines = ui::entry_lines(&app.entries[0], &app, false);
                assert!(lines[0].to_string().contains("Error · Capacity"));
                assert!(!lines[1].style.add_modifier.contains(Modifier::BOLD));
                for span in &lines[0].spans {
                    assert!(!span.style.add_modifier.contains(Modifier::BOLD));
                }
                let (map, b) = draw(&app, 100, 30);
                let region = map.region(&TextRegionId::Entry(0)).unwrap();
                let first = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
                for y in [first.y - 1, first.y] {
                    for x in region.rect.x..region.rect.right() {
                        let cell = &b[(x, y)];
                        if !cell.symbol().trim().is_empty() {
                            assert_eq!(cell.fg, expected);
                            assert!(
                                !cell.modifier.contains(Modifier::BOLD),
                                "bold error at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }
}
