use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, EFFORTS, parse_submission},
    composer::Composer,
    interaction::{HitAction, TextRegionId, UiMap},
    prompt_settings::{self, SettingValue},
    runtime::{MouseController, selection_text},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};
use std::time::{Duration, Instant};
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut map = UiMap::default();
    t.draw(|f| map = ui::render_interactive(f, app)).unwrap();
    (map, t.backend().buffer().clone())
}
fn parse(text: &str) -> prompt_settings::ParsedSettings {
    let mut c = Composer::default();
    c.set(text);
    prompt_settings::parse(&c)
}
#[test]
fn placeholder_is_dark_gray_visual_only_disappears_on_input_and_cannot_be_copied() {
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let mut app = App::default();
        let (map, b) = draw(&app, w, h);
        let r = map.region(&TextRegionId::Composer).unwrap();
        assert!(r.source.is_empty());
        assert!(
            ui::dump(&app, w, h)
                .unwrap()
                .contains("What's on your mind for Polaris?")
        );
        assert_eq!(b[(r.rect.x, r.rect.y)].fg, Color::Rgb(82, 82, 82));
        let mut mouse = MouseController::default();
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            mouse.event(
                MouseEvent {
                    kind,
                    column: r.rect.x + 20,
                    row: r.rect.y,
                    modifiers: KeyModifiers::NONE,
                },
                &map,
                &mut app,
            );
        }
        assert!(selection_text(&app, &map).is_empty());
        assert!(app.input.text.is_empty());
        app.input.set(" ");
        let (filled_map, filled) = draw(&app, w, h);
        let rect = filled_map.region(&TextRegionId::Composer).unwrap().rect;
        assert!(
            !(rect.x..rect.right())
                .map(|x| filled[(x, rect.y)].symbol())
                .collect::<String>()
                .contains("What's on your mind")
        );
        app.input.clear();
        app.theme.color = false;
        let (_, b) = draw(&app, w, h);
        assert!(
            b.content
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
    }
}
#[test]
fn all_five_efforts_work_in_order_through_shortcuts_picker_and_mouse() {
    assert_eq!(EFFORTS, ["low", "medium", "high", "xhigh", "max"]);
    let mut app = App::default();
    for level in ["xhigh", "max", "max"] {
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
        assert_eq!(app.effort_label(), level);
    }
    for level in ["xhigh", "high", "medium", "low", "low"] {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
        assert_eq!(app.effort_label(), level);
    }
    app.command("/model");
    for _ in 0..3 {
        key(&mut app, KeyCode::Right);
    }
    assert_eq!(app.picker_effort(), 3);
    assert!(ui::dump(&app, 80, 24).unwrap().contains("[ xhigh ]"));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.effort_label(), "xhigh");
    app.command("/model");
    let (map, _) = draw(&app, 80, 24);
    assert!(
        map.targets
            .iter()
            .any(|t| t.action == HitAction::PopupEffortValue(4))
    );
    let rect = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::PopupEffortValue(4))
        .unwrap()
        .rect;
    let mut mouse = MouseController::default();
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        mouse.event(
            MouseEvent {
                kind,
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            },
            &map,
            &mut app,
        );
    }
    assert_eq!(app.picker_effort(), 4);
}
#[test]
fn goal_status_is_blue_immediately_above_editor_and_pauses_resumes_without_copy_pollution() {
    let mut app = App::default();
    app.input.set("/goal improve validation");
    app.submit();
    app.goal.as_mut().unwrap().running_since = Some(Instant::now() - Duration::from_secs(71));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let (map, b) = draw(&app, w, h);
        let r = map.region(&TextRegionId::Composer).unwrap();
        let y = r.rect.y - 2;
        let label = (0..w).map(|x| b[(x, y)].symbol()).collect::<String>();
        assert!(label.contains("goal active (1m 11s)"));
        assert_eq!(
            label.trim_end().len(),
            (ui::content_column(ratatui::layout::Rect::new(0, 0, w, h)).right() - 2) as usize,
            "goal label must align with the right content margin"
        );
        let x = label.find('g').unwrap() as u16;
        assert_eq!(b[(x, y)].fg, Color::Rgb(137, 180, 250));
        assert!(
            map.text_regions
                .iter()
                .all(|r| r.cells.iter().all(|c| c.y != y))
        );
    }
    key(&mut app, KeyCode::Esc);
    let goal = app.goal.as_ref().unwrap();
    assert!(!goal.is_active());
    assert!(goal.elapsed() >= Duration::from_secs(71));
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        let (map, b) = draw(&app, w, h);
        let y = map.region(&TextRegionId::Composer).unwrap().rect.y - 2;
        let label = (0..w).map(|x| b[(x, y)].symbol()).collect::<String>();
        assert!(label.contains("goal paused"));
        assert_eq!(
            label.trim_end().len(),
            (ui::content_column(ratatui::layout::Rect::new(0, 0, w, h)).right() - 2) as usize
        );
    }
    app.input.set("continue");
    app.submit();
    assert!(app.goal.as_ref().unwrap().is_active());
    for _ in 0..18 {
        app.tick();
    }
    assert!(!app.goal.as_ref().unwrap().is_active());
    app.command("/clear");
    assert!(app.goal.is_none());
}
#[test]
fn dollar_toggles_lists_ranges_unicode_and_custom_settings_are_typed() {
    for toggle in ["$websearch", "$websearch=on", "$websearch=[on]"] {
        let p = parse(toggle);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.settings.websearch, Some(true));
    }
    let p = parse(
        "$websearch=off $subagents $subagents-model=[nvidia/model(low-xhigh), model2(medium-max), model3] $rag=[資料,docs] $links=[https://a.com, https://b.com] $custom=[one,two]",
    );
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    assert_eq!(p.settings.websearch, Some(false));
    assert_eq!(p.settings.subagents, Some(true));
    let models = p.settings.subagent_models.unwrap();
    assert_eq!(models[0].model, "nvidia/model");
    assert_eq!(models[0].min_effort.as_deref(), Some("low"));
    assert_eq!(models[0].max_effort.as_deref(), Some("xhigh"));
    assert!(models[2].min_effort.is_none());
    assert_eq!(p.settings.rag.unwrap(), ["資料", "docs"]);
    assert_eq!(
        p.settings.links.unwrap(),
        ["https://a.com", "https://b.com"]
    );
    assert_eq!(
        p.settings.extra["custom"],
        SettingValue::List(vec!["one".into(), "two".into()])
    );
    assert_eq!(
        parse("$rag=[one,two]").settings,
        parse("$rag=[one, two]").settings
    );
    assert_eq!(
        parse("$rag=[\"one,two\", 'three four']")
            .settings
            .rag
            .unwrap(),
        ["one,two", "three four"]
    );
    assert_eq!(
        parse("$subagents-model=[model(low-to-xhigh)]")
            .settings
            .subagent_models
            .unwrap()[0]
            .max_effort
            .as_deref(),
        Some("xhigh")
    );
}
#[test]
fn settings_persist_until_changed_settings_only_do_not_start_reply_and_errors_are_atomic() {
    let mut app = App::default();
    app.input.set("$websearch $subagents=off $rag=[one, two]");
    app.submit();
    assert!(!app.working);
    assert!(app.submissions.is_empty());
    assert_eq!(app.prompt_settings.websearch, Some(true));
    app.input.set("normal prompt");
    app.submit();
    assert_eq!(app.submissions[0].settings, app.prompt_settings);
    app.input.set("$websearch=off $links=[https://a.com]");
    app.submit();
    assert!(app.working);
    assert_eq!(app.submissions.len(), 1);
    assert_eq!(app.prompt_settings.websearch, Some(false));
    assert_eq!(app.submissions[0].settings.websearch, Some(true));
    key(&mut app, KeyCode::Esc);
    app.input.set("$rag=[] next prompt");
    app.submit();
    assert_eq!(app.submissions[1].settings.rag, Some(vec![]));
    assert_eq!(app.submissions[1].settings.websearch, Some(false));
    key(&mut app, KeyCode::Esc);
    let old = app.prompt_settings.clone();
    app.input
        .set("$websearch=on $subagents-model=[model(max-low)]");
    let draft = app.input.clone();
    app.submit();
    assert_eq!(app.prompt_settings, old);
    assert_eq!(app.input, draft);
    assert!(!app.notice.is_empty());
    assert!(!app.working);
    app.command("/clear");
    assert_eq!(app.prompt_settings, old);
}
#[test]
fn malformed_settings_are_rejected_and_code_payloads_and_urls_do_not_execute() {
    for text in [
        "$websearch=maybe",
        "$rag",
        "$rag=[a,,b]",
        "$rag=[a,]",
        "$rag=[a",
        "$subagents-model=[model(unknown-high)]",
        "$subagents-model=[model(max-low)]",
        "$links=[[a]]",
        "$links=[a]junk",
    ] {
        assert!(!parse(text).errors.is_empty(), "accepted {text}");
    }
    let fence = char::from(96).to_string();
    for text in [
        format!("{fence}$websearch{fence}"),
        format!("{fence}{fence}{fence}\n$subagents\n{fence}{fence}{fence}"),
        "https://a.com/$websearch cost $5 \\$websearch".into(),
    ] {
        let p = parse(&text);
        assert_eq!(p.settings, Default::default(), "{text}");
    }
    let mut app = App::default();
    app.paste(&format!("$websearch {}", "x".repeat(501)));
    let r = parse_submission(&app.input);
    assert_eq!(r.settings, Default::default());
    let mut c = Composer::default();
    c.set("$rag=[a, /goal, @file] normal");
    let r = parse_submission(&c);
    assert!(!r.goal);
    assert!(r.files.is_empty());
}
#[test]
fn dollar_completion_supports_keyboard_mouse_wheel_list_templates_and_existing_values() {
    let mut app = App::default();
    app.input.set("$");
    assert_eq!(app.suggestions().len(), 6);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text, "$websearch ");
    assert!(!app.working);
    app.input.set("$rag");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text, "$rag=[");
    assert!(!app.completion_open());
    app.input.set("$web=off");
    app.input.cursor = 4;
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text, "$websearch=off");
    app.input.set("Check $");
    app.selected = 0;
    let (map, _) = draw(&app, 100, 30);
    let panel = map.completion_rect.unwrap();
    MouseController::default().event(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panel.x,
            row: panel.y,
            modifiers: KeyModifiers::NONE,
        },
        &map,
        &mut app,
    );
    assert_eq!(app.selected, 1);
    assert_eq!(app.scroll, 0);
}

