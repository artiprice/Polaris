use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, SCENARIOS, Screen},
    interaction::{HitAction, UiMap},
    preferences::{PreferenceField, SaveStatus},
    runtime::MouseController,
    settings::{self, CATEGORIES, SettingsAction},
    theme::ThemeMode,
    ui,
    usage::UsageBarStyle,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
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
fn click(app: &mut App, map: &UiMap, action: SettingsAction) {
    let r = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Settings(action))
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
fn candidate_bounds_focus_changes_and_reopening_discard_only_unconfirmed_choices() {
    let mut app = App::default();
    app.command("/settings");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.highlighted_value, Some(0));
    assert_eq!(app.theme.mode, ThemeMode::Dark);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.settings.highlighted_value, Some(1));
    key(&mut app, KeyCode::Left);
    assert_eq!(app.settings.highlighted_value, Some(0));
    assert!(!app.settings.categories_focused);
    key(&mut app, KeyCode::Left);
    assert!(app.settings.categories_focused);
    assert_eq!(app.settings.highlighted_value, None);
    key(&mut app, KeyCode::Left);
    assert!(app.settings.categories_focused);
    for leave in [KeyCode::Tab, KeyCode::BackTab] {
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Right);
        key(&mut app, leave);
        assert_eq!(app.settings.highlighted_value, None);
        key(&mut app, leave);
        assert_eq!(app.settings.highlighted_value, Some(0));
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.settings.highlighted_value, Some(0));
        key(&mut app, KeyCode::Tab);
        settings::activate(&mut app, SettingsAction::Category(0));
    }
    settings::activate(&mut app, SettingsAction::Choose { row: 0, value: 1 });
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.settings.highlighted_value, None);
    app.command("/settings");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings.highlighted_value, Some(1));
    assert_eq!(app.theme.mode, ThemeMode::Terminal);
    settings::activate(&mut app, SettingsAction::Category(1));
    settings::activate(&mut app, SettingsAction::Choose { row: 2, value: 0 });
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Left);
    assert!(app.settings.categories_focused);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.selected, 2); // Enter Usage at the applied style.
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Left);
    assert!(app.settings.categories_focused);
    settings::activate(
        &mut app,
        SettingsAction::Category(settings::SettingsCategory::Providers.index()),
    );
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Left);
    assert!(app.settings.categories_focused);
    assert_eq!(app.screen, Screen::Settings);
}
#[test]
fn pending_markers_and_hover_preserve_applied_values_across_render_modes_and_resize() {
    let mut app = App::default();
    app.command("/settings");
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for color in [true, false] {
            for ascii in [true, false] {
                app.theme.mode = mode;
                app.theme.color = color;
                app.ascii = ascii;
                for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
                    settings::activate(&mut app, SettingsAction::Category(0));
                    settings::activate(&mut app, SettingsAction::Choose { row: 0, value: 1 });
                    let before = app.preferences.values.clone();
                    let (b, map) = draw(&app, w, h);
                    assert!(text(&b).contains("Enter to apply"));
                    assert!(text(&b).contains("[x] Dark"));
                    assert!(text(&b).contains("[ ] Terminal"));
                    assert!(text(&b).contains(">[ ] Terminal"));
                    let r = map
                        .targets
                        .iter()
                        .find(|t| {
                            t.action
                                == HitAction::Settings(SettingsAction::Choose { row: 0, value: 0 })
                        })
                        .unwrap()
                        .rect;
                    app.mouse_position = Some((r.x, r.y));
                    draw(&app, w, h);
                    assert_eq!(app.settings.highlighted_value, Some(1));
                    assert_eq!(app.preferences.values, before);
                    assert_eq!(app.theme.mode, mode);
                    assert_eq!(app.theme.color, color);
                    assert_eq!(app.ascii, ascii);
                    capture::export("settings-pending-theme", &b);
                    settings::activate(&mut app, SettingsAction::Category(1));
                    settings::activate(&mut app, SettingsAction::Select(2));
                    let b = draw(&app, w, h).0;
                    assert!(text(&b).contains("[ ] Segments"));
                    assert!(text(&b).contains("Enter to apply"));
                    if h >= 40 {
                        assert!(text(&b).contains("[x] Strip"));
                    }
                    assert_eq!(
                        app.preferences.values.usage_bar_style,
                        UsageBarStyle::ContinuousStrip
                    );
                    capture::export("settings-pending-usage", &b);
                }
            }
        }
    }
}
#[test]
fn keyboard_category_focus_choices_and_close_preserve_draft_work_and_scroll() {
    let mut app = App::default();
    app.input.set("keep this 界 draft");
    app.input.cursor = 5;
    app.scroll = 17;
    app.working = true;
    app.command("/settings");
    assert!(app.settings.categories_focused);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.tab, 1);
    key(&mut app, KeyCode::Tab);
    assert!(!app.settings.categories_focused);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::FineTicks
    );
    key(&mut app, KeyCode::BackTab);
    assert!(app.settings.categories_focused);
    key(&mut app, KeyCode::Up);
    assert_eq!(app.tab, 0);
    assert_eq!(app.selected, 0);
    key(&mut app, KeyCode::Right);
    assert!(!app.settings.categories_focused);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.theme.mode, ThemeMode::Dark);
    assert_eq!(app.settings.highlighted_value, Some(1));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme.mode, ThemeMode::Terminal);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.theme.mode, ThemeMode::Terminal);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme.mode, ThemeMode::Dark);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.ascii);
    key(&mut app, KeyCode::Enter);
    assert!(app.ascii);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.input.text, "keep this 界 draft");
    assert_eq!(app.input.cursor, 5);
    assert_eq!(app.scroll, 17);
    assert!(app.working);
}
#[test]
fn mouse_only_highlights_then_enter_applies_and_every_category_remains_reachable() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        let mut app = App::default();
        app.command("/settings");
        for category in 0..CATEGORIES.len() {
            let (_, map) = draw(&app, w, h);
            click(&mut app, &map, SettingsAction::Category(category));
            assert_eq!(app.tab, category);
            for row in 0..settings::row_count(category) {
                settings::activate(&mut app, SettingsAction::Select(row));
                let (_, map) = draw(&app, w, h);
                let value = if category == 0 { 1 } else { 0 };
                let before = app.preferences.values.clone();
                let mode = app.mode;
                click(&mut app, &map, SettingsAction::Choose { row, value });
                assert_eq!(app.preferences.values, before);
                assert_eq!(app.mode, mode);
                assert_eq!(app.screen, Screen::Settings);
                key(&mut app, KeyCode::Enter);
                if category == 1 {
                    assert_eq!(
                        app.preferences.values.usage_bar_style,
                        [
                            UsageBarStyle::ContinuousStrip,
                            UsageBarStyle::FineTicks,
                            UsageBarStyle::SegmentedRail
                        ][row]
                    );
                }
                if category == settings::SettingsCategory::Providers.index() {
                    assert_eq!(app.screen, Screen::Providers);
                    key(&mut app, KeyCode::Esc);
                    assert_eq!(app.screen, Screen::Settings);
                    assert_eq!(app.tab, settings::SettingsCategory::Providers.index());
                }
            }
        }
    }
}
#[test]
fn wheel_and_page_keys_scroll_content_while_title_categories_and_hints_stay_pinned() {
    let mut app = App::default();
    app.load_scenario("usage-bars").unwrap();
    let (before, map) = draw(&app, 60, 20);
    assert!(text(&before).contains("Strip / default"));
    let mut mouse = MouseController::default();
    mouse.event(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 35,
            row: 12,
            modifiers: KeyModifiers::NONE,
        },
        &map,
        &mut app,
    );
    assert_eq!(app.scroll, 3);
    assert!(!app.settings.reveal_selection);
    app.settings.page_rows = ui::settings_page_rows(&app, 60, 20);
    let page = app.settings.page_rows;
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.scroll, 3 + page);
    app.scroll = ui::settings_max_scroll(&app, 60, 20);
    let (after, map) = draw(&app, 60, 20);
    let s = text(&after);
    assert!(s.contains("Segments"));
    assert!(s.contains("Settings") && s.contains("Esc close"));
    assert_eq!(before[(6, 5)], after[(6, 5)]); // title is pinned
    assert!(
        map.targets
            .iter()
            .any(|t| t.action == HitAction::Settings(SettingsAction::Category(0)))
    );
    key(&mut app, KeyCode::PageUp);
    assert!(app.scroll < ui::settings_max_scroll(&app, 60, 20));
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    let (_, map) = draw(&app, 60, 20);
    click(&mut app, &map, SettingsAction::Choose { row: 2, value: 0 });
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::ContinuousStrip
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::SegmentedRail
    );
}
#[test]
fn keyboard_reveal_then_wheel_scrolls_from_the_visible_offset_after_resize() {
    let mut app = App::default();
    app.load_scenario("usage-bars").unwrap();
    settings::activate(&mut app, SettingsAction::Select(2));
    app.scroll = ui::settings_scroll_offset(&app, 60, 20);
    assert_eq!(app.scroll, 7);
    let (_, map) = draw(&app, 60, 20);
    let mut mouse = MouseController::default();
    mouse.event(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 35,
            row: 12,
            modifiers: KeyModifiers::NONE,
        },
        &map,
        &mut app,
    );
    assert_eq!(app.scroll, 4);
    app.scroll = ui::settings_scroll_offset(&app, 160, 60);
    assert_eq!(app.scroll, 0);
    let s = text(&draw(&app, 160, 60).0);
    assert!(s.contains("Strip / default") && s.contains("Segments"));
}

