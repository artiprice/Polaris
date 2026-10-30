// Unit tests of apps/polaris-cli/src/demo_gallery.rs (`tests`); compiled into that module via #[path].
use super::{DemoGallery, GalleryEffect, GalleryPage, SCENARIOS};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris_ui_cli::screen::RenderOptions;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn buffer_text(buffer: &Buffer) -> String {
    let mut rows = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let mut line = String::new();
        for x in buffer.area.x..buffer.area.right() {
            if let Some(cell) = buffer.cell((x, y)) {
                line.push_str(cell.symbol());
            }
        }
        rows.push(line);
    }
    rows.join("\n")
}

#[test]
fn preview_catalog_exposes_every_audited_scenario() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Preview);
    const AUDITED_SCENARIOS: [&str; 27] = [
        "welcome",
        "conversation",
        "working",
        "agents-working",
        "agents-mixed",
        "agents-completed",
        "agents-many",
        "messages",
        "approval",
        "diff",
        "error",
        "onboarding",
        "models",
        "providers",
        "settings",
        "sessions",
        "status",
        "usage",
        "usage-demo",
        "stats",
        "stats-stacked",
        "stats-lines",
        "stats-trends",
        "stats-details",
        "stats-demo",
        "transcript",
        "help",
    ];
    assert_eq!(SCENARIOS.len(), 29);
    for (position, expected_name) in AUDITED_SCENARIOS.iter().enumerate() {
        assert_eq!(
            SCENARIOS.get(position).map(|row| row.0),
            Some(*expected_name)
        );
    }
    for (position, (name, _description)) in SCENARIOS.iter().enumerate() {
        gallery.open(GalleryPage::Preview);
        for _ in 0..position {
            let _effect = gallery.handle_key(key(KeyCode::Down));
        }
        match gallery.handle_key(key(KeyCode::Enter)) {
            GalleryEffect::LoadScenario(selected) => {
                assert_eq!(selected, *name, "Enter must load the highlighted fixture");
            }
            effect => return Err(format!("expected fixture action, got {effect:?}").into()),
        }
    }
    assert_eq!(
        SCENARIOS.len(),
        29,
        "the 27 audited scenarios and two added fixtures stay in the gallery"
    );
    Ok(())
}

#[test]
fn app_owned_scenarios_preserve_the_current_gallery_page() {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Welcome);
    for scenario in ["questions", "editor", "working", "conversation"] {
        assert!(
            gallery.load_scenario(scenario),
            "recognized scenario: {scenario}"
        );
        assert_eq!(gallery.page(), GalleryPage::Welcome, "scenario: {scenario}");
    }
}

#[test]
fn scenario_loading_keeps_empty_and_sample_views_distinct() -> TestResult {
    let mut gallery = DemoGallery::new();
    assert!(!gallery.load_scenario("unknown"));
    assert!(gallery.stats_demo_visible());
    assert!(gallery.usage_demo_visible());
    assert!(gallery.load_scenario("stats"));
    let mut terminal = Terminal::new(TestBackend::new(112, 32))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let empty_stats = buffer_text(terminal.backend().buffer());
    assert!(empty_stats.contains("Unavailable"));
    assert!(!empty_stats.contains("DEMO DATA"));
    assert_eq!(gallery.page(), GalleryPage::Stats);
    assert!(gallery.load_scenario("stats-demo"));
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let sample_stats = buffer_text(terminal.backend().buffer());
    assert!(sample_stats.contains("DEMO DATA"));
    assert!(sample_stats.contains("Total tokens"));

    for (name, _) in SCENARIOS {
        assert!(
            gallery.load_scenario(name),
            "declared scenario {name} loads"
        );
        if *name == "stats" {
            assert!(!gallery.stats_demo_visible());
        } else if *name == "stats-demo" {
            assert!(gallery.stats_demo_visible());
            assert_eq!(gallery.stats_tab_label(), "Overview");
        } else if *name == "stats-lines" {
            assert!(gallery.stats_demo_visible());
            assert_eq!(gallery.stats_tab_label(), "Models");
            assert_eq!(gallery.stats_chart_label(), "Lines");
        } else if *name == "stats-trends" {
            assert!(gallery.stats_demo_visible());
            assert_eq!(gallery.stats_chart_label(), "Trends");
        } else if *name == "stats-details" {
            assert!(gallery.stats_demo_visible());
            assert!(gallery.stats_details_visible());
            assert_eq!(gallery.stats_tab_label(), "Overview");
        } else if *name == "usage" {
            assert!(!gallery.usage_demo_visible());
        } else if *name == "usage-demo" {
            assert!(gallery.usage_demo_visible());
        }
    }
    assert!(gallery.load_scenario("stats"));
    assert!(!gallery.stats_demo_visible());
    assert!(gallery.load_scenario("stats-demo"));
    assert!(gallery.stats_demo_visible());
    Ok(())
}

