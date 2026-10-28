#[path = "support/capture.rs"]
mod capture;
use crossterm::event::{
    KeyCode as K, KeyEvent, KeyModifiers as M, MouseButton, MouseEvent, MouseEventKind,
};
use polaris::{
    app::{App, Screen},
    interaction::{HitAction, UiMap},
    preferences::{PreferenceState, SaveStatus},
    runtime::MouseController,
    settings::{self, SettingsAction, SettingsCategory},
    stats::{self, ChartStyle, StatsAction, StatsRange, StatsTab},
    stats_chart, stats_data as data, ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
fn key(a: &mut App, k: K) {
    a.key(KeyEvent::new(k, M::NONE));
}
fn demo(name: &str) -> App {
    let mut a = App::default();
    a.load_scenario(name).unwrap();
    a
}
fn draw(a: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, a)).unwrap();
    (map, t.backend().buffer().clone())
}
fn text(b: &Buffer) -> String {
    b.content
        .chunks(b.area.width as usize)
        .map(|r| r.iter().map(|c| c.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn mouse(a: &mut App, map: &UiMap, kind: MouseEventKind, x: u16, y: u16) {
    MouseController::default().event(
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: M::NONE,
        },
        map,
        a,
    );
}
fn click(a: &mut App, map: &UiMap, action: HitAction) {
    let r = map
        .targets
        .iter()
        .find(|t| t.action == action)
        .unwrap_or_else(|| panic!("missing {action:?}"))
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
                modifiers: M::NONE,
            },
            map,
            a,
        );
    }
}
#[test]
fn navigation_and_close_preserve_main_draft_work_and_inspected_agent() {
    let mut a = demo("agents-working");
    a.view_agent(Some(1));
    a.input.set("keep 界 draft");
    a.input.cursor = 5;
    a.scroll = 7;
    a.command("/stats");
    assert!(!a.stats.demo);
    key(&mut a, K::Char('d'));
    assert!(a.stats.demo);
    assert!(!a.metrics_demo);
    key(&mut a, K::Char('r'));
    assert_eq!(a.stats.range, StatsRange::Week);
    key(&mut a, K::Char('r'));
    assert_eq!(a.stats.range, StatsRange::Month);
    key(&mut a, K::Char('r'));
    assert_eq!(a.stats.range, StatsRange::All);
    key(&mut a, K::Tab);
    assert_eq!(a.stats.tab, StatsTab::Models);
    key(&mut a, K::Char('c'));
    assert_eq!(stats::effective_style(&a), ChartStyle::Lines);
    a.key(KeyEvent::new(K::Char('o'), M::CONTROL));
    assert!(a.stats.details);
    key(&mut a, K::Char('['));
    assert!(a.stats.inspected.is_some());
    key(&mut a, K::Esc);
    assert_eq!(a.screen, Screen::Home);
    assert_eq!(a.input.text, "keep 界 draft");
    assert_eq!(a.input.cursor, 5);
    assert_eq!(a.scroll, 7);
    assert_eq!(a.agents.viewed, Some(1));
    assert!(a.working);
    assert_eq!(a.stats.preview, None);
}
#[test]
fn stats_is_independent_from_usage_and_commands_work_during_response() {
    let mut a = App::default();
    a.working = true;
    a.input.set("/stats");
    a.submit();
    assert_eq!(a.screen, Screen::Stats);
    key(&mut a, K::Char('d'));
    assert!(!a.metrics_demo);
    key(&mut a, K::Enter);
    a.command("/usage");
    assert!(!a.metrics_demo);
    key(&mut a, K::Char('d'));
    assert!(a.metrics_demo);
    key(&mut a, K::Enter);
    a.command("/stats");
    assert!(!a.stats.demo);
}
#[test]
fn unavailable_is_not_zero_and_source_badge_stays_visible() {
    let a = demo("stats");
    let (_, b) = draw(&a, 80, 24);
    let t = text(&b);
    assert!(t.contains("Unavailable"));
    assert!(t.contains("No activity source"));
    assert!(!t.contains("0 tokens"));
    assert!(!t.contains("DEMO DATA"));
}
#[test]
fn range_changes_data_and_clamps_inspection() {
    let mut a = demo("stats-demo");
    let all = text(&draw(&a, 120, 40).1);
    key(&mut a, K::Char('r'));
    let week = text(&draw(&a, 120, 40).1);
    assert_ne!(all, week);
    for _ in 0..40 {
        key(&mut a, K::Char('['));
    }
    assert_eq!(a.stats.inspected, Some(0));
    for _ in 0..40 {
        key(&mut a, K::Char(']'));
    }
    assert_eq!(a.stats.inspected, Some(6));
    key(&mut a, K::Tab);
    assert_eq!(a.stats.inspected, None);
}
#[test]
fn calendar_wraps_without_losing_any_date_and_all_details_are_reachable() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        let mut a = demo("stats-demo");
        let sheet = ui::stats_sheet(&a, w, h);
        let max = stats::max_scroll(&a, sheet, h);
        let mut dates = std::collections::BTreeSet::new();
        let mut shown = String::new();
        for scroll in 0..=max {
            a.scroll = scroll;
            let (m, b) = draw(&a, w, h);
            shown += &text(&b);
            for target in m.targets {
                if let HitAction::Stats(StatsAction::Inspect(i)) = target.action {
                    dates.insert(i);
                }
            }
        }
        assert_eq!(dates.len(), 365, "{w}x{h}");
        for s in [
            "Mon",
            "Wed",
            "Fri",
            "Less",
            "More",
            "Top model",
            "Current streak",
            "Longest streak",
            "Longest session",
        ] {
            assert!(shown.contains(s), "{w}x{h} missing {s}");
        }
    }
}
#[test]
fn scrolling_clamps_and_keeps_header_source_inspector_and_footer_pinned() {
    let mut a = demo("stats-stacked");
    let sheet = ui::stats_sheet(&a, 60, 20);
    a.stats.page_rows = stats::page_rows(&a, sheet);
    let (m, before) = draw(&a, 60, 20);
    mouse(&mut a, &m, MouseEventKind::ScrollDown, 30, 12);
    assert_eq!(a.scroll, 3);
    key(&mut a, K::PageDown);
    assert_eq!(a.scroll, 3 + a.stats.page_rows);
    a.scroll = usize::MAX;
    let (_, after) = draw(&a, 60, 20);
    assert_ne!(text(&before), text(&after));
    for s in ["Statistics", "DEMO DATA", "Stacked", "Sample"] {
        assert!(text(&after).contains(s));
    }
    let y = sheet.bottom() - 4;
    for x in 4..56 {
        assert_eq!(before[(x, y)], after[(x, y)]);
    }
    assert!(stats::max_scroll(&a, sheet, 20) > 0);
}
#[test]
fn mouse_tabs_ranges_styles_details_and_inspection_do_not_write_preferences() {
    let mut a = demo("stats-demo");
    let saved = a.preferences.saved.clone();
    let (m, _) = draw(&a, 120, 40);
    let point = m
        .targets
        .iter()
        .find(|t| matches!(t.action, HitAction::Stats(StatsAction::Inspect(_))))
        .unwrap();
    mouse(
        &mut a,
        &m,
        MouseEventKind::Moved,
        point.rect.x,
        point.rect.y,
    );
    assert!(matches!(a.stats.hover, Some(StatsAction::Inspect(_))));
    assert_eq!(a.stats.inspected, None);
    click(
        &mut a,
        &m,
        HitAction::Stats(StatsAction::Tab(StatsTab::Models)),
    );
    let (m, _) = draw(&a, 120, 40);
    click(
        &mut a,
        &m,
        HitAction::Stats(StatsAction::Style(ChartStyle::Trends)),
    );
    let (m, _) = draw(&a, 120, 40);
    click(
        &mut a,
        &m,
        HitAction::Stats(StatsAction::Range(StatsRange::Week)),
    );
    let (m, _) = draw(&a, 120, 40);
    click(&mut a, &m, HitAction::Stats(StatsAction::Details));
    assert!(a.stats.details);
    assert_eq!(stats::effective_style(&a), ChartStyle::Trends);
    assert_eq!(a.preferences.saved, saved);
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Stacked);
}
#[test]
fn settings_only_applies_chart_on_enter_and_restart_uses_saved_default() {
    let dir = std::env::temp_dir().join(format!("polaris-stats-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    let mut a = App::default();
    a.preferences = PreferenceState::load(path.clone(), true);
    a.command("/settings");
    settings::activate(
        &mut a,
        SettingsAction::Category(SettingsCategory::Statistics.index()),
    );
    settings::activate(&mut a, SettingsAction::Select(2));
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Stacked);
    assert!(!path.exists());
    key(&mut a, K::Char(' '));
    assert!(!path.exists());
    key(&mut a, K::Enter);
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Trends);
    assert!(matches!(a.preferences.status, SaveStatus::Saved));
    let state = PreferenceState::load(path.clone(), true);
    assert_eq!(state.values.stats_chart_style, ChartStyle::Trends);
    settings::activate(&mut a, SettingsAction::Select(1));
    key(&mut a, K::Left);
    assert!(a.settings.categories_focused);
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Trends);
    key(&mut a, K::Esc);
    a.command("/stats");
    assert_eq!(stats::effective_style(&a), ChartStyle::Trends);
    key(&mut a, K::Tab);
    key(&mut a, K::Char('c'));
    assert_eq!(stats::effective_style(&a), ChartStyle::Stacked);
    key(&mut a, K::Esc);
    a.command("/stats");
    assert_eq!(stats::effective_style(&a), ChartStyle::Trends);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn settings_old_files_default_safely_and_save_failures_keep_active_choice() {
    let dir = std::env::temp_dir().join(format!("polaris-stats-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    std::fs::write(&path, r#"{"schema_version":1,"future_field":42}"#).unwrap();
    let mut a = App::default();
    a.preferences = PreferenceState::load(path.clone(), true);
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Stacked);
    a.preferences.path = Some(dir.clone());
    a.command("/settings");
    settings::activate(
        &mut a,
        SettingsAction::Category(SettingsCategory::Statistics.index()),
    );
    settings::activate(&mut a, SettingsAction::Select(1));
    key(&mut a, K::Enter);
    assert!(matches!(a.preferences.status, SaveStatus::Error(_)));
    assert_eq!(a.preferences.values.stats_chart_style, ChartStyle::Lines);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("future_field")
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn chart_variants_share_records_and_remain_distinct_with_no_lost_buckets() {
    let a = demo("stats-demo");
    let d = data::demo();
    let s = d.summary(d.start, d.end);
    let models = data::chart_models(&s);
    let buckets = d.buckets(d.start, d.end, true, &models);
    let mut outputs = vec![];
    for style in [ChartStyle::Stacked, ChartStyle::Lines] {
        let c = stats_chart::timeline(&a, style, 44, 8, &buckets, &models, None);
        let hit = c
            .hits
            .iter()
            .filter_map(|(_, a)| {
                if let StatsAction::Inspect(i) = a {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(hit.len(), buckets.len());
        outputs.push(format!("{:?}", c.lines));
    }
    assert_ne!(outputs[0], outputs[1]);
}
#[test]
fn all_layouts_styles_details_and_fallbacks_have_bounded_controls_and_persistent_labels() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for style in ChartStyle::ALL {
            for details in [false, true] {
                for profile in 0..4 {
                    let mut a = demo("stats-stacked");
                    a.stats.preview = Some(style);
                    a.stats.details = details;
                    a.ascii = profile == 1;
                    a.theme.color = profile != 2;
                    a.theme.rgb = profile != 3;
                    let sheet = ui::stats_sheet(&a, w, h);
                    for scroll in [0, stats::max_scroll(&a, sheet, h)] {
                        a.scroll = scroll;
                        let (m, b) = draw(&a, w, h);
                        let t = text(&b);
                        assert!(t.contains("DEMO DATA"));
                        assert!(
                            t.contains("Esc/Enter close"),
                            "closing hints must fit at {w}x{h}"
                        );
                        assert!(
                            t.contains("Stacked") && t.contains("Lines") && t.contains("Trends")
                        );
                        assert!(
                            m.targets
                                .iter()
                                .all(|t| t.rect.right() <= w && t.rect.bottom() <= h)
                        );
                        assert!(
                            b.content.iter().all(|c| !c
                                .modifier
                                .intersects(Modifier::REVERSED | Modifier::BOLD))
                        );
                        if a.ascii {
                            assert!(t.is_ascii());
                        }
                        if profile == 0 && scroll == 0 {
                            capture::export(
                                &format!(
                                    "stats-{}-{}",
                                    style.label().to_lowercase(),
                                    if details { "details" } else { "summary" }
                                ),
                                &b,
                            );
                        }
                    }
                }
            }
        }
        let a = demo("stats-demo");
        capture::export("stats-overview", &draw(&a, w, h).1);
    }
}

#[test]
fn zero_missing_tiny_and_overlapping_series_keep_truthful_values_and_portable_markers() {
    use data::{Bucket, ModelTotal, Tokens};
    let mut a = App::default();
    a.ascii = true;
    let models = vec![
        ModelTotal {
            id: "a".into(),
            name: "First".into(),
            tokens: Tokens::known(1, 0, 0, 0),
            color: 4,
        },
        ModelTotal {
            id: "b".into(),
            name: "Second".into(),
            tokens: Tokens::known(1_000_000_000, 0, 0, 0),
            color: 1,
        },
    ];
    let buckets = vec![
        Bucket {
            start: 0,
            end: 0,
            models: vec![Some(0), Some(0)],
        },
        Bucket {
            start: 1,
            end: 1,
            models: vec![None, Some(1_000_000_000)],
        },
        Bucket {
            start: 2,
            end: 2,
            models: vec![Some(1), Some(1_000_000_000)],
        },
    ];
    for style in [ChartStyle::Stacked, ChartStyle::Lines] {
        let chart = stats_chart::timeline(&a, style, 60, 8, &buckets, &models, None);
        let symbols = chart
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(symbols.contains("1.0B"));
        assert!(symbols.is_ascii());
        assert!(
            symbols.contains('2'),
            "visible model marker follows legend order rather than palette slot"
        );
        assert_eq!(chart.hits.len(), 3);
        if style == ChartStyle::Stacked {
            assert!(symbols.contains('?'));
        } else {
            assert!(
                chart.lines.iter().take(8).all(|l| !l
                    .spans
                    .iter()
                    .skip(8)
                    .any(|s| s.content.contains('5')))
            );
        }
    }
    assert_eq!(buckets[2].models[0], Some(1));
}
#[test]
fn terminal_native_and_reduced_motion_keep_calendar_geometry_and_controls() {
    for name in ["stats-demo", "stats-stacked", "stats-lines", "stats-trends"] {
        let mut a = demo(name);
        a.theme.mode = polaris::theme::ThemeMode::Terminal;
        a.reduce_motion = true;
        let (m, b) = draw(&a, 80, 24);
        let t = text(&b);
        assert!(t.contains("DEMO DATA"));
        assert!(
            m.targets
                .iter()
                .any(|t| t.action == HitAction::Stats(StatsAction::Details))
        );
        assert!(
            b.content
                .iter()
                .all(|c| !c.modifier.contains(Modifier::REVERSED))
        );
    }
}

#[test]
fn hover_and_clicked_interval_have_distinct_shades_without_recoloring_chart_text() {
    let mut a = demo("stats-stacked");
    let (map, before) = draw(&a, 120, 40);
    let target = map
        .targets
        .iter()
        .find(|t| matches!(t.action, HitAction::Stats(StatsAction::Inspect(_))))
        .unwrap()
        .clone();
    mouse(
        &mut a,
        &map,
        MouseEventKind::Moved,
        target.rect.x,
        target.rect.y,
    );
    let (_, hovered) = draw(&a, 120, 40);
    assert_ne!(
        before[(target.rect.x, target.rect.y)].bg,
        hovered[(target.rect.x, target.rect.y)].bg
    );
    click(&mut a, &map, target.action);
    let (_, selected) = draw(&a, 120, 40);
    assert_ne!(
        hovered[(target.rect.x, target.rect.y)].bg,
        selected[(target.rect.x, target.rect.y)].bg
    );
    for y in target.rect.y..target.rect.bottom() {
        for x in target.rect.x..target.rect.right() {
            assert_eq!(before[(x, y)].fg, hovered[(x, y)].fg);
            assert_eq!(before[(x, y)].symbol(), hovered[(x, y)].symbol());
        }
    }
}

#[test]
fn limited_color_calendars_keep_all_four_intensity_levels_readable() {
    use ratatui::style::Color;
    for terminal in [false, true] {
        for enabled in [false, true] {
            let mut a = demo("stats-demo");
            a.theme.rgb = false;
            a.theme.color = enabled;
            if terminal {
                a.theme.mode = polaris::theme::ThemeMode::Terminal;
            }
            a.scroll = stats::max_scroll(&a, ui::stats_sheet(&a, 80, 24), 24);
            let (_, b) = draw(&a, 80, 24);
            if enabled {
                let y = (0..b.area.height)
                    .find(|&y| {
                        (0..b.area.width)
                            .map(|x| b[(x, y)].symbol())
                            .collect::<String>()
                            .contains("Less ")
                    })
                    .unwrap();
                let row = (0..b.area.width)
                    .map(|x| b[(x, y)].symbol())
                    .collect::<String>();
                let x = row.find("Less ").unwrap() as u16 + 5;
                for (i, color) in [234, 22, 28, 64, 76].into_iter().enumerate() {
                    for dx in 0..2 {
                        assert_eq!(b[(x + i as u16 * 2 + dx, y)].bg, Color::Indexed(color));
                        assert_eq!(b[(x + i as u16 * 2 + dx, y)].symbol(), " ");
                    }
                }
            } else {
                assert!(text(&b).contains("Less . 1 2 3 4"));
            }
        }
    }
}

#[test]
fn calendar_day_tiles_fill_their_entire_width_in_every_colored_profile() {
    use ratatui::style::Color;
    let palette = [
        Color::Rgb(18, 18, 18),
        Color::Rgb(45, 70, 14),
        Color::Rgb(65, 110, 14),
        Color::Rgb(95, 150, 10),
        Color::Rgb(118, 185, 0),
    ];
    let d = data::demo();
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for profile in 0..4 {
            let mut a = demo("stats-demo");
            a.ascii = profile == 1;
            if profile == 2 {
                a.theme.rgb = false;
            }
            if profile == 3 {
                a.theme.mode = polaris::theme::ThemeMode::Terminal;
            }
            let max = stats::max_scroll(&a, ui::stats_sheet(&a, w, h), h);
            let mut days = std::collections::BTreeSet::new();
            for scroll in 0..=max {
                a.scroll = scroll;
                let (m, b) = draw(&a, w, h);
                for target in &m.targets {
                    let HitAction::Stats(StatsAction::Inspect(index)) = &target.action else {
                        continue;
                    };
                    days.insert(*index);
                    let count = d
                        .summary(d.start + *index as i32, d.start + *index as i32)
                        .sessions;
                    let level = match count {
                        0 => 0,
                        1 => 1,
                        2..=3 => 2,
                        4..=7 => 3,
                        _ => 4,
                    };
                    let expected = if profile >= 2 {
                        Color::Indexed([234, 22, 28, 64, 76][level])
                    } else {
                        palette[level]
                    };
                    for x in target.rect.x..target.rect.right() {
                        assert_eq!(
                            b[(x, target.rect.y)].bg,
                            expected,
                            "{w}x{h}, profile {profile}, day {index}"
                        );
                        assert_eq!(b[(x, target.rect.y)].symbol(), " ");
                    }
                    assert_eq!(target.rect.height, 1);
                    if w >= 120 {
                        assert_eq!(target.rect.width, 2);
                    }
                }
                if profile == 0 && scroll == 0 {
                    capture::export("calendar-solid", &b);
                }
            }
            assert_eq!(days.len(), 365);
        }
    }
}
#[test]
fn calendar_hover_and_selection_preserve_color_and_exact_cell_boundaries() {
    use ratatui::style::Color;
    let mut a = demo("stats-demo");
    let (map, before) = draw(&a, 120, 40);
    let target = map
        .targets
        .iter()
        .find(|t| matches!(t.action, HitAction::Stats(StatsAction::Inspect(_))))
        .unwrap()
        .clone();
    let HitAction::Stats(StatsAction::Inspect(index)) = target.action else {
        panic!()
    };
    let base = before[(target.rect.x, target.rect.y)].bg;
    let Color::Rgb(r, g, b) = base else { panic!() };
    let blend = |v: u8, p: u16| ((u16::from(v) * (100 - p) + 255 * p + 50) / 100) as u8;
    mouse(
        &mut a,
        &map,
        MouseEventKind::Moved,
        target.rect.right() - 1,
        target.rect.y,
    );
    let (_, hovered) = draw(&a, 120, 40);
    assert_eq!(a.stats.inspected, None);
    assert_eq!(a.stats.hover, Some(StatsAction::Inspect(index)));
    click(&mut a, &map, HitAction::Stats(StatsAction::Inspect(index)));
    let (_, selected) = draw(&a, 120, 40);
    assert_eq!(a.stats.inspected, Some(index));
    mouse(
        &mut a,
        &map,
        MouseEventKind::Moved,
        target.rect.x,
        target.rect.y,
    );
    let (_, both) = draw(&a, 120, 40);
    for x in target.rect.x..target.rect.right() {
        for (buffer, p) in [(&hovered, 20), (&selected, 35), (&both, 45)] {
            assert_eq!(
                buffer[(x, target.rect.y)].bg,
                Color::Rgb(blend(r, p), blend(g, p), blend(b, p))
            );
            assert_eq!(buffer[(x, target.rect.y)].symbol(), " ");
        }
    }
    if target.rect.right() < 116 {
        assert_eq!(
            before[(target.rect.right(), target.rect.y)].bg,
            hovered[(target.rect.right(), target.rect.y)].bg
        );
    }
    assert!(text(&both).contains(&data::date(data::demo().start + index as i32)));
    capture::export("calendar-solid-hover", &hovered);
    capture::export("calendar-solid-selected", &selected);
}
