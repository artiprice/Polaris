use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, Screen},
    interaction::{HitAction, UiMap},
    runtime::MouseController,
    theme::{Theme, ThemeMode},
    ui,
    usage::{self, Gauge, UsageAction, UsageBarStyle, UsageSnapshot},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    text::Line,
};
#[path = "support/capture.rs"]
mod capture;
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn draw(app: &App, w: u16, h: u16) -> (Buffer, UiMap) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (t.backend().buffer().clone(), map)
}
fn text(b: &Buffer) -> String {
    (0..b.area.height)
        .map(|y| {
            (0..b.area.width)
                .map(|x| b[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn line(l: &Line<'_>) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}
fn gauge(n: Option<u64>, d: Option<u64>) -> Gauge {
    Gauge {
        label: "Current window".into(),
        used: n,
        capacity: d,
        unit: "units",
        reset: None,
    }
}
fn click(app: &mut App, map: &UiMap, a: UsageAction) {
    let r = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Usage(a))
        .unwrap()
        .rect;
    let mut m = MouseController::default();
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        m.event(
            MouseEvent {
                kind,
                column: r.x,
                row: r.y,
                modifiers: KeyModifiers::NONE,
            },
            map,
            app,
        );
    }
}
#[test]
fn sample_totals_come_from_rows_and_subagents_are_included_not_extra() {
    let s = UsageSnapshot::demo();
    let models = s.models.as_ref().unwrap();
    assert_eq!(s.input, Some(12_400));
    assert_eq!(s.output, Some(3_180));
    assert_eq!(s.cost.as_ref().unwrap().minor, 42);
    assert_eq!(
        models.iter().map(|m| m.input).sum::<u64>(),
        s.input.unwrap()
    );
    assert_eq!(
        models.iter().map(|m| m.output).sum::<u64>(),
        s.output.unwrap()
    );
    assert_eq!(
        models
            .iter()
            .map(|m| m.cost.as_ref().unwrap().minor)
            .sum::<u64>(),
        42
    );
    assert!(
        models
            .iter()
            .all(|m| m.cost.as_ref().unwrap().currency == "USD")
    );
    assert_eq!(models.iter().map(|m| m.cache_read).sum::<u64>(), 100_000);
    assert_eq!(models.iter().map(|m| m.cache_write).sum::<u64>(), 2_000);
    let agents = s.subagents.as_ref().unwrap();
    assert_eq!(
        agents.iter().map(|a| a.input + a.output).sum::<u64>(),
        3_080
    );
    assert!(
        agents
            .iter()
            .all(|a| models.iter().any(|m| m.name == a.model))
    );
    assert_eq!(s.context.percentage(), "20%");
    assert_eq!(s.limits[0].percentage(), "42%");
    assert_eq!(s.limits[1].percentage(), "58%");
    let empty = UsageSnapshot::unavailable();
    assert!(empty.cost.is_none() && empty.models.is_none() && empty.subagents.is_none());
}
#[test]
fn missing_zero_fractional_exhausted_and_overflowing_ratios_have_distinct_honest_states() {
    for (g, p, status) in [
        (gauge(None, Some(100)), "Unavailable", ""),
        (gauge(Some(0), None), "Unavailable", ""),
        (gauge(Some(5), Some(0)), "Unavailable", ""),
        (gauge(Some(0), Some(100)), "0%", ""),
        (gauge(Some(1), Some(1000)), "<1%", ""),
        (gauge(Some(79), Some(100)), "79%", ""),
        (gauge(Some(80), Some(100)), "80%", "Near limit"),
        (gauge(Some(100), Some(100)), "100%", "Limit reached"),
        (gauge(Some(108), Some(100)), "108%", "Over limit"),
    ] {
        assert_eq!(g.percentage(), p);
        assert_eq!(g.status(), status);
        for style in UsageBarStyle::ALL {
            for width in [0, 1, 7, 31, 55, 100] {
                let b = usage::bar(&g, style, width, Theme::default(), false);
                assert!(b.width() <= usize::from(width.min(55)));
                if p == "Unavailable" && width >= 13 {
                    assert_eq!(line(&b), "Unavailable");
                }
                assert!(
                    b.spans
                        .iter()
                        .all(|s| !s.style.add_modifier.contains(Modifier::BOLD))
                );
            }
        }
    }
    assert_eq!(
        gauge(Some(u64::MAX), Some(1)).percentage(),
        format!("{}%", u128::from(u64::MAX) * 100)
    );
}
#[test]
fn bars_have_exact_distinct_geometry_endpoints_and_portable_noncolor_shapes() {
    let theme = Theme::default();
    assert_eq!(
        line(&usage::bar(
            &gauge(Some(0), Some(100)),
            UsageBarStyle::FineTicks,
            9,
            theme,
            false
        )),
        "╋─┬─┬─┬─┬"
    );
    assert_eq!(
        line(&usage::bar(
            &gauge(Some(100), Some(100)),
            UsageBarStyle::FineTicks,
            9,
            theme,
            false
        )),
        "┬─┬─┬─┬─╋"
    );
    let mut mono = theme;
    mono.color = false;
    assert_eq!(
        line(&usage::bar(
            &gauge(Some(50), Some(100)),
            UsageBarStyle::ContinuousStrip,
            8,
            mono,
            false
        )),
        "▄▄▄▄····"
    );
    assert_eq!(
        line(&usage::bar(
            &gauge(Some(50), Some(100)),
            UsageBarStyle::SegmentedRail,
            7,
            mono,
            false
        )),
        "▮ ▮ ▯ ▯"
    );
    for style in UsageBarStyle::ALL {
        for n in [0, 1, 42, 80, 100, 108] {
            let g = gauge(Some(n), Some(100));
            let b = usage::bar(&g, style, 55, theme, false);
            let active: Vec<_> = b
                .spans
                .iter()
                .filter(|s| s.style.fg != theme.rule().fg)
                .collect();
            assert!(!active.is_empty() || n < 2 || style != UsageBarStyle::FineTicks);
            if n >= 100 {
                assert!(
                    active
                        .iter()
                        .all(|s| s.style.fg == Some(Color::Rgb(172, 66, 66)))
                );
            }
            let ascii = usage::bar(&g, style, 55, theme, true);
            assert!(line(&ascii).is_ascii());
            let nc = usage::bar(&g, style, 55, mono, false);
            assert!(
                nc.spans
                    .iter()
                    .all(|s| s.style.fg.is_none() || s.style.fg == Some(Color::Reset))
            );
        }
    }
}
#[test]
fn usage_keeps_details_but_style_controls_belong_to_settings() {
    let mut app = App::default();
    app.input.set("keep 界 draft");
    app.input.cursor = 5;
    app.working = true;
    app.scroll = 17;
    let original = app.input.clone();
    app.command("/usage");
    for c in ['1', '2', '3', 'c'] {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::ContinuousStrip
    );
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(app.usage.details);
    assert!(!app.tool_details_expanded);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.scroll, 17);
    assert_eq!(app.input.text, original.text);
    assert_eq!(app.input.cursor, original.cursor);
    assert!(app.working);
    app.command("/settings");
    polaris::settings::activate(&mut app, polaris::settings::SettingsAction::Category(1));
    polaris::settings::activate(
        &mut app,
        polaris::settings::SettingsAction::Choose { row: 2, value: 0 },
    );
    polaris::settings::activate(&mut app, polaris::settings::SettingsAction::Apply);
    key(&mut app, KeyCode::Esc);
    app.command("/usage");
    assert!(app.usage.details);
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::SegmentedRail
    );
    key(&mut app, KeyCode::Char('d'));
    assert!(app.metrics_demo);
    key(&mut app, KeyCode::Esc);
    assert!(app.working);
}

#[test]
fn usage_details_button_and_wheel_remain_interactive_without_a_gallery() {
    let mut app = App::default();
    app.load_scenario("usage-demo").unwrap();
    let (_, map) = draw(&app, 80, 24);
    assert_eq!(
        map.targets
            .iter()
            .filter(|t| matches!(t.action, HitAction::Usage(_)))
            .count(),
        1
    );
    click(&mut app, &map, UsageAction::Details);
    assert!(app.usage.details);
    let (_, map) = draw(&app, 60, 20);
    let mut mouse = MouseController::default();
    mouse.event(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 20,
            row: 10,
            modifiers: KeyModifiers::NONE,
        },
        &map,
        &mut app,
    );
    assert_eq!(app.scroll, 3);
    click(&mut app, &map, UsageAction::Details);
    assert!(!app.usage.details);
    assert_eq!(app.scroll, 0);
    let text = text(&draw(&app, 120, 40).0);
    assert!(!text.contains("Compare") && !text.contains("1 Ticks") && !text.contains("2 Strip"));
}

#[test]
fn unavailable_and_demo_details_remain_distinct_and_models_and_cache_are_readable() {
    let mut app = App::default();
    app.command("/usage");
    let mut data = UsageSnapshot::unavailable();
    app.usage.details = true;
    let unavailable = usage::information(&data, &app, 52)
        .iter()
        .map(line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(unavailable.contains("Unavailable"));
    assert!(!unavailable.contains("0%") && !unavailable.contains("0.00"));
    for row in usage::information(&data, &app, 52) {
        for span in row.spans {
            if span.content.contains("Unavailable") {
                assert_ne!(
                    span.style.fg,
                    Some(Color::Rgb(118, 185, 0)),
                    "missing data looked healthy"
                );
            }
        }
    }
    assert!(!text(&draw(&app, 80, 24).0).contains("DEMO DATA"));
    data.context.used = Some(12_400);
    let partial = usage::information(&data, &app, 52)
        .iter()
        .map(line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(partial.contains("12,400 tokens used; capacity unavailable"));
    let data = UsageSnapshot::demo();
    let details = usage::information(&data, &app, 52)
        .iter()
        .map(line)
        .collect::<Vec<_>>()
        .join("\n");
    for s in [
        "Models",
        "Cache",
        "100,000 / 2,000 tokens",
        "90% / 90 of 100 requests",
        "General purpose",
        "Explore",
        "Included in session/model totals",
    ] {
        assert!(details.contains(s), "missing {s}");
    }
    app.usage.details = false;
    let core = usage::information(&data, &app, 104)
        .iter()
        .map(line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!core.contains("Subagents") && !core.contains("Hit rate"));
    assert!(core.contains("Models") && core.contains("USD 0.42"));
}
#[test]
fn every_style_fits_resizes_and_keeps_pinned_details_and_occlusion() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60), (240, 60)] {
        for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
            for color in [true, false] {
                for ascii in [false, true] {
                    let mut app = App::default();
                    app.load_scenario("usage-demo").unwrap();
                    app.theme.mode = mode;
                    app.theme.color = color;
                    app.ascii = ascii;
                    for style in UsageBarStyle::ALL {
                        app.preferences.values.usage_bar_style = style;
                        app.scroll = ui::usage_max_scroll(&app, w, h);
                        let (b, map) = draw(&app, w, h);
                        let sheet = ui::popup_area(&app, b.area);
                        for target in map
                            .targets
                            .iter()
                            .filter(|t| matches!(t.action, HitAction::Usage(_)))
                        {
                            assert!(sheet.contains((target.rect.x, target.rect.y).into()));
                            assert!(
                                target.rect.right() <= w && target.rect.bottom() < sheet.bottom()
                            );
                        }
                        assert!(
                            map.text_regions.iter().all(|r| r
                                .cells
                                .iter()
                                .all(|c| !sheet.contains((c.x, c.y).into())))
                        );
                        let s = text(&b);
                        assert!(s.contains("DEMO DATA") && s.contains("Ctrl+O Details"));
                        assert!(
                            b.content
                                .iter()
                                .all(|c| !c.modifier.contains(Modifier::REVERSED))
                        );
                        if ascii {
                            assert!(s.is_ascii());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn overview_and_expanded_details_resize_without_losing_reachability_or_draft() {
    let mut app = App::default();
    app.input.set("unsent draft");
    app.working = true;
    app.command("/usage");
    app.metrics_demo = true;
    for expanded in [false, true] {
        app.usage.details = expanded;
        for (w, h) in [(60, 20), (80, 24), (120, 40), (240, 60), (80, 24)] {
            app.scroll = usize::MAX;
            app.scroll = app.scroll.min(ui::usage_max_scroll(&app, w, h));
            let (b, map) = draw(&app, w, h);
            let s = text(&b);
            assert!(s.contains("DEMO DATA"));
            assert!(s.contains("Models") || expanded);
            if expanded {
                assert!(s.contains("Included in session/model totals"));
            }
            assert!(
                map.targets
                    .iter()
                    .filter(|t| matches!(t.action, HitAction::Usage(_)))
                    .all(|t| t.rect.right() <= w)
            );
            assert_eq!(app.input.text, "unsent draft");
            assert!(app.working);
        }
    }
}
#[test]
fn export_actual_usage_buffers_for_all_styles_and_details() {
    for style in UsageBarStyle::ALL {
        for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60), (240, 60)] {
            let mut app = App::new("C:/workspace/polaris".into());
            app.load_scenario("usage-demo").unwrap();
            app.preferences.values.usage_bar_style = style;
            for details in [false, true] {
                app.usage.details = details;
                let (b, _) = draw(&app, w, h);
                assert!(text(&b).contains("DEMO DATA"));
                capture::export(
                    &format!(
                        "usage-{}-{}",
                        style.label().to_lowercase(),
                        if details { "details" } else { "core" }
                    ),
                    &b,
                );
            }
        }
    }
    let mut app = App::default();
    app.command("/usage");
    capture::export("usage-unavailable", &draw(&app, 120, 40).0);
}

#[test]
fn wrapped_metadata_and_limit_status_keep_their_semantic_colors() {
    let mut app = App::default();
    app.load_scenario("usage-demo").unwrap();
    let mut overflow = UsageSnapshot::demo();
    overflow.limits[0].used = Some(1_080);
    let rows = usage::information(&overflow, &app, 52);
    let status = rows.iter().find(|r| line(r) == "Over limit").unwrap();
    assert!(
        status
            .spans
            .iter()
            .all(|s| s.style.fg == Some(Color::Rgb(172, 66, 66))
                && !s.style.add_modifier.contains(Modifier::BOLD))
    );
    let metadata = rows
        .iter()
        .find(|r| line(r).contains("1,080 / 1,000 units"))
        .unwrap();
    assert!(
        metadata
            .spans
            .iter()
            .all(|s| s.style.fg == Some(Color::Rgb(133, 133, 133)))
    );
    app.usage.details = true;
    let mut data = UsageSnapshot::demo();
    data.models.as_mut().unwrap()[0].name = "界é very long model ".repeat(20);
    data.models.as_mut().unwrap()[0].cache_read = u64::MAX;
    data.models.as_mut().unwrap()[1].cache_read = u64::MAX;
    for width in [52, 104, 232] {
        let rows = usage::information(&data, &app, width);
        assert!(rows.iter().all(|l| l.width() <= width as usize));
        assert!(rows.iter().map(line).collect::<String>().contains("Cache"));
    }
}
#[test]
fn paging_tracks_the_visible_information_height_and_hover_fill_stays_inside_controls() {
    let mut app = App::default();
    app.load_scenario("usage-demo").unwrap();
    for expanded in [false, true] {
        app.usage.details = expanded;
        for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60), (80, 60)] {
            if h == 60 {
                assert_eq!(
                    ui::popup_area(&app, ratatui::layout::Rect::new(0, 0, w, h)).height,
                    if expanded && w < 112 { 40 } else { 30 }
                );
            }
            app.scroll = 0;
            app.usage.page_rows = ui::usage_page_rows(&app, w, h);
            assert!(app.usage.page_rows > 0);
            let expected = app.usage.page_rows;
            key(&mut app, KeyCode::PageDown);
            assert_eq!(app.scroll, expected);
            key(&mut app, KeyCode::PageUp);
            assert_eq!(app.scroll, 0);
            let (before, map) = draw(&app, w, h);
            let r = map
                .targets
                .iter()
                .find(|t| t.action == HitAction::Usage(UsageAction::Details))
                .unwrap()
                .rect;
            app.mouse_position = Some((r.x, r.y));
            let (after, _) = draw(&app, w, h);
            for (i, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
                if a.bg != b.bg {
                    assert!(r.contains(((i % w as usize) as u16, (i / w as usize) as u16).into()));
                }
            }
            app.mouse_position = None;
        }
    }
}
