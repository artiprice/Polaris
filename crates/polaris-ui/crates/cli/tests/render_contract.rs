use std::error::Error;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use polaris_ui_cli::screen::{
    CompletionOverlay, InfoSheet, PanelPlacement, PanelVisibility, RenderError, RenderOptions,
    ScreenState, composer_caret_position, composer_cursor_at, composer_vertical_cursor,
    composer_visual_guards, dump_cells, dump_text, render,
};
use polaris_ui_model::completion::{CompletionKind, Suggestion};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::view::ViewState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn fixture() -> Result<(ViewState, Composer), Box<dyn Error>> {
    let model = ModelId::new("local:test")?;
    let view = ViewState::new(model, Effort::Low);
    let composer = Composer::new();
    Ok((view, composer))
}

#[test]
fn dump_rejects_zero_and_oversized_viewports() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    assert!(matches!(
        dump_text(&state, &options, 0, 20),
        Err(RenderError::InvalidDimensions { .. })
    ));
    assert!(matches!(
        dump_text(&state, &options, 401, 200),
        Err(RenderError::InvalidDimensions { .. })
    ));
    assert!(matches!(
        dump_text(&state, &options, 400, 201),
        Err(RenderError::InvalidDimensions { .. })
    ));
    Ok(())
}

#[test]
fn tiny_live_viewport_shows_resize_notice_without_panicking() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let mut terminal = Terminal::new(TestBackend::new(24, 8))?;
    let mut metadata = None;
    terminal.draw(|frame| metadata = Some(render(frame, &state, &options)))?;
    let metadata = metadata.ok_or("render did not return metadata")?;
    assert!(metadata.viewport.width <= 24);
    assert!(metadata.viewport.height <= 8);
    let text = dump_text(&state, &options, 24, 8)?;
    assert!(text.to_lowercase().contains("resize"));
    Ok(())
}

#[test]
fn wide_grapheme_caret_geometry_matches_composer() -> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    composer.insert("A界é")?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
    let mut metadata = None;
    terminal.draw(|frame| metadata = Some(render(frame, &state, &options)))?;
    let metadata = metadata.ok_or("render did not return metadata")?;
    let caret = metadata
        .caret
        .ok_or("visible composer has no caret geometry")?;
    assert!(caret.x >= metadata.composer_rect.x);
    assert!(caret.y >= metadata.composer_rect.y);
    assert!(
        caret.x
            < metadata
                .composer_rect
                .x
                .saturating_add(metadata.composer_rect.width)
    );
    assert!(
        caret.y
            < metadata
                .composer_rect
                .y
                .saturating_add(metadata.composer_rect.height)
    );
    assert_eq!(
        Some(caret),
        composer_caret_position(&composer, metadata.composer_rect)
    );
    Ok(())
}

#[test]
fn untrusted_controls_and_bidi_are_inert_in_dump() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("safe \u{1b}]52;c;secret\u{7} text \u{202e}still readable")?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let text = dump_text(&state, &options, 80, 24)?;
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains("safe"));
    assert!(text.contains("still readable"));
    Ok(())
}

#[test]
fn optional_panel_uses_side_rail_only_at_threshold() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer).with_panel(PanelVisibility::Open);
    let options = RenderOptions::default();
    let wide = dump_cells(&state, &options, 120, 28)?;
    let wide_meta = wide.metadata;
    assert_eq!(wide_meta.panel_placement, PanelPlacement::SideRail);
    assert_eq!(wide_meta.panel_rect.map(|rect| rect.width), Some(34));
    let narrow = dump_cells(&state, &options, 119, 40)?;
    assert_eq!(narrow.metadata.panel_placement, PanelPlacement::BottomSheet);
    assert_eq!(narrow.metadata.composer_count, 1);
    let closed = ScreenState::new(&view, &composer);
    let closed = dump_cells(&closed, &options, 120, 28)?;
    assert_eq!(closed.metadata.panel_placement, PanelPlacement::Hidden);
    Ok(())
}