#[test]
fn paste_sanitizes_control_characters_and_obeys_field_limits() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::ProviderEdit);
    let _effect = gallery.handle_paste("A\nB\u{0000}é");
    assert_eq!(gallery.provider_draft.label, "ABé");
    let _effect = gallery.handle_paste(&"x".repeat(300));
    assert_eq!(gallery.provider_draft.label.chars().count(), 256);
    gallery.open(GalleryPage::Sessions);
    let _effect = gallery.handle_paste("stream\ning");
    let mut terminal = Terminal::new(TestBackend::new(112, 30))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    assert!(output.contains("Explain the streaming parser"));
    assert!(!output.contains("Refactor request validation"));
    Ok(())
}

#[test]
fn model_picker_returns_a_typed_model_and_effort() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Model);
    let _effect = gallery.handle_key(key(KeyCode::Down));
    let _effect = gallery.handle_key(key(KeyCode::Right));
    match gallery.handle_key(key(KeyCode::Enter)) {
        GalleryEffect::ModelSelected(selection) => {
            assert_eq!(selection.id, "nvidia/nvidia-nemotron-nano-9b-v2");
            assert_eq!(selection.name, "Nemotron Nano 9B v2");
            assert_eq!(selection.effort.name(), "xhigh");
        }
        effect => return Err(format!("expected model selection, got {effect:?}").into()),
    }
    Ok(())
}

#[test]
fn approval_buttons_make_only_a_typed_mock_decision() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Approval);
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let approve = gallery.hit_region(0).ok_or("Approve should be clickable")?;
    assert_eq!(
        gallery.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: approve.x,
            row: approve.y,
            modifiers: KeyModifiers::NONE,
        }),
        GalleryEffect::ApprovalDecision(true)
    );
    let _effect = gallery.handle_key(key(KeyCode::Down));
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::ApprovalDecision(false)
    );
    Ok(())
}

#[test]
fn agent_roster_routes_each_reference_fixture_by_name() -> TestResult {
    let expected = [
        ("agents-working", 2),
        ("agents-mixed", 7),
        ("agents-completed", 2),
        ("agents-many", 9),
    ];
    let mut gallery = DemoGallery::new();
    for (index, (scenario, row_count)) in expected.iter().enumerate() {
        gallery.open(GalleryPage::Agents);
        for _ in 0..index {
            let _effect = gallery.handle_key(key(KeyCode::Down));
        }
        match gallery.handle_key(key(KeyCode::Enter)) {
            GalleryEffect::LoadScenario(selected) => {
                assert_eq!(selected, *scenario);
                assert!(gallery.load_scenario(selected));
                assert_eq!(gallery.agent_fixture_rows().len(), *row_count);
                assert_eq!(gallery.page(), GalleryPage::Agents);
                let mut terminal = Terminal::new(TestBackend::new(112, 24))?;
                terminal.draw(|frame| {
                    gallery.render(frame, frame.area(), RenderOptions::default());
                })?;
                let output = buffer_text(terminal.backend().buffer());
                assert!(output.contains(scenario));
                assert!(output.contains("Registry"));
                let last = row_count.saturating_sub(1);
                let hit = gallery
                    .hit_region(last)
                    .ok_or("visible agent rows need hits")?;
                match gallery.handle_mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: hit.x,
                    row: hit.y,
                    modifiers: KeyModifiers::NONE,
                }) {
                    GalleryEffect::ViewedConversation { title, owner, .. } => {
                        let row = gallery
                            .agent_fixture_rows()
                            .get(last)
                            .ok_or("selected agent row should exist")?;
                        assert_eq!(title, row.task);
                        assert_eq!(owner, row.name);
                    }
                    effect => {
                        return Err(
                            format!("expected agent conversation view, got {effect:?}").into()
                        );
                    }
                }
            }
            effect => {
                return Err(format!("expected agent fixture route, got {effect:?}").into());
            }
        }
    }
    Ok(())
}

