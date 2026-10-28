use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
#[path = "support/selection.rs"]
mod selection;
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
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
    widgets::{Paragraph, Wrap},
};
fn capture(app: &App, w: u16, h: u16) -> (Buffer, UiMap) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (t.backend().buffer().clone(), map)
}
fn text(b: &Buffer, r: Rect) -> String {
    (r.y..r.bottom())
        .map(|y| {
            (r.x..r.right())
                .map(|x| b[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn stars(b: &Buffer) -> Vec<(u16, u16)> {
    (0..b.area.height)
        .flat_map(|y| {
            (0..b.area.width).filter_map(move |x| (b[(x, y)].symbol() == "✦").then_some((x, y)))
        })
        .collect()
}
#[test]
fn centered_welcome_and_compact_header_have_exact_geometry_and_regular_version() {
    for (w, h) in [(60, 20), (80, 24), (100, 30), (120, 40), (160, 60)] {
        let app = App::default();
        let (b, map) = capture(&app, w, h);
        let col = ui::content_column(b.area);
        assert_eq!(col, Rect::new(2, 0, w - 4, h));
        assert_eq!(text(&b, Rect::new(0, 0, w, 2)).trim(), "");
        assert!(
            text(&b, Rect::new(col.x + 2, 2, col.width - 4, 1))
                .starts_with("✦ Polaris v0.1.0alpha")
        );
        assert!(!b[(col.x + 12, 2)].modifier.contains(Modifier::BOLD));
        assert_eq!(b[(col.x + 12, 2)].fg, Color::Rgb(133, 133, 133));
        let all = text(&b, b.area);
        assert!(!all.contains("Account not configured"));
        assert_eq!(stars(&b).len(), 2);
        let (hero_x, hero_y) = stars(&b)[1];
        assert_eq!(hero_x, col.x + col.width / 2 - 1);
        let grid = text(&b, Rect::new(hero_x - 4, hero_y - 2, 9, 5));
        assert_eq!(
            grid,
            "    ╷    \n    │    \n╶───✦───╴\n    │    \n    ╵    "
        );
        assert_eq!(b[(hero_x, hero_y)].fg, Color::Rgb(192, 229, 140));
        assert_eq!(b[(hero_x - 1, hero_y)].fg, Color::Rgb(118, 185, 0));
        assert_eq!(b[(hero_x - 4, hero_y)].fg, Color::Rgb(107, 107, 107));
        assert!(b.content.iter().all(|c| c.bg == Color::Rgb(0, 0, 0)));
        let r = map.region(&TextRegionId::Composer).unwrap().rect;
        assert_eq!(r.bottom(), h - 3);
        assert_eq!(text(&b, Rect::new(0, h - 2, w, 1)).trim(), "");
        assert!(all.lines().last().unwrap().contains("[ auto mode on ]"));
    }
}
#[test]
fn typing_submission_popups_previews_and_clear_preserve_brand_lifecycle() {
    let mut app = App::default();
    let (welcome, _) = capture(&app, 120, 40);
    app.input.set("Typing keeps the star");
    assert_eq!(stars(&capture(&app, 120, 40).0), stars(&welcome));
    app.submit();
    app.working = false;
    let (chat, map) = capture(&app, 120, 40);
    assert_eq!(stars(&chat).len(), 1);
    assert_eq!(map.region(&TextRegionId::Conversation).unwrap().rect.y, 4);
    for command in [
        "/model",
        "/status",
        "/stats",
        "/providers",
        "/preview",
        "/onboarding",
    ] {
        app.command(command);
        let (popup, map) = capture(&app, 120, 40);
        let sheet = ui::popup_area(&app, Rect::new(0, 0, 120, 40));
        for y in 0..sheet.y {
            for x in 0..120 {
                assert_eq!(
                    popup[(x, y)],
                    chat[(x, y)],
                    "{command} changed the uncovered header"
                );
            }
        }
        assert!(
            map.text_regions
                .iter()
                .all(|r| r.cells.iter().all(|c| !sheet.contains((c.x, c.y).into())))
        );
        app.close();
    }
    app.paste(&"long paste\n".repeat(60));
    let id = app.input.spans[0].attachment.id;
    app.open_attachment(id);
    assert_eq!(stars(&capture(&app, 120, 40).0), stars(&chat));
    app.close();
    app.command("/clear");
    assert_eq!(stars(&capture(&app, 120, 40).0), stars(&welcome));
}
#[test]
fn unicode_workspace_and_explicit_local_account_clip_without_leaving_the_column() {
    let mut app = App::default();
    app.account_email = Some("开发者👩‍💻é-account".repeat(20));
    app.workspace = "C:/项目/🦀/workspace/".repeat(20);
    for ascii in [false, true] {
        app.ascii = ascii;
        let (b, _) = capture(&app, 60, 20);
        let all = text(&b, b.area);
        assert!(all.contains("Polaris v0.1.0alpha"));
        let account = text(&b, Rect::new(6, 3, 50, 1));
        assert!(account.ends_with(if ascii { "..." } else { "…" }));
        for y in 0..20 {
            assert_eq!(text(&b, Rect::new(0, y, 2, 1)), "  ");
            assert_eq!(text(&b, Rect::new(58, y, 2, 1)), "  ");
        }
        assert!(app.account_email.as_ref().unwrap().contains("👩‍💻"));
    }
}
#[test]
fn scroll_limits_source_cells_and_cross_message_copy_follow_the_shared_column() {
    let mut app = App::default();
    let lines = (0..100)
        .map(|n| format!("line {n:03} contains selectable 界é text"))
        .collect::<Vec<_>>();
    app.entries.push(Entry {
        draft: None,
        kind: EntryKind::User,
        title: String::new(),
        lines: lines.clone(),
    });
    for (w, h) in [(60, 20), (80, 24), (100, 30), (120, 40), (160, 60)] {
        for scroll in [usize::MAX, 0] {
            app.scroll = scroll;
            let (before, map) = capture(&app, w, h);
            let r = map.region(&TextRegionId::Conversation).unwrap();
            assert_eq!(r.rect.y, 4);
            assert_eq!(r.rect.bottom(), h - ui::composer_height(&app, w, h) - 2);
            let total = Paragraph::new(ui::entry_lines(&app.entries[0], &app, false))
                .wrap(Wrap { trim: false })
                .line_count(r.rect.width);
            assert_eq!(
                ui::conversation_max_scroll(&app, w, h),
                total - r.rect.height as usize
            );
            for c in &r.cells {
                if let Some(g) = r.source[c.byte_offset..]
                    .chars()
                    .next()
                    .filter(|g| !g.is_whitespace())
                {
                    assert!(before[(c.x, c.y)].symbol().starts_with(g));
                }
            }
            if scroll == usize::MAX {
                assert!(r.cells.iter().any(|c| c.byte_offset == 0));
            } else {
                assert!(text(&before, r.rect).contains("line 099"));
            }
            app.view_selection = Some(ViewSelection {
                region: TextRegionId::Conversation,
                anchor: 0,
                end: r.source.len(),
            });
            let (after, selected) = capture(&app, w, h);
            assert_eq!(selection_text(&app, &selected), lines.join("\n"));
            for (index, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
                let (x, y) = ((index % w as usize) as u16, (index / w as usize) as u16);
                let copied = r.cells.iter().any(|c| {
                    c.y == y
                        && c.byte_offset < r.source.len()
                        && !r.source[c.byte_offset..].starts_with('\n')
                        && x >= c.x
                        && x < c.x
                            + UnicodeWidthStr::width(
                                r.source[c.byte_offset..].graphemes(true).next().unwrap(),
                            )
                            .max(1) as u16
                });
                assert_eq!(
                    b.bg,
                    // TestBackend clears structural wide-glyph continuation cells to Reset.
                    if copied && a.bg != Color::Reset {
                        selection::background(app.theme, a)
                    } else {
                        a.bg
                    },
                    "{w}x{h} scroll {scroll} at {x},{y}: before {a:?}, after {b:?}, cells {:?}",
                    r.cells
                        .iter()
                        .filter(|c| c.y == y && c.x.abs_diff(x) < 4)
                        .collect::<Vec<_>>()
                );
                assert_eq!(b.fg, a.fg);
            }
            app.view_selection = None;
        }
    }
}
#[test]
fn north_star_is_regular_unfilled_and_portable_through_all_theme_fallbacks() {
    for rgb in [true, false] {
        for color in [true, false] {
            for ascii in [true, false] {
                for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
                    let mut app = App::default();
                    app.theme = Theme { mode, color, rgb };
                    app.ascii = ascii;
                    for chat in [false, true] {
                        if chat {
                            app.entries
                                .push(Entry::new(EntryKind::User, "", &["hello"]));
                        }
                        let (b, _) = capture(&app, 100, 30);
                        let all = text(&b, b.area);
                        assert!(!all.contains("┬"));
                        if ascii {
                            assert!(all.is_ascii());
                            assert!(all.contains('*'));
                        } else {
                            assert_eq!(stars(&b).len(), if chat { 1 } else { 2 });
                        }
                        if !color {
                            assert!(
                                b.content
                                    .iter()
                                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
                            );
                        }
                        if mode == ThemeMode::Terminal {
                            assert!(b.content.iter().all(|c| c.bg == Color::Reset));
                        }
                        if !rgb {
                            assert!(b.content.iter().all(|c| !matches!(c.fg, Color::Rgb(..))
                                && !matches!(c.bg, Color::Rgb(..))));
                        }
                        for (x, y) in stars(&b) {
                            assert!(
                                !b[(x, y)]
                                    .modifier
                                    .intersects(Modifier::BOLD | Modifier::REVERSED)
                            );
                        }
                    }
                }
            }
        }
    }
    let small = ui::dump(&App::default(), 59, 19).unwrap();
    assert!(small.contains("Resize to at least"));
    assert!(!small.contains("v0.1.0alpha"));
}
#[test]
fn drafts_expand_to_three_eighths_and_restore_the_large_welcome_after_resize() {
    let mut app = App::default();
    app.input.set(&"line\n".repeat(100));
    for (w, h) in [
        (60, 20),
        (80, 24),
        (100, 30),
        (120, 40),
        (160, 60),
        (120, 100),
    ] {
        let (_, map) = capture(&app, w, h);
        let r = map.region(&TextRegionId::Composer).unwrap().rect;
        assert_eq!(r.height, h * 3 / 8);
        assert_eq!(r.width, ui::editor_width(w) as u16);
        assert_eq!(r.bottom(), h - 3);
        let conversation = map.region(&TextRegionId::Conversation).unwrap();
        assert!(conversation.rect.height >= 4);
    }
    app.input.clear();
    assert_eq!(stars(&capture(&app, 120, 40).0).len(), 2);
}
#[test]
fn selection_only_shades_source_cells_and_never_changes_star_chrome_across_themes() {
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for rgb in [true, false] {
            for color in [true, false] {
                for chat in [false, true] {
                    let mut app = App::default();
                    app.theme = Theme { mode, rgb, color };
                    app.input.set("select @file.rs /plan");
                    if chat {
                        app.entries.push(Entry::new(
                            EntryKind::User,
                            "",
                            &["select @file.rs /plan"],
                        ));
                    }
                    let (before, map) = capture(&app, 100, 30);
                    let r = map.region(&TextRegionId::Composer).unwrap();
                    app.view_selection = Some(ViewSelection {
                        region: TextRegionId::Composer,
                        anchor: 0,
                        end: r.source.len(),
                    });
                    let (after, _) = capture(&app, 100, 30);
                    for (index, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
                        let (x, y) = ((index % 100) as u16, (index / 100) as u16);
                        let selected = r
                            .cells
                            .iter()
                            .any(|c| c.x == x && c.y == y && c.byte_offset < r.source.len());
                        assert_eq!(
                            b.bg,
                            if selected && color {
                                selection::background(app.theme, a)
                            } else {
                                a.bg
                            }
                        );
                        assert_eq!(a.fg, b.fg);
                        if !selected {
                            assert_eq!(a, b);
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn explicit_account_and_long_draft_keep_every_interaction_inside_the_screen() {
    let mut app = App::default();
    app.account_email = Some("local@example.invalid".into());
    app.input.set(&"draft\n".repeat(80));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let (b, map) = capture(&app, w, h);
        assert!(text(&b, b.area).contains("local@example.invalid"));
        let composer = map.region(&TextRegionId::Composer).unwrap();
        assert!(composer.rect.bottom() < h - 2);
        for target in &map.targets {
            assert!(b.area.contains((target.rect.x, target.rect.y).into()));
            assert!(target.rect.right() <= w && target.rect.bottom() <= h);
        }
        app.command("/model");
        let (_, m) = capture(&app, w, h);
        let popup = ui::popup_area(&app, Rect::new(0, 0, w, h));
        assert!(
            m.text_regions
                .iter()
                .all(|r| r.cells.iter().all(|c| !popup.contains((c.x, c.y).into())))
        );
        app.close();
    }
}