#[test]
fn all_categories_resize_and_occlude_underlying_content_with_portable_styles() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
            for color in [true, false] {
                for ascii in [true, false] {
                    let mut app = App::default();
                    app.load_scenario("conversation").unwrap();
                    app.command("/settings");
                    app.theme.mode = mode;
                    app.theme.color = color;
                    app.ascii = ascii;
                    for category in 0..CATEGORIES.len() {
                        settings::activate(&mut app, SettingsAction::Category(category));
                        for row in 0..settings::row_count(category) {
                            settings::activate(&mut app, SettingsAction::Select(row));
                            let (b, map) = draw(&app, w, h);
                            let sheet = ui::popup_area(&app, b.area);
                            let s = text(&b);
                            assert!(s.contains("Settings") && s.contains("Esc close"));
                            for t in map
                                .targets
                                .iter()
                                .filter(|t| matches!(t.action, HitAction::Settings(_)))
                            {
                                assert!(
                                    t.rect.x >= sheet.x
                                        && t.rect.right() <= sheet.right()
                                        && t.rect.y > sheet.y
                                        && t.rect.bottom() < sheet.bottom()
                                );
                            }
                            assert!(map.text_regions.iter().all(|r| {
                                r.cells.iter().all(|c| !sheet.contains((c.x, c.y).into()))
                            }));
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
}
#[test]
fn hover_is_visible_and_confined_to_rows_and_categories() {
    let mut app = App::default();
    app.load_scenario("usage-bars").unwrap();
    for (w, h) in [(60, 20), (120, 40)] {
        let (before, map) = draw(&app, w, h);
        let target = map
            .targets
            .iter()
            .find(|t| t.action == HitAction::Settings(SettingsAction::Choose { row: 0, value: 0 }))
            .unwrap()
            .rect;
        app.mouse_position = Some((target.x, target.y));
        let (after, map) = draw(&app, w, h);
        let mut changed = 0;
        for (i, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
            if a.bg != b.bg {
                changed += 1;
                let p = ((i % w as usize) as u16, (i / w as usize) as u16);
                assert!(map.targets.iter().any(
                    |t| matches!(t.action, HitAction::Settings(_)) && t.rect.contains(p.into())
                ));
                assert_eq!(b.bg, Color::Rgb(24, 24, 24));
            }
        }
        assert!(changed > 0);
        app.mouse_position = None;
    }
}
#[test]
fn compatibility_alias_opens_settings_usage_and_gallery_is_not_advertised() {
    let mut app = App::default();
    app.load_scenario("usage-bars").unwrap();
    assert_eq!(app.screen, Screen::Settings);
    assert_eq!(app.tab, 1);
    assert!(!app.metrics_demo);
    assert!(!SCENARIOS.iter().any(|(name, _)| *name == "usage-bars"));
    let s = text(&draw(&app, 120, 40).0);
    assert!(s.contains("42% sample") && s.contains("Ticks") && s.contains("Segments"));
    assert!(!s.contains("BAR STYLE DEMO"));
}
#[test]
fn saved_status_diagnostics_and_launch_override_have_honest_regular_weight_labels() {
    let mut app = App::default();
    app.command("/settings");
    app.preferences.overrides.insert(PreferenceField::Theme);
    for status in [
        SaveStatus::Saved,
        SaveStatus::Warning("Invalid UI preferences; original file kept.".into()),
        SaveStatus::Error("Could not save. Session changes remain active.".into()),
    ] {
        app.preferences.status = status;
        let (b, _) = draw(&app, 120, 40);
        let s = text(&b);
        assert!(s.contains(app.preferences.status.label()));
        assert!(s.contains("launch override"));
        if matches!(app.preferences.status, SaveStatus::Error(_)) {
            for y in 0..b.area.height {
                for x in 0..b.area.width {
                    if b[(x, y)].fg == Color::Rgb(172, 66, 66) {
                        assert!(!b[(x, y)].modifier.contains(Modifier::BOLD));
                    }
                }
            }
        }
    }
}
#[test]
fn export_settings_categories_narrow_and_diagnostic_buffers() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for (category, label) in CATEGORIES.into_iter().enumerate() {
            let mut app = App::new("C:/workspace/polaris".into());
            app.command("/settings");
            settings::activate(&mut app, SettingsAction::Category(category));
            let (b, _) = draw(&app, w, h);
            capture::export(&format!("settings-{}", label.to_lowercase()), &b);
            if category == 1 {
                for row in 0..3 {
                    settings::activate(&mut app, SettingsAction::Select(row));
                    let (b, _) = draw(&app, w, h);
                    capture::export(&format!("settings-usage-row-{row}"), &b);
                }
            }
        }
    }
    let mut app = App::default();
    app.command("/settings");
    app.preferences.status = SaveStatus::Error(
        "Could not save UI preferences: directory is read-only. Session changes remain active."
            .into(),
    );
    capture::export("settings-save-error", &draw(&app, 80, 24).0);
}