#[test]
fn mixed_agent_fixture_preserves_all_seven_statuses_and_unknown_token_counts() -> TestResult {
    let mut gallery = DemoGallery::new();
    assert!(gallery.load_scenario("agents-mixed"));
    let rows = gallery.agent_fixture_rows();
    assert_eq!(rows.len(), 7);
    let statuses: Vec<super::AgentStatus> = rows.iter().map(|row| row.status).collect();
    assert_eq!(
        statuses,
        [
            super::AgentStatus::Working,
            super::AgentStatus::Waiting,
            super::AgentStatus::Paused,
            super::AgentStatus::Queued,
            super::AgentStatus::Completed,
            super::AgentStatus::Failed,
            super::AgentStatus::Cancelled,
        ]
    );
    assert_eq!(rows.get(1).map(|row| row.tokens), Some(None));
    assert_eq!(rows.first().map(|row| row.tokens), Some(Some(12_400)));
    assert_eq!(
        rows.get(5).and_then(|row| row.failure_note),
        Some("This is a labeled failure fixture; no backend request failed.")
    );
    assert!(
        rows.first()
            .is_some_and(|row| row.transcript_summary.contains("typed entries"))
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 18))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    for status in [
        "Working",
        "Waiting",
        "Paused",
        "Queued",
        "Completed",
        "Failed",
        "Cancelled",
    ] {
        assert!(
            output.contains(status),
            "visible mixed fixture includes {status}"
        );
    }
    assert!(output.contains("Sample failure [demo]"), "{output}");
    Ok(())
}

#[test]
fn completed_and_many_agent_fixtures_keep_counts_statuses_and_scroll_hits() -> TestResult {
    let mut gallery = DemoGallery::new();
    assert!(gallery.load_scenario("agents-completed"));
    assert!(
        gallery
            .agent_fixture_rows()
            .iter()
            .all(|row| row.status == super::AgentStatus::Completed)
    );
    assert_eq!(gallery.agent_fixture_rows().len(), 2);

    assert!(gallery.load_scenario("agents-many"));
    assert_eq!(gallery.agent_fixture_rows().len(), 9);
    for _ in 0..8 {
        let _effect = gallery.handle_key(key(KeyCode::Down));
    }
    let mut terminal = Terminal::new(TestBackend::new(80, 9))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    assert!(output.contains("Review 7"), "{output}");
    let last_visible = gallery
        .hit_region(4)
        .ok_or("last visible scrolled agent row should have a target")?;
    match gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: last_visible.x,
        row: last_visible.y,
        modifiers: KeyModifiers::NONE,
    }) {
        GalleryEffect::ViewedConversation { owner, .. } => assert_eq!(owner, "Review 7"),
        effect => return Err(format!("expected scrolled agent view, got {effect:?}").into()),
    }
    Ok(())
}

#[test]
fn statistics_controls_change_view_range_chart_details_and_bucket() {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    let _effect = gallery.handle_key(key(KeyCode::Tab));
    assert_eq!(gallery.stats_tab_label(), "Models");
    let _effect = gallery.handle_key(key(KeyCode::Char('r')));
    assert_eq!(gallery.stats_range_label(), "Last 7 days");
    let _effect = gallery.handle_key(key(KeyCode::Char('c')));
    assert_eq!(gallery.stats_chart_label(), "Lines");
    let _effect = gallery.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(gallery.stats_details_visible());
    let _effect = gallery.handle_key(key(KeyCode::Char(']')));
    assert!(
        gallery.stats_bucket().is_some(),
        "Right selects a date bucket"
    );
}