#[test]
fn default_canvas_uses_north_star_background() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let dump = dump_cells(&state, &options, 80, 24)?;
    let canvas = dump.cells.first().ok_or("empty cell dump")?;
    assert_eq!(canvas.bg_rgb, Some((0, 0, 0)));
    let composer_x = usize::from(dump.metadata.composer_rect.x);
    let composer_y = usize::from(dump.metadata.composer_rect.y);
    let composer_index = composer_y
        .saturating_mul(usize::from(dump.width))
        .saturating_add(composer_x);
    assert_eq!(
        dump.cells.get(composer_index).map(|cell| cell.bg_rgb),
        Some(Some((20, 20, 20)))
    );

    let mut sent_view = view.clone();
    sent_view.push_user("submitted prompt")?;
    let sent_state = ScreenState::new(&sent_view, &composer);
    let sent_dump = dump_cells(&sent_state, &options, 80, 24)?;
    let span = sent_dump
        .metadata
        .source_spans
        .first()
        .ok_or("sent prompt source span missing")?;
    let prompt_index = usize::from(span.rect.y)
        .saturating_mul(usize::from(sent_dump.width))
        .saturating_add(usize::from(span.rect.x));
    assert_eq!(
        sent_dump.cells.get(prompt_index).map(|cell| cell.bg_rgb),
        Some(Some((20, 20, 20)))
    );
    Ok(())
}

#[test]
fn opaque_overlays_remove_covered_source_spans() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        (0..80)
            .map(|n| format!("transcript row {n}\n"))
            .collect::<String>(),
    )?;
    let options = RenderOptions::default();
    let narrow = ScreenState::new(&view, &composer).with_panel(PanelVisibility::Open);
    let dump = dump_cells(&narrow, &options, 119, 40)?;
    let panel = dump.metadata.panel_rect.ok_or("panel sheet missing")?;
    let covered_y = panel.y.saturating_add(panel.height.saturating_sub(10));
    assert!(
        dump.metadata
            .source_spans
            .iter()
            .any(|span| span.rect.y < covered_y)
    );
    assert!(
        dump.metadata
            .source_spans
            .iter()
            .all(|span| span.rect.y < covered_y)
    );

    let lines = vec!["private sheet content".to_owned()];
    let sheet = InfoSheet {
        title: "Info",
        lines: &lines,
        footer: None,
        scroll: 0,
    };
    let opaque = ScreenState::new(&view, &composer).with_sheet(Some(sheet));
    let dump = dump_cells(&opaque, &options, 80, 24)?;
    assert!(dump.metadata.source_spans.is_empty());
    Ok(())
}

#[test]
fn ansi_palette_preserves_canvas_and_prompt_roles() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions {
        colors: polaris_ui_cli::screen::ColorMode::Ansi,
        ascii: false,
    };
    let dump = dump_cells(&state, &options, 80, 24)?;
    assert_eq!(
        dump.cells.first().map(|cell| cell.bg),
        Some(Color::Indexed(0))
    );
    let rect = dump.metadata.composer_rect;
    let index = usize::from(rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(rect.x));
    assert_eq!(
        dump.cells.get(index).map(|cell| cell.bg),
        Some(Color::Indexed(8))
    );
    Ok(())
}

#[test]
fn history_scroll_and_workspace_are_source_driven() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        (0..60)
            .map(|n| format!("history marker {n:02}\n"))
            .collect::<String>(),
    )?;
    let options = RenderOptions::default();
    let bottom = ScreenState::new(&view, &composer);
    let bottom = dump_cells(&bottom, &options, 80, 24)?;
    let scrolled = ScreenState::new(&view, &composer)
        .with_history_scroll(1)
        .with_workspace(Some("workspace:alpha"));
    let scrolled = dump_cells(&scrolled, &options, 80, 24)?;
    let last_bottom = bottom
        .metadata
        .source_spans
        .last()
        .ok_or("bottom source span missing")?;
    let last_scrolled = scrolled
        .metadata
        .source_spans
        .last()
        .ok_or("scrolled source span missing")?;
    assert!(last_scrolled.source_bytes.end < last_bottom.source_bytes.end);
    let text = dump_text(
        &ScreenState::new(&view, &composer).with_workspace(Some("workspace:alpha")),
        &options,
        80,
        24,
    )?;
    assert!(text.contains("workspace:alpha"));
    Ok(())
}

