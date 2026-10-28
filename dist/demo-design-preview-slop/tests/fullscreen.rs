use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{
    app::{App, Entry, EntryKind, Screen},
    theme::ThemeMode,
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};

fn buffer(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn textarea_has_horizontal_rules_open_sides_and_keeps_chrome_outside_source_at_every_size() {
    for scenario in ["welcome", "conversation", "working", "approval", "error"] {
        for (w, h) in [(160, 60), (120, 40), (100, 30), (80, 24), (60, 20)] {
            let mut app = App::default();
            app.load_scenario(scenario).unwrap();
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut map = polaris::interaction::UiMap::default();
            t.draw(|frame| map = ui::render_interactive(frame, &app))
                .unwrap();
            let b = t.backend().buffer();
            let r = map
                .region(&polaris::interaction::TextRegionId::Composer)
                .unwrap()
                .rect;
            let column = ui::content_column(b.area);
            for x in column.x..column.right() {
                assert_eq!(b[(x, r.y - 1)].symbol(), "─");
                assert_eq!(b[(x, h - 3)].symbol(), "─");
            }
            for y in r.y..r.bottom() {
                assert_eq!(b[(column.x, y)].symbol(), " ");
                assert_eq!(b[(column.right() - 1, y)].symbol(), " ");
            }
            assert_eq!(b[(r.x - 2, r.y)].symbol(), ">");
            assert!(r.bottom() <= h - 3);
            assert!(
                map.region(&polaris::interaction::TextRegionId::Composer)
                    .unwrap()
                    .cells
                    .iter()
                    .all(|c| r.contains((c.x, c.y).into()) || c.x == r.right())
            );
            let text = ui::dump(&app, w, h).unwrap();
            assert!(text.lines().last().unwrap().contains("[ auto mode on ]"));
            app.ascii = true;
            let ascii = ui::dump(&app, w, h).unwrap();
            assert!(ascii.is_ascii());
            assert!(ascii.contains(&"-".repeat(column.width as usize)));
        }
    }
}

#[test]
fn composer_stays_at_the_bottom_and_scrolls_multiline_input_to_its_cursor() {
    let mut app = App::default();
    app.input.set(&"textarea line\n".repeat(12));
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut map = polaris::interaction::UiMap::default();
    t.draw(|frame| map = ui::render_interactive(frame, &app))
        .unwrap();
    let r = map
        .region(&polaris::interaction::TextRegionId::Composer)
        .unwrap()
        .rect;
    let cursor = t.get_cursor_position().unwrap();
    assert_eq!(cursor.y, 20);
    assert_eq!(cursor.x, 6);
    assert_eq!(r.height, 9);
    let b = t.backend().buffer();
    assert!((r.x..r.right()).all(|x| b[(x, 20)].symbol().trim().is_empty()));
    assert!(
        (r.x..r.right())
            .map(|x| b[(x, 19)].symbol())
            .collect::<String>()
            .contains("textarea line")
    );
    let rows = ui::dump(&app, 80, 24).unwrap();
    let rows = rows.lines().collect::<Vec<_>>();
    assert!(rows[22].is_empty());
    assert!(rows[23].contains("[ auto mode on ]"));
}

#[test]
fn selection_surfaces_are_confined_to_interactive_controls_and_patch_data() {
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        for color in [true, false] {
            for scenario in [
                "preview",
                "settings",
                "models",
                "providers",
                "sessions",
                "diff",
                "approval",
                "commands",
                "provider-edit",
            ] {
                let mut app = App::default();
                app.theme.mode = mode;
                app.theme.color = color;
                match scenario {
                    "preview" => app.command("/preview"),
                    "commands" => app.input.set("/"),
                    "provider-edit" => app.start_provider_edit(0),
                    other => app.load_scenario(other).unwrap(),
                }
                if scenario == "settings" {
                    app.settings.categories_focused = false;
                }
                let before = buffer(&app, 120, 40);
                match scenario {
                    "diff" => app.tab = 1,
                    "models" => app.selected = (app.selected + 1) % app.model_rows().len(),
                    "approval" => app.approval_selected = 2,
                    _ => app.selected = 1,
                }
                let after = buffer(&app, 120, 40);
                let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
                let mut allowed = polaris::interaction::UiMap::default();
                terminal
                    .draw(|frame| allowed = ui::render_interactive(frame, &app))
                    .unwrap();
                for (index, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
                    if scenario == "diff" && color && a.bg != b.bg {
                        // Changing files changes actual patch data, including red/green row surfaces.
                        let (x, y) = ((index % 120) as u16, (index / 120) as u16);
                        let sheet = ui::popup_area(&app, ratatui::layout::Rect::new(0, 0, 120, 40));
                        assert!(
                            sheet.contains((x, y).into()),
                            "diff color escaped its sheet"
                        );
                        let allowed = [
                            app.theme.base().bg.unwrap_or(ratatui::style::Color::Reset),
                            app.theme.diff_added().bg.unwrap(),
                            app.theme.diff_removed().bg.unwrap(),
                        ];
                        assert!(allowed.contains(&a.bg) && allowed.contains(&b.bg));
                    } else if color && mode == ThemeMode::Dark && a.bg != b.bg {
                        let p = ((index % 120) as u16, (index / 120) as u16);
                        assert!(
                            allowed
                                .targets
                                .iter()
                                .any(|target| target.rect.contains(p.into())),
                            "control fill escaped an interaction target"
                        );
                        let fills = [
                            Color::Rgb(0, 0, 0),
                            Color::Rgb(25, 25, 25),
                            Color::Rgb(24, 24, 24),
                        ];
                        assert!(fills.contains(&a.bg) && fills.contains(&b.bg));
                    } else {
                        assert_eq!(
                            a.bg, b.bg,
                            "selection changed background in {scenario}, {mode:?}, color={color}"
                        );
                    }
                    assert!(
                        !b.modifier.contains(Modifier::REVERSED),
                        "reverse video in {scenario}"
                    );
                }
                if color {
                    assert!(
                        before
                            .content
                            .iter()
                            .zip(&after.content)
                            .any(|(a, b)| a.fg != b.fg || a.bg != b.bg || a.modifier != b.modifier),
                        "selection should change foreground in {scenario}"
                    );
                }
            }
        }
    }
}

#[test]
fn page_keys_scroll_fullscreen_history_without_disturbing_the_draft() {
    let mut app = App::default();
    app.input.set("keep this draft");
    app.entries.push(Entry {
        draft: None,
        kind: EntryKind::Note,
        title: "Demo history fixture".into(),
        lines: (0..200).map(|i| format!("history line {i:03}")).collect(),
    });
    assert!(ui::conversation_max_scroll(&app, 80, 24) > 10);
    let latest = ui::dump(&app, 80, 24).unwrap();
    assert!(latest.contains("history line 199"));
    app.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    let older = ui::dump(&app, 80, 24).unwrap();
    assert!(!older.contains("history line 199"));
    assert!(older.contains("keep this draft"));
    let scroll = app.scroll;
    app.open(Screen::Settings);
    app.close();
    assert_eq!(app.scroll, scroll);
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(ui::dump(&app, 80, 24).unwrap(), latest);
}