#[test]
fn usage_and_settings_choices_are_functional_and_explicitly_local() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Usage);
    assert!(gallery.usage_demo_visible());
    let _effect = gallery.handle_key(key(KeyCode::Char('d')));
    assert!(!gallery.usage_demo_visible());
    let _effect = gallery.handle_key(key(KeyCode::Char('d')));
    assert!(gallery.usage_demo_visible());
    let _effect = gallery.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(gallery.usage_details_visible());
    gallery.open(GalleryPage::Settings);
    for _ in 0..3 {
        let _effect = gallery.handle_key(key(KeyCode::Tab));
    }
    match gallery.handle_key(key(KeyCode::Enter)) {
        GalleryEffect::SettingSelected { .. } => {}
        effect => {
            return Err(format!("expected a local settings effect, got {effect:?}").into());
        }
    }
    Ok(())
}

#[test]
fn settings_style_rows_are_distinct_live_choices() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Settings);
    let mut terminal = Terminal::new(TestBackend::new(112, 32))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let appearance = buffer_text(terminal.backend().buffer());
    assert!(appearance.contains("Appearance"));
    let _effect = gallery.handle_key(key(KeyCode::Down));
    assert_eq!(gallery.settings_category_label(), "Usage");
    let _effect = gallery.handle_key(key(KeyCode::Enter));
    let _effect = gallery.handle_key(key(KeyCode::Down));
    let _effect = gallery.handle_key(key(KeyCode::Enter));
    assert_eq!(gallery.usage_bar_style(), super::UsageBarStyle::Segments);
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let usage = buffer_text(terminal.backend().buffer());
    assert!(usage.contains("Ticks"));
    assert!(usage.contains("Strip"));
    assert!(usage.contains("Segments"));
    assert!(!usage.contains("Illustrative preview bars"));

    gallery.open(GalleryPage::Settings);
    let _effect = gallery.handle_key(key(KeyCode::Down));
    let _effect = gallery.handle_key(key(KeyCode::Down));
    let _effect = gallery.handle_key(key(KeyCode::Enter));
    let _effect = gallery.handle_key(key(KeyCode::Down));
    let _effect = gallery.handle_key(key(KeyCode::Down));
    match gallery.handle_key(key(KeyCode::Enter)) {
        GalleryEffect::SettingSelected { key, value } => {
            assert_eq!(key, "stats_chart_style");
            assert_eq!(value, "Trends");
        }
        effect => return Err(format!("expected chart style effect, got {effect:?}").into()),
    }
    assert_eq!(gallery.stats_chart_label(), "Trends");
    Ok(())
}

#[test]
fn visible_mouse_targets_dispatch_and_small_viewports_have_no_hidden_hits() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Preview);
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| {
        gallery.render(frame, frame.area(), RenderOptions::default());
    })?;
    let hit = gallery
        .hit_region(0)
        .ok_or("first visible scenario should have a hit region")?;
    match gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    }) {
        GalleryEffect::LoadScenario("welcome") => {}
        effect => {
            return Err(format!("click did not load the visible scenario: {effect:?}").into());
        }
    }
    let mut tiny = Terminal::new(TestBackend::new(1, 1))?;
    gallery.open(GalleryPage::Preview);
    tiny.draw(|frame| {
        gallery.render(frame, Rect::new(0, 0, 1, 1), RenderOptions::default());
    })?;
    assert!(
        gallery.hit_region(0).is_none(),
        "clipped rows must not retain click targets"
    );
    Ok(())
}

#[test]
fn renderer_uses_the_dark_reference_palette_and_selected_green_accent() -> TestResult {
    use ratatui::style::Color;

    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Preview);
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let buffer = terminal.backend().buffer();
    let title = buffer.cell((0, 0)).ok_or("title cell should be visible")?;
    let rule = buffer.cell((0, 1)).ok_or("rule cell should be visible")?;
    let selected = buffer
        .cell((1, 3))
        .ok_or("selected scenario row should be visible")?;
    assert_eq!(title.fg, Color::Rgb(232, 232, 232));
    assert_eq!(title.bg, Color::Rgb(0, 0, 0));
    assert_eq!(rule.fg, Color::Rgb(52, 52, 52));
    assert_eq!(selected.fg, Color::Rgb(118, 185, 0));
    assert_eq!(selected.bg, Color::Rgb(0, 0, 0));
    Ok(())
}