#[test]
fn completion_overlay_paints_selection_and_hides_underlying_sources() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        (0..60)
            .map(|n| format!("covered row {n}\n"))
            .collect::<String>(),
    )?;
    let suggestions = vec![
        Suggestion {
            label: "help".to_owned(),
            insert: "/help".to_owned(),
            description: "Show help",
            kind: CompletionKind::Command,
            range: 0..1,
        },
        Suggestion {
            label: "run".to_owned(),
            insert: "/run".to_owned(),
            description: "Run a task",
            kind: CompletionKind::Command,
            range: 0..1,
        },
    ];
    let state = ScreenState::new(&view, &composer).with_completion(Some(CompletionOverlay {
        title: "Commands",
        items: &suggestions,
        selected: Some(1),
    }));
    let options = RenderOptions::default();
    let dump = dump_cells(&state, &options, 80, 24)?;
    let text = dump_text(&state, &options, 80, 24)?;
    assert!(text.contains("Commands"));
    assert!(text.contains("> run"));
    assert!(dump.metadata.source_spans.is_empty());
    let overlay_y = dump
        .metadata
        .body
        .y
        .saturating_add(dump.metadata.body.height.saturating_sub(12))
        .saturating_add(3);
    let overlay_x = dump.metadata.body.x.saturating_add(1);
    let index = usize::from(overlay_y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(overlay_x));
    assert_eq!(
        dump.cells.get(index).map(|cell| cell.fg_rgb),
        Some(Some((118, 185, 0)))
    );
    Ok(())
}

#[test]
fn caller_selected_panel_content_renders_in_both_responsive_modes() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let lines = vec![
        "Task: inspect renderer".to_owned(),
        "Status: active".to_owned(),
    ];
    let content = InfoSheet {
        title: "Selected tasks",
        lines: &lines,
        footer: Some("Esc close"),
        scroll: 0,
    };
    let state = ScreenState::new(&view, &composer)
        .with_panel(PanelVisibility::Open)
        .with_panel_content(Some(content));
    let options = RenderOptions::default();
    let wide = dump_text(&state, &options, 120, 28)?;
    assert!(wide.contains("Selected tasks"));
    assert!(wide.contains("Task: inspect renderer"));
    assert!(wide.contains("Esc close"));

    let narrow = dump_text(&state, &options, 119, 40)?;
    assert!(narrow.contains("Selected tasks"));
    assert!(narrow.contains("Task: inspect renderer"));
    assert!(narrow.contains("Esc close"));
    Ok(())
}

#[test]
fn composer_mouse_vertical_and_guard_helpers_share_wrap_rows() -> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    composer.insert("abcdefghi\nxy")?;
    let area = ratatui::layout::Rect::new(5, 7, 8, 4);
    composer.move_end();
    assert_eq!(composer_visual_guards(&composer, area), (false, true));
    assert_eq!(composer_vertical_cursor(&composer, area, false), Some(8));
    assert_eq!(composer_cursor_at(&composer, area, 12, 7), Some(5));
    let mouse_cursor = composer_cursor_at(&composer, area, 9, 8)
        .ok_or("mouse cell did not map to a composer boundary")?;
    assert_eq!(mouse_cursor, 8);
    composer.set_cursor(mouse_cursor);
    assert_eq!(composer_vertical_cursor(&composer, area, true), Some(12));
    composer.move_home();
    assert_eq!(composer_visual_guards(&composer, area), (true, false));
    assert_eq!(composer_vertical_cursor(&composer, area, false), None);
    let _state = ScreenState::new(&view, &composer);
    Ok(())
}

#[test]
fn composer_mouse_targets_chip_boundaries_only() -> Result<(), Box<dyn Error>> {
    let (_view, mut composer) = fixture()?;
    composer.insert(&"x".repeat(501))?;
    let chip = composer
        .chips()
        .first()
        .ok_or("long paste did not create a chip")?
        .range
        .clone();
    let area = ratatui::layout::Rect::new(0, 0, 60, 5);
    let cursor = composer_cursor_at(&composer, area, 14, 0)
        .ok_or("mouse cell did not map to a chip boundary")?;
    assert!(cursor <= chip.start || cursor >= chip.end);
    Ok(())
}

#[test]
fn long_composer_window_tracks_cursor_and_renders_recent_rows() -> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    let draft = format!("{}END", "x".repeat(480));
    composer.insert(&draft)?;
    composer.move_home();
    let options = RenderOptions::default();
    let head = dump_text(&ScreenState::new(&view, &composer), &options, 80, 24)?;
    assert!(!head.contains("END"));
    composer.move_end();
    let tail = dump_text(&ScreenState::new(&view, &composer), &options, 80, 24)?;
    assert!(tail.contains("END"));
    Ok(())
}