#[test]
fn exact_bare_toggle_submits_with_one_return_and_status_reports_saved_values() {
    let mut app = App::default();
    app.input.set("$websearch");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.prompt_settings.websearch, Some(true));
    assert!(app.input.text.is_empty());
    assert!(!app.working);
    app.input.set("$subagents=off $rag=[docs,source]");
    key(&mut app, KeyCode::Enter);
    app.command("/status");
    app.scroll = 100;
    let text = ui::dump(&app, 100, 30).unwrap();
    assert!(text.contains("$websearch=on"));
    assert!(text.contains("$subagents=off"));
    assert!(text.contains("$rag=[docs, source]"));
    assert!(text.contains("services not connected"));
}

#[test]
fn screenshot_multiline_settings_update_session_and_submission_and_later_prompts() {
    let mut app = App::default();
    app.input
        .set("$websearch $subagents $rag=[rag1, rag2]\nhello");
    key(&mut app, KeyCode::Enter);
    assert!(app.working);
    assert_eq!(app.prompt_settings.websearch, Some(true));
    assert_eq!(app.prompt_settings.subagents, Some(true));
    assert_eq!(
        app.prompt_settings.rag,
        Some(vec!["rag1".into(), "rag2".into()])
    );
    assert_eq!(app.submissions[0].settings, app.prompt_settings);
    assert_eq!(
        app.entries[0].draft.as_ref().unwrap().text,
        "$websearch $subagents $rag=[rag1, rag2]\nhello"
    );
    key(&mut app, KeyCode::Esc);
    app.input.set("next prompt");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.submissions[1].settings, app.submissions[0].settings);
}