#[test]
fn mouse_hits_are_invalidated_after_navigation() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Preview);
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    terminal.draw(|frame| {
        gallery.render(frame, frame.area(), RenderOptions::default());
    })?;
    let hit = gallery
        .hit_region(0)
        .ok_or("first visible scenario should have a hit region")?;
    let _effect = gallery.handle_key(key(KeyCode::Down));
    assert!(
        gallery
            .handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.x,
                row: hit.y,
                modifiers: KeyModifiers::NONE,
            })
            .is_consumed(),
        "stale hit geometry must not activate the old selection"
    );
    Ok(())
}
#[test]
fn usage_starts_with_labeled_fixture_and_details_toggle() -> TestResult {
    use ratatui::style::Color;

    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Usage);
    assert!(
        gallery.usage_demo_visible(),
        "the audited Usage screen opens on labeled preview metrics"
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 40))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    assert!(output.contains("DEMO DATA"));
    assert!(output.contains("Total cost"));
    assert!(output.contains("USD 0.42"));
    assert!(output.contains("15,580 total / 12,400 in / 3,180 out"));
    assert!(output.contains("2m 14s wall / 1m 56s API"));
    assert!(output.contains("2 files / +14 -3"));
    assert!(output.contains("Current window"));
    assert!(output.contains("20% used"));
    assert!(output.contains("25,600 / 128,000 tokens"));
    assert!(output.contains("Resets in 2h 30m"));
    assert!(output.contains("Resets in 4d"));
    assert!(output.contains("Nemotron Super 49B v1.5"));
    assert!(output.contains("12,400 in / 3,180 out"));
    let active = terminal
        .backend()
        .buffer()
        .cell((2, 10))
        .ok_or("active usage bar cell should be visible")?;
    let marker = terminal
        .backend()
        .buffer()
        .cell((11, 10))
        .ok_or("usage leading marker should be visible")?;
    assert_eq!(active.fg, Color::Rgb(118, 185, 0));
    assert_eq!(marker.fg, Color::Rgb(192, 229, 140));
    let _effect = gallery.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(gallery.usage_details_visible());
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let detailed = buffer_text(terminal.backend().buffer());
    assert!(detailed.contains("USD 0.42"));
    assert!(detailed.contains("Cache"));
    assert!(detailed.contains("100,000 / 2,000 tokens"));
    assert!(detailed.contains("90% / 90 of 100 requests"));
    assert!(detailed.contains("General purpose / 2 calls / 2,200 tokens"));
    assert!(detailed.contains("Explore / 1 call / 880 tokens"));
    assert!(detailed.contains("Included in session/model totals."));

    let mut narrow = DemoGallery::new();
    narrow.open(GalleryPage::Usage);
    let _effect = narrow.handle_key(key(KeyCode::Char(' ')));
    let mut narrow_terminal = Terminal::new(TestBackend::new(80, 40))?;
    narrow_terminal.draw(|frame| {
        narrow.render(frame, frame.area(), RenderOptions::default());
    })?;
    let narrow_output = buffer_text(narrow_terminal.backend().buffer());
    assert!(narrow_output.contains("Included in session/model totals."));
    assert!(narrow_output.contains("Resets in 2h 30m"));
    Ok(())
}

#[test]
fn statistics_matches_two_tabs_ranges_and_labeled_shared_fixture() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    assert_eq!(gallery.stats_tab_label(), "Overview");
    assert_eq!(gallery.stats_range_label(), "All time");
    let mut terminal = Terminal::new(TestBackend::new(112, 42))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let overview = buffer_text(terminal.backend().buffer());
    assert!(overview.contains("DEMO DATA"));
    assert!(overview.contains("Activity / sessions per day"));
    assert!(overview.contains("Nemotron Super 49B v1.5"));
    let _effect = gallery.handle_key(key(KeyCode::Tab));
    assert_eq!(gallery.stats_tab_label(), "Models");
    let _effect = gallery.handle_key(key(KeyCode::Tab));
    assert_eq!(gallery.stats_tab_label(), "Overview", "Stats has two tabs");
    let _effect = gallery.handle_key(key(KeyCode::Char('r')));
    assert_eq!(gallery.stats_range_label(), "Last 7 days");
    let _effect = gallery.handle_key(key(KeyCode::Tab));
    let _effect = gallery.handle_key(key(KeyCode::Char('c')));
    assert_eq!(gallery.stats_chart_label(), "Lines");
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let chart = buffer_text(terminal.backend().buffer());
    assert!(chart.contains("weekly totals") || chart.contains("daily totals"));
    assert!(chart.contains("Nemotron Nano 9B v2"));
    Ok(())
}

#[test]
fn stats_charts_render_time_buckets_and_split_without_dropping_the_tail() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    let _effect = gallery.handle_key(key(KeyCode::Tab));
    let mut terminal = Terminal::new(TestBackend::new(112, 42))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let stacked = buffer_text(terminal.backend().buffer());
    assert!(stacked.contains("Stacked buckets"));
    assert!(stacked.contains("Timeline panel 1/2"));
    assert!(stacked.contains("Timeline panel 2/2"));
    let _effect = gallery.handle_key(key(KeyCode::Char('c')));
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let lines = buffer_text(terminal.backend().buffer());
    assert!(lines.contains("Lines use one shared scale"));
    assert!(lines.contains("Timeline panel 2/2"));
    let _effect = gallery.handle_key(key(KeyCode::Char('c')));
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let trends = buffer_text(terminal.backend().buffer());
    assert!(trends.contains("Trends use independent model scales"));
    assert!(trends.contains("Timeline panel 2/2"));
    Ok(())
}

#[test]
fn normalized_stats_fixture_reconciles_with_reference_totals() {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    let all = gallery.stats_summary();
    assert_eq!(all.sessions, 892);
    assert_eq!(all.active_days, 253);
    assert_eq!(all.tokens, 76_965_087);
    assert_eq!(all.model_tokens, [36_183_635, 23_262_466, 17_518_986]);
    let _effect = gallery.handle_key(key(KeyCode::Char('r')));
    let week = gallery.stats_summary();
    assert_eq!(week.sessions, 14);
    assert_eq!(week.tokens, 279_588);
    assert_eq!(week.model_tokens, [147_708, 82_700, 49_180]);
}

#[test]
fn usage_bar_styles_have_distinct_shapes_and_respect_empty_capacity() {
    let ticks = super::usage_bar(super::UsageBarStyle::Ticks, 2, 4, 8, true);
    let strip = super::usage_bar(super::UsageBarStyle::Strip, 2, 4, 8, true);
    let segments = super::usage_bar(super::UsageBarStyle::Segments, 2, 4, 8, true);
    assert_eq!(ticks.chars().count(), 8);
    assert_eq!(strip.chars().count(), 8);
    assert_eq!(segments.chars().count(), 8);
    assert_ne!(ticks, strip);
    assert_ne!(strip, segments);
    assert_eq!(
        super::usage_bar(super::UsageBarStyle::Strip, 0, 0, 8, true),
        "Unavailable"
    );
}

#[test]
fn stats_mouse_tab_targets_follow_current_rendered_geometry() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    let mut terminal = Terminal::new(TestBackend::new(100, 32))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let target = gallery
        .hit_region(1)
        .ok_or("Models tab should have a click target")?;
    let _effect = gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: target.x,
        row: target.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(gallery.stats_tab_label(), "Models");

    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let week = gallery
        .hit_region(3)
        .ok_or("Last 7 days range should have a click target")?;
    let _effect = gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: week.x,
        row: week.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(gallery.stats_range_label(), "Last 7 days");

    let stale = target;
    gallery.open(GalleryPage::Usage);
    let _effect = gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: stale.x,
        row: stale.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(gallery.page(), GalleryPage::Usage);
    assert_eq!(gallery.stats_tab_label(), "Models");
    Ok(())
}

#[test]
fn calendar_click_selects_a_real_day_and_opens_its_inspector() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    let mut terminal = Terminal::new(TestBackend::new(112, 42))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let day = gallery
        .hit_region(5)
        .ok_or("first visible calendar day should have a hit target")?;
    let _effect = gallery.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: day.x,
        row: day.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(gallery.stats_bucket(), Some(0));
    let _effect = gallery.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    assert!(output.contains("Inspector · Oct 19, 2025 ·"));
    Ok(())
}