#[test]
fn session_skills_apply_in_multiline_prompts_persist_replace_clear_and_show_in_completion() {
    let mut app = App::default();
    app.input.set("$websearch $skills=[lint, review]\nhello");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.prompt_settings.skills,
        Some(vec!["lint".into(), "review".into()])
    );
    assert_eq!(
        app.submissions[0].settings.skills,
        app.prompt_settings.skills
    );
    key(&mut app, KeyCode::Esc);
    app.input.set("$");
    let items = app.suggestions();
    assert!(
        items
            .iter()
            .find(|s| s.label == "$skills")
            .unwrap()
            .detail
            .starts_with("[lint, review]")
    );
    assert!(
        items
            .iter()
            .find(|s| s.label == "$websearch")
            .unwrap()
            .detail
            .starts_with("on")
    );
    app.input.set("$skills=[test]");
    key(&mut app, KeyCode::Enter);
    assert!(!app.working);
    assert_eq!(app.prompt_settings.skills, Some(vec!["test".into()]));
    app.command("/clear");
    assert_eq!(app.prompt_settings.skills, Some(vec!["test".into()]));
    app.input.set("next");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.submissions[0].settings.skills,
        Some(vec!["test".into()])
    );
    key(&mut app, KeyCode::Esc);
    app.input.set("$skills=bad");
    let old = app.prompt_settings.clone();
    app.submit();
    assert_eq!(app.prompt_settings, old);
    assert_eq!(app.input.text, "$skills=bad");
    app.input.set("$skills=[]");
    app.submit();
    assert_eq!(app.prompt_settings.skills, Some(vec![]));
    app.input.set("$ski");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text, "$skills=[");
    assert_eq!(
        parse("$skills=[one,two]").settings.skills,
        parse("$skills=[one, two]").settings.skills
    );
}
#[test]
fn dollar_menu_selection_matches_sent_blue_and_hover_is_lighter_without_surface_changes() {
    for (rgb, color) in [(true, true), (false, true), (true, false)] {
        let mut app = App::default();
        app.theme.rgb = rgb;
        app.theme.color = color;
        app.input.set("$websearch\nhello");
        app.submit();
        app.working = false;
        let (sent_map, sent) = draw(&app, 100, 30);
        let source = sent_map.region(&TextRegionId::Conversation).unwrap();
        let first = source.cells.iter().find(|c| c.byte_offset == 0).unwrap();
        let sent_color = sent[(first.x, first.y)].fg;
        app.input.set("$");
        let (map, before) = draw(&app, 100, 30);
        let rect = map
            .targets
            .iter()
            .find(|t| t.action == HitAction::Suggestion(0))
            .unwrap()
            .rect;
        assert_eq!(before[(rect.x, rect.y)].fg, sent_color);
        if color {
            assert_eq!(
                sent_color,
                if rgb {
                    Color::Rgb(137, 180, 250)
                } else {
                    Color::LightBlue
                }
            );
        }
        app.mouse_position = Some((rect.x, rect.y));
        let (_, after) = draw(&app, 100, 30);
        if color {
            assert_eq!(
                after[(rect.x, rect.y)].fg,
                if rgb {
                    Color::Rgb(180, 210, 255)
                } else {
                    Color::LightCyan
                }
            );
            assert_ne!(after[(rect.x, rect.y)].fg, sent_color);
        }
        for (a, b) in before.content.iter().zip(&after.content) {
            assert_eq!(a.bg, b.bg);
        }
        assert_eq!(app.selected, 0);
    }
}