#[test]
fn normalized_fixture_calendar_uses_reference_start_and_end_dates() {
    assert_eq!(super::stats_date(0), "Oct 19, 2025");
    assert_eq!(super::stats_date(364), "Oct 18, 2026");
}

#[test]
fn provider_picker_opens_reference_form_and_session_rows_open_a_fixture() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::ProviderPicker);
    for _ in 0..1 {
        let _effect = gallery.handle_key(key(KeyCode::Down));
    }
    let _effect = gallery.handle_key(key(KeyCode::Enter));
    assert_eq!(gallery.page(), GalleryPage::Model);
    assert!(
        gallery.selected_model().is_none(),
        "non-NVIDIA rows must not expose NVIDIA model ids"
    );
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::Consumed
    );
    assert_eq!(gallery.page(), GalleryPage::ProviderEdit);

    gallery.open(GalleryPage::Sessions);
    match gallery.handle_key(key(KeyCode::Enter)) {
        GalleryEffect::ViewedConversation {
            scenario, title, ..
        } => {
            assert_eq!(scenario, "conversation");
            assert_eq!(title, "Refactor request validation");
        }
        effect => {
            return Err(format!("session activation should load a fixture, got {effect:?}").into());
        }
    }
    Ok(())
}

#[test]
fn nested_provider_navigation_restores_model_and_configured_draft_is_retained() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Model);
    let _effect = gallery.handle_key(key(KeyCode::Char('p')));
    assert_eq!(gallery.page(), GalleryPage::ProviderPicker);
    assert_eq!(
        gallery.handle_key(key(KeyCode::Esc)),
        GalleryEffect::Consumed
    );
    assert_eq!(gallery.page(), GalleryPage::Model);
    assert_eq!(gallery.handle_key(key(KeyCode::Esc)), GalleryEffect::Close);

    gallery.open(GalleryPage::Providers);
    for _ in 0..4 {
        let _effect = gallery.handle_key(key(KeyCode::Down));
    }
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::Consumed
    );
    assert_eq!(gallery.page(), GalleryPage::ProviderEdit);
    let _effect = gallery.handle_key(key(KeyCode::Char('D')));
    for _ in 0..4 {
        let _effect = gallery.handle_key(key(KeyCode::Tab));
    }
    let saved = gallery.handle_key(key(KeyCode::Enter));
    assert!(matches!(saved, GalleryEffect::ProviderSaved(_)));
    gallery.open(GalleryPage::Providers);
    for _ in 0..4 {
        let _effect = gallery.handle_key(key(KeyCode::Down));
    }
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::Consumed
    );
    assert_eq!(gallery.provider_draft.label, "D");
    assert_eq!(
        gallery.handle_key(key(KeyCode::Esc)),
        GalleryEffect::Consumed
    );
    assert_eq!(gallery.page(), GalleryPage::Providers);
    Ok(())
}

#[test]
fn info_pages_enter_closes_and_primary_detail_hotkeys_remain_distinct() {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Stats);
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::Close
    );
    gallery.open(GalleryPage::Usage);
    assert!(!gallery.usage_details_visible());
    assert_eq!(
        gallery.handle_key(key(KeyCode::Enter)),
        GalleryEffect::Close
    );
    let _effect = gallery.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(gallery.usage_details_visible());
}

#[test]
fn sessions_search_filters_rows_and_settings_exposes_six_categories() -> TestResult {
    let mut gallery = DemoGallery::new();
    gallery.open(GalleryPage::Sessions);
    for ch in "streaming".chars() {
        let _effect = gallery.handle_key(key(KeyCode::Char(ch)));
    }
    let mut terminal = Terminal::new(TestBackend::new(112, 30))?;
    terminal.draw(|frame| gallery.render(frame, frame.area(), RenderOptions::default()))?;
    let output = buffer_text(terminal.backend().buffer());
    assert!(output.contains("Explain the streaming parser"));
    assert!(!output.contains("Refactor request validation"));
    gallery.open(GalleryPage::Settings);
    assert_eq!(gallery.settings_categories().len(), 6);
    for _ in 0..5 {
        let _effect = gallery.handle_key(key(KeyCode::Down));
    }
    assert_eq!(gallery.settings_category_label(), "Providers");
    Ok(())
}
