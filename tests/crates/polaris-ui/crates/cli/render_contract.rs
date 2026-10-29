use std::error::Error;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{Cid, FrameId, ModelId};
use polaris_ui_cli::screen::{
    ColorMode, CompletionOverlay, ComposerHit, FooterAction, HitTarget, InfoSheet, PanelPlacement,
    PanelVisibility, PresentationTool, PresentationToolStatus, RenderCache, RenderError,
    RenderOptions, ScreenState, TranscriptPoint, TranscriptSelection, TranscriptSpanSelection,
    composer_caret_position, composer_cursor_at, composer_hit_at, composer_rect,
    composer_vertical_cursor, composer_visual_guards, dump_cells, dump_text,
    footer_action_at_with_labels, footer_action_at_with_profile, prepare_metadata,
    prepare_metadata_with_cache, render,
};
use polaris_ui_model::completion::{CompletionKind, Suggestion};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::questionnaire::{
    QuestionKind, QuestionOption, QuestionSpec, QuestionnaireAction, QuestionnaireSpec,
    QuestionnaireState,
};
use polaris_ui_model::view::{ToolActivity, ToolStatus, ViewState};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

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
fn composer_rect_matches_render_metadata_without_transcript_dependency()
-> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    composer.insert(
        "a wrapped draft that contributes multiple visible rows "
            .repeat(4)
            .as_str(),
    )?;
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 100, 30);
    let state = ScreenState::new(&view, &composer)
        .with_notice(Some("status row"))
        .with_panel(PanelVisibility::Open);

    assert_eq!(
        composer_rect(&state, area),
        prepare_metadata(&state, &options, area).composer_rect
    );
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
fn assistant_markdown_hides_markers_and_keeps_prose_bullets_uncolored() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        "## Parser notes
- **Bold** claim with `fn parse()`
- second point
keep **unclosed",
    )?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let text = dump_text(&state, &options, 80, 24)?;
    assert!(text.contains("Parser notes"), "{text}");
    assert!(
        !text.contains("## Parser"),
        "heading marker is hidden: {text}"
    );
    assert!(text.contains("• Bold claim with fn parse()"), "{text}");
    assert!(text.contains("• second point"), "{text}");
    assert!(
        text.contains("keep **unclosed"),
        "unmatched markers stay: {text}"
    );
    Ok(())
}

#[test]
fn diff_fences_number_old_and_new_lines_from_hunk_headers() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        "```diff
@@ -10,3 +10,3 @@
 context
-old line
+new line
 tail
```",
    )?;
    let state = ScreenState::new(&view, &composer);
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    let row = |needle: &str| {
        text.lines()
            .find(|line| line.contains(needle))
            .map(str::to_owned)
            .unwrap_or_default()
    };
    assert!(row("context").contains("  10   10 "), "{text}");
    assert!(row("-old line").contains("  11      "), "{text}");
    assert!(row("+new line").contains("       11 "), "{text}");
    assert!(row("tail").contains("  12   12 "), "{text}");
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
        // dist/docs/ui-design.md:9 specifies a black live editor. The submitted
        // prompt assertion below deliberately retains its separate #141414 surface.
        Some(Some((0, 0, 0)))
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
    assert!(
        dump.metadata
            .source_cells
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
    assert!(dump.metadata.source_cells.is_empty());
    assert_eq!(
        prepare_metadata(&opaque, &options, ratatui::layout::Rect::new(0, 0, 80, 24)).source_cells,
        dump.metadata.source_cells
    );
    assert_eq!(
        prepare_metadata(&opaque, &options, ratatui::layout::Rect::new(0, 0, 80, 24)).source_spans,
        dump.metadata.source_spans
    );
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
        Some(Color::Indexed(0))
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
    let to_latest = scrolled
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::ToLatest)
        .ok_or("to-latest label target missing")?;
    assert_eq!(
        scrolled
            .metadata
            .source_at(to_latest.rect.x, to_latest.rect.y),
        None,
        "the overpainted label has no source hit"
    );
    assert_eq!(
        scrolled
            .metadata
            .transcript_boundary_at(to_latest.rect.x, to_latest.rect.y),
        None,
        "the to-latest label must not become a drag endpoint"
    );
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
    assert_eq!(
        prepare_metadata(&state, &options, ratatui::layout::Rect::new(0, 0, 80, 24)).source_cells,
        dump.metadata.source_cells
    );
    let text = dump_text(&state, &options, 80, 24)?;
    // Reference menus use a blank top row and foreground focus, not a titled box.
    // See @prime/evidence/ui/repair-assertion-migration.md for the source mapping.
    assert!(text.contains("run"));
    assert!(!text.contains("Commands"));
    let covered_y = dump.metadata.body.bottom().saturating_sub(12);
    assert!(
        dump.metadata
            .source_spans
            .iter()
            .all(|span| span.rect.y < covered_y)
    );
    assert!(
        dump.metadata
            .source_cells
            .iter()
            .all(|span| span.rect.y < covered_y)
    );
    let overlay_y = dump
        .metadata
        .body
        .y
        .saturating_add(dump.metadata.body.height.saturating_sub(12))
        .saturating_add(2);
    let overlay_x = dump.metadata.body.x;
    let index = usize::from(overlay_y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(overlay_x));
    assert_eq!(
        dump.cells.get(index).map(|cell| cell.fg_rgb),
        Some(Some((182, 227, 91)))
    );
    Ok(())
}

#[test]
fn completion_hover_keeps_surface_and_applies_each_semantic_foreground()
-> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let suggestions = vec![
        Suggestion {
            label: "/run".into(),
            insert: "/run".into(),
            description: "command",
            kind: CompletionKind::Command,
            range: 0..0,
        },
        Suggestion {
            label: "@user".into(),
            insert: "@user".into(),
            description: "mention",
            kind: CompletionKind::Mention,
            range: 0..0,
        },
        Suggestion {
            label: "#task".into(),
            insert: "#task".into(),
            description: "directive",
            kind: CompletionKind::Directive,
            range: 0..0,
        },
    ];
    for (index, foreground, focused) in [
        (0, (219, 245, 176), (182, 227, 91)),
        (1, (255, 253, 230), (249, 226, 175)),
        (2, (180, 210, 255), (137, 180, 250)),
    ] {
        let state = ScreenState::new(&view, &composer)
            .with_completion(Some(CompletionOverlay {
                title: "ignored",
                items: &suggestions,
                selected: None,
            }))
            .with_hover(Some(HitTarget::Completion(index)));
        let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
        let region = dump
            .metadata
            .hits
            .iter()
            .find(|hit| hit.target == HitTarget::Completion(index))
            .ok_or("completion hit missing")?;
        let cell_index = usize::from(region.rect.y)
            .saturating_mul(usize::from(dump.width))
            .saturating_add(usize::from(region.rect.x));
        let cell = dump
            .cells
            .get(cell_index)
            .ok_or("completion cell missing")?;
        assert_eq!(cell.fg_rgb, Some(foreground));
        assert_eq!(cell.bg_rgb, Some((0, 0, 0)));
        let focused_state =
            ScreenState::new(&view, &composer).with_completion(Some(CompletionOverlay {
                title: "ignored",
                items: &suggestions,
                selected: Some(index),
            }));
        let focused_dump = dump_cells(&focused_state, &RenderOptions::default(), 80, 24)?;
        let focused_region = focused_dump
            .metadata
            .hits
            .iter()
            .find(|hit| hit.target == HitTarget::Completion(index))
            .ok_or("focused completion hit missing")?;
        let row_start =
            usize::from(focused_region.rect.y).saturating_mul(usize::from(focused_dump.width));
        let row_end = row_start.saturating_add(usize::from(focused_dump.width));
        let initial = suggestions
            .get(index)
            .ok_or("completion item missing")?
            .label
            .chars()
            .next()
            .map(|ch| ch.to_string())
            .ok_or("completion label is empty")?;
        let focused_cell = focused_dump
            .cells
            .get(row_start..row_end)
            .and_then(|row| row.iter().find(|cell| cell.symbol == initial))
            .ok_or("focused completion label cell missing")?;
        assert_eq!(focused_cell.fg_rgb, Some(focused));
    }
    Ok(())
}

#[test]
fn to_latest_uses_exact_base_and_hover_surfaces() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant(
        (0..40)
            .map(|index| format!("line {index}\n"))
            .collect::<String>(),
    )?;
    let state = ScreenState::new(&view, &composer).with_history_scroll(1);
    let base = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let hit = base
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::ToLatest)
        .ok_or("to-latest hit missing")?;
    let cell_index = usize::from(hit.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(hit.rect.x));
    let hovered = dump_cells(
        &ScreenState::new(&view, &composer)
            .with_history_scroll(1)
            .with_hover(Some(HitTarget::ToLatest)),
        &RenderOptions::default(),
        80,
        24,
    )?;
    assert_eq!(
        base.cells.get(cell_index).and_then(|cell| cell.bg_rgb),
        Some((38, 38, 38))
    );
    assert_eq!(
        hovered.cells.get(cell_index).and_then(|cell| cell.bg_rgb),
        Some((55, 55, 55))
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
    // Preserve the original six-column / three-row editable area, now with
    // the reference's two rules and four-column text inset.
    let area = ratatui::layout::Rect::new(5, 7, 12, 5);
    composer.move_end();
    assert_eq!(composer_visual_guards(&composer, area), (false, true));
    assert_eq!(composer_vertical_cursor(&composer, area, false), Some(8));
    assert_eq!(composer_cursor_at(&composer, area, 14, 8), Some(5));
    let mouse_cursor = composer_cursor_at(&composer, area, 11, 9)
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
fn composer_mouse_hit_separates_source_text_from_padding_and_uses_graphemes()
-> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    let text = "a\t界e\u{301}";
    composer.insert(text)?;
    let state = ScreenState::new(&view, &composer);
    let area = Rect::new(0, 0, 80, 24);
    let rect = composer_rect(&state, area);
    let text_x = rect.x.saturating_add(4);
    let text_y = rect.y.saturating_add(1);
    assert_eq!(
        composer_hit_at(&composer, rect, text_x, text_y),
        Some(ComposerHit {
            cursor: 0,
            source_bytes: Some(0..1),
        })
    );
    let tab_hit = composer_hit_at(&composer, rect, text_x.saturating_add(2), text_y)
        .ok_or("tab hit missing")?;
    assert_eq!(tab_hit.source_bytes, Some(1..2));
    let wide_hit = composer_hit_at(&composer, rect, text_x.saturating_add(5), text_y)
        .ok_or("wide grapheme hit missing")?;
    assert_eq!(wide_hit.source_bytes, Some(2..5));
    let padding_hit = composer_hit_at(&composer, rect, text_x.saturating_add(20), text_y)
        .ok_or("padding should still map to a caret")?;
    assert_eq!(padding_hit.source_bytes, None);
    let prompt_margin_hit = composer_hit_at(&composer, rect, rect.x.saturating_add(2), text_y)
        .ok_or("prompt margin should still map to a caret")?;
    assert_eq!(prompt_margin_hit.source_bytes, None);
    assert_eq!(
        composer_cursor_at(&composer, rect, text_x.saturating_add(20), text_y),
        Some(padding_hit.cursor),
        "legacy caret mapping keeps its padding behavior"
    );
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
    let hit = composer_hit_at(&composer, area, 16, 1)
        .ok_or("mouse cell did not map to a chip boundary")?;
    assert!(hit.cursor <= chip.start || hit.cursor >= chip.end);
    let source = hit
        .source_bytes
        .ok_or("painted chip cell did not map to source")?;
    assert!(source.start >= chip.start && source.end <= chip.end);
    Ok(())
}

#[test]
fn long_composer_window_tracks_cursor_and_renders_recent_rows() -> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    let draft = format!("{}END", "x".repeat(480));
    // Growing editors can show the old 483-character fixture completely. Keep
    // the head/tail scrolling checks by typing enough text to exceed nine rows.
    for _ in 0..3 {
        composer.insert(&"x".repeat(480))?;
    }
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

#[test]
fn hitmap_uses_painted_regions_and_hover_is_visible() -> Result<(), Box<dyn Error>> {
    let (view, mut composer) = fixture()?;
    composer.insert("draft")?;
    let options = RenderOptions::default();
    let state = ScreenState::new(&view, &composer);
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let metadata = prepare_metadata(&state, &options, area);
    assert!(metadata.caret.is_some());
    assert!(
        metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::Transcript)
    );
    assert!(
        !metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::ToLatest)
    );
    assert_eq!(
        metadata.hit_at(metadata.composer_rect.x + 5, metadata.composer_rect.y + 1),
        Some(HitTarget::Composer)
    );
    let footer = metadata
        .hits
        .iter()
        .find(|hit| matches!(hit.target, HitTarget::Footer(_)))
        .ok_or("footer hit missing")?;
    let (x, y) = (footer.rect.x, footer.rect.y);
    assert!(matches!(metadata.hit_at(x, y), Some(HitTarget::Footer(_))));
    let mode = metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Footer(FooterAction::ToggleMode))
        .ok_or("mode control missing")?;
    let footer_cell = usize::from(mode.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(mode.rect.x));
    let hovered_footer = dump_cells(
        &ScreenState::new(&view, &composer)
            .with_hover(Some(HitTarget::Footer(FooterAction::ToggleMode))),
        &options,
        80,
        24,
    )?;
    let focused_footer = dump_cells(
        &ScreenState::new(&view, &composer).with_footer_focus(Some(FooterAction::ToggleMode)),
        &options,
        80,
        24,
    )?;
    assert_eq!(
        hovered_footer
            .cells
            .get(footer_cell)
            .and_then(|cell| cell.bg_rgb),
        Some((24, 24, 24))
    );
    assert_eq!(
        focused_footer
            .cells
            .get(footer_cell)
            .and_then(|cell| cell.bg_rgb),
        Some((25, 25, 25))
    );

    let plain = dump_cells(&state, &options, 80, 24)?;
    let explicit_no_hover = dump_cells(
        &ScreenState::new(&view, &composer).with_hover(None),
        &options,
        80,
        24,
    )?;
    assert_eq!(plain.cells, explicit_no_hover.cells);
    let unfocused = ScreenState::new(&view, &composer).with_editor_focus(false);
    assert!(prepare_metadata(&unfocused, &options, area).caret.is_none());
    assert!(
        dump_cells(&unfocused, &options, 80, 24)?
            .metadata
            .caret
            .is_none()
    );
    let hovered_transcript = dump_cells(
        &ScreenState::new(&view, &composer).with_hover(Some(HitTarget::Transcript)),
        &options,
        80,
        24,
    )?;
    assert_eq!(plain.cells, hovered_transcript.cells);

    let hovered_composer = dump_cells(
        &ScreenState::new(&view, &composer).with_hover(Some(HitTarget::Composer)),
        &options,
        80,
        24,
    )?;
    assert_eq!(plain.cells, hovered_composer.cells);

    let lines = vec!["sheet text".to_owned()];
    let sheet_state = ScreenState::new(&view, &composer).with_sheet(Some(InfoSheet {
        title: "Info",
        lines: &lines,
        footer: None,
        scroll: 0,
    }));
    let sheet_map = prepare_metadata(&sheet_state, &options, area);
    let sheet_rect = sheet_map
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Sheet)
        .ok_or("sheet hit missing")?
        .rect;
    assert_eq!(
        sheet_map.hit_at(sheet_rect.x, sheet_rect.y),
        Some(HitTarget::Sheet)
    );
    assert!(!sheet_map.hits.iter().any(|hit| {
        hit.target == HitTarget::Composer
            && hit
                .rect
                .contains(ratatui::layout::Position::new(sheet_rect.x, sheet_rect.y))
    }));

    let suggestions = vec![
        Suggestion {
            label: "first".to_owned(),
            insert: "first".to_owned(),
            description: "first option",
            kind: CompletionKind::Command,
            range: 0..0,
        },
        Suggestion {
            label: "second".to_owned(),
            insert: "second".to_owned(),
            description: "second option",
            kind: CompletionKind::Command,
            range: 0..0,
        },
    ];
    let completion_state =
        ScreenState::new(&view, &composer).with_completion(Some(CompletionOverlay {
            title: "commands",
            items: &suggestions,
            selected: Some(1),
        }));
    let completion_map = prepare_metadata(&completion_state, &options, area);
    let completion_hit = completion_map
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Completion(1))
        .ok_or("completion row missing")?;
    assert_eq!(
        completion_map.hit_at(completion_hit.rect.x, completion_hit.rect.y),
        Some(HitTarget::Completion(1))
    );
    assert_eq!(
        completion_map.hit_at(0, completion_hit.rect.y),
        Some(HitTarget::Completion(1))
    );
    assert!(
        !completion_map
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::Transcript
                && hit.rect.contains(ratatui::layout::Position::new(
                    completion_hit.rect.x,
                    completion_hit.rect.y
                )))
    );
    let plain_completion = dump_cells(&completion_state, &options, 80, 24)?;
    let hovered_completion = dump_cells(
        &completion_state.with_hover(Some(HitTarget::Completion(1))),
        &options,
        80,
        24,
    )?;
    let completion_cell = usize::from(completion_hit.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(completion_hit.rect.x));
    assert_ne!(
        plain_completion.cells.get(completion_cell),
        hovered_completion.cells.get(completion_cell)
    );

    let panel_state = ScreenState::new(&view, &composer).with_panel(PanelVisibility::Open);
    let panel_map = prepare_metadata(
        &panel_state,
        &options,
        ratatui::layout::Rect::new(0, 0, 120, 28),
    );
    let panel_hit = panel_map
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Panel)
        .ok_or("panel hit missing")?;
    assert_eq!(
        panel_map.hit_at(panel_hit.rect.x, panel_hit.rect.y),
        Some(HitTarget::Panel)
    );
    let plain_panel = dump_cells(&panel_state, &options, 120, 28)?;
    let hovered_panel = dump_cells(
        &panel_state.with_hover(Some(HitTarget::Panel)),
        &options,
        120,
        28,
    )?;
    assert_eq!(plain_panel.cells, hovered_panel.cells);

    let scrolled_state = ScreenState::new(&view, &composer).with_history_scroll(1);
    let scrolled_map = prepare_metadata(&scrolled_state, &options, area);
    let latest_region = scrolled_map
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::ToLatest)
        .ok_or("to-latest affordance hit missing")?;
    assert_eq!(
        scrolled_map.hit_at(latest_region.rect.x, latest_region.rect.y),
        Some(HitTarget::ToLatest)
    );
    assert!(
        scrolled_map
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::Transcript)
    );
    let latest_dump = dump_text(&scrolled_state, &options, 80, 24)?;
    assert!(latest_dump.contains("[to latest]"));

    let mut tool_view = view.clone();
    tool_view.tools.push(ToolActivity {
        frame: FrameId::from_u128(1),
        call: Cid::from_bytes([0; 32]),
        title: Some("shell".to_owned()),
        status: ToolStatus::Running,
        latency_us: None,
        expanded: None,
    });
    let tool_state = ScreenState::new(&tool_view, &composer).with_panel(PanelVisibility::Open);
    let tool_map = prepare_metadata(
        &tool_state,
        &options,
        ratatui::layout::Rect::new(0, 0, 120, 28),
    );
    let tool_hit = tool_map
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Tool(0))
        .ok_or("tool hit missing")?;
    assert_eq!(
        tool_map.hit_at(tool_hit.rect.x, tool_hit.rect.y),
        Some(HitTarget::Tool(0))
    );
    let plain_tool = dump_cells(&tool_state, &options, 120, 28)?;
    let hovered_tool = dump_cells(
        &tool_state.with_hover(Some(HitTarget::Tool(0))),
        &options,
        120,
        28,
    )?;
    let tool_cell = usize::from(tool_hit.rect.y)
        .saturating_mul(120)
        .saturating_add(usize::from(tool_hit.rect.x));
    let plain_title = plain_tool
        .cells
        .get(tool_cell)
        .ok_or("tool title cell missing")?;
    let hovered_title = hovered_tool
        .cells
        .get(tool_cell)
        .ok_or("hovered tool title cell missing")?;
    assert_eq!(plain_title.bg_rgb, hovered_title.bg_rgb);
    assert_eq!(hovered_title.fg_rgb, Some((255, 253, 230)));
    assert_ne!(plain_title.fg_rgb, hovered_title.fg_rgb);
    Ok(())
}

#[test]
fn compact_questionnaire_overlay_keeps_typed_targets_and_review_rows() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_assistant("transcript remains above the question overlay")?;
    let mut questions = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "req-1".into(),
        revision: 1,
        requester_label: "reviewer".into(),
        questions: vec![QuestionSpec {
            id: "q1".into(),
            prompt: "Choose a route".into(),
            required: true,
            kind: QuestionKind::SingleChoice {
                options: vec![
                    QuestionOption {
                        id: "safe".into(),
                        label: "Safe route".into(),
                    },
                    QuestionOption {
                        id: "fast".into(),
                        label: "Fast route".into(),
                    },
                ],
            },
        }],
    })?;
    questions.apply(QuestionnaireAction::ToggleChoice)?;
    questions.apply(QuestionnaireAction::FocusNotes)?;
    questions.apply(QuestionnaireAction::InsertText(
        "because\nand one more note row".into(),
    ))?;
    let state = ScreenState::new(&view, &composer).with_questionnaire(&questions, false);
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let rect = dump
        .metadata
        .questionnaire_rect
        .ok_or("question overlay geometry")?;
    assert_eq!(rect.bottom(), dump.metadata.composer_rect.bottom());
    assert!(rect.height <= 12);
    assert_eq!(dump.metadata.composer_count, 0);
    assert!(
        dump.metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::QuestionTab(0))
    );
    assert!(
        dump.metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::QuestionChoice(0))
    );
    assert!(
        dump.metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::QuestionNotes)
    );
    assert!(
        dump.metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::QuestionSubmit)
    );
    assert!(
        dump.metadata
            .source_cells
            .iter()
            .any(|cell| cell.rect.y < rect.y)
    );
    let editor = dump
        .metadata
        .questionnaire_editor
        .as_ref()
        .ok_or("notes editor geometry missing")?;
    assert_eq!(editor.geometry.rows().len(), 2);
    assert_eq!(editor.visible_rows, 0..2);
    let notes_hit = dump
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::QuestionNotes)
        .ok_or("notes target missing")?;
    assert_eq!(notes_hit.rect.height, 2);
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    assert!(text.contains("None of the above"));
    assert!(text.contains("Tab notes | Enter review/submit | arrows | Esc interrupt"));
    assert!(dump.metadata.caret.is_some());
    let choice_hit = dump
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::QuestionChoice(0))
        .ok_or("choice target missing")?;
    let choice_cell = usize::from(choice_hit.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(choice_hit.rect.x.saturating_add(2)));
    assert_eq!(
        dump.cells.get(choice_cell).and_then(|cell| cell.fg_rgb),
        Some((118, 185, 0))
    );
    let hovered = dump_cells(
        &state.with_hover(Some(HitTarget::QuestionChoice(0))),
        &RenderOptions::default(),
        80,
        24,
    )?;
    assert_eq!(
        hovered.cells.get(choice_cell).and_then(|cell| cell.bg_rgb),
        Some((35, 56, 0))
    );

    let review = ScreenState::new(&view, &composer).with_questionnaire(&questions, true);
    let review_text = dump_text(&review, &RenderOptions::default(), 80, 24)?;
    assert!(review_text.contains("Safe route"));
    assert!(review_text.contains("because"));
    assert!(review_text.contains("one more note row"));
    Ok(())
}

#[test]
fn compact_question_review_scrolls_through_every_answer() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let questions = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "review-all".into(),
        revision: 1,
        requester_label: "reviewer".into(),
        questions: (0..8)
            .map(|index| QuestionSpec {
                id: format!("q{index}"),
                prompt: format!("Question {index}"),
                required: false,
                kind: QuestionKind::FreeText,
            })
            .collect(),
    })?;
    let first = dump_text(
        &ScreenState::new(&view, &composer).with_questionnaire(&questions, true),
        &RenderOptions::default(),
        60,
        20,
    )?;
    let last = dump_text(
        &ScreenState::new(&view, &composer)
            .with_questionnaire(&questions, true)
            .with_questionnaire_scroll(2),
        &RenderOptions::default(),
        60,
        20,
    )?;
    assert!(first.contains("Question 0"));
    assert!(!first.contains("Question 7"));
    assert!(!last.contains("Question 0"));
    assert!(last.contains("Question 7"));
    assert!(last.contains("Unanswered"));
    Ok(())
}

#[test]
fn free_text_question_uses_wrapped_editor_geometry_and_visible_caret() -> Result<(), Box<dyn Error>>
{
    let (view, composer) = fixture()?;
    let mut questions = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "free-text-editor".into(),
        revision: 1,
        requester_label: "reviewer".into(),
        questions: vec![QuestionSpec {
            id: "q1".into(),
            prompt: "Describe the answer".into(),
            required: true,
            kind: QuestionKind::FreeText,
        }],
    })?;
    questions.apply(QuestionnaireAction::InsertText(
        "first answer row\nsecond answer row\nthird answer row\nfourth answer row\nfifth answer row".into(),
    ))?;
    questions.apply(QuestionnaireAction::SetCursor(80))?;
    questions.apply(QuestionnaireAction::SelectAll)?;

    let state = ScreenState::new(&view, &composer).with_questionnaire(&questions, false);
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let editor = dump
        .metadata
        .questionnaire_editor
        .as_ref()
        .ok_or("free-text answer editor geometry missing")?;
    assert_eq!(editor.geometry.rows().len(), 5);
    assert_eq!(editor.visible_rows, 1..5);
    assert_eq!(editor.rect.height, 4);
    assert!(dump.metadata.caret.is_some());
    assert!(
        dump.metadata
            .hits
            .iter()
            .any(|hit| { hit.target == HitTarget::QuestionAnswer && hit.rect == editor.rect })
    );
    let selected_cell = usize::from(editor.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(editor.rect.x));
    assert!(
        dump.cells
            .get(selected_cell)
            .and_then(|cell| cell.bg_rgb)
            .is_some_and(|background| background != (0, 0, 0))
    );
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    assert!(text.contains("second answer row"));
    assert!(text.contains("fifth answer row"));
    Ok(())
}

#[test]
fn free_text_answer_and_notes_keep_separate_editors_and_targets() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let mut questions = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "free-text-notes".into(),
        revision: 1,
        requester_label: "reviewer".into(),
        questions: vec![QuestionSpec {
            id: "q1".into(),
            prompt: "Answer with context".into(),
            required: true,
            kind: QuestionKind::FreeText,
        }],
    })?;
    questions.apply(QuestionnaireAction::InsertText("answer text".into()))?;
    questions.apply(QuestionnaireAction::FocusNotes)?;
    questions.apply(QuestionnaireAction::InsertText("separate note".into()))?;
    assert_eq!(questions.answer_text(), "answer text");
    assert_eq!(questions.notes(), "separate note");

    let note_state = ScreenState::new(&view, &composer).with_questionnaire(&questions, false);
    let note_dump = dump_cells(&note_state, &RenderOptions::default(), 80, 24)?;
    let note_editor = note_dump
        .metadata
        .questionnaire_editor
        .as_ref()
        .ok_or("notes editor geometry missing")?;
    assert!(note_dump.metadata.hits.iter().any(|hit| {
        hit.target == HitTarget::QuestionNotes
            && hit
                .rect
                .contains(Position::new(note_editor.rect.x, note_editor.rect.y))
    }));
    assert!(
        !note_dump
            .metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::QuestionAnswer)
    );

    questions.apply(QuestionnaireAction::FocusChoices)?;
    let answer_state = ScreenState::new(&view, &composer).with_questionnaire(&questions, false);
    let answer_dump = dump_cells(&answer_state, &RenderOptions::default(), 80, 24)?;
    let answer_editor = answer_dump
        .metadata
        .questionnaire_editor
        .as_ref()
        .ok_or("answer editor geometry missing after focus return")?;
    assert!(
        answer_dump.metadata.hits.iter().any(|hit| {
            hit.target == HitTarget::QuestionAnswer && hit.rect == answer_editor.rect
        })
    );
    assert!(
        answer_dump
            .metadata
            .hits
            .iter()
            .any(|hit| { hit.target == HitTarget::QuestionNotes })
    );
    Ok(())
}

#[test]
fn review_tabs_highlight_submit_and_leave_questions_inactive() -> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let questions = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "review-tabs".into(),
        revision: 1,
        requester_label: "reviewer".into(),
        questions: (0..2)
            .map(|index| QuestionSpec {
                id: format!("q{index}"),
                prompt: format!("Question {index}"),
                required: false,
                kind: QuestionKind::FreeText,
            })
            .collect(),
    })?;
    let state = ScreenState::new(&view, &composer).with_questionnaire(&questions, true);
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let question_tab = dump
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::QuestionTab(0))
        .ok_or("question tab hit missing")?;
    let submit = dump
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::QuestionSubmit)
        .ok_or("submit hit missing")?;
    let cell_index = |x: u16, y: u16| {
        usize::from(y)
            .saturating_mul(80)
            .saturating_add(usize::from(x))
    };
    let question_color = dump
        .cells
        .get(cell_index(question_tab.rect.x, question_tab.rect.y))
        .and_then(|cell| cell.fg_rgb);
    let submit_color = dump
        .cells
        .get(cell_index(submit.rect.x, submit.rect.y))
        .and_then(|cell| cell.fg_rgb);
    assert_ne!(question_color, Some((118, 185, 0)));
    assert_eq!(submit_color, Some((118, 185, 0)));
    Ok(())
}

#[test]
fn profile_footer_has_a_separate_target_without_removing_model_or_effort()
-> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let area = Rect::new(0, 0, 60, 20);
    let state = ScreenState::new(&view, &composer).with_profile_label(Some("offline"));
    let metadata = prepare_metadata(&state, &RenderOptions::default(), area);
    for action in [
        FooterAction::ToggleMode,
        FooterAction::Profile,
        FooterAction::Model,
        FooterAction::Effort,
    ] {
        assert!(
            metadata
                .hits
                .iter()
                .any(|hit| hit.target == HitTarget::Footer(action)),
            "missing {action:?}"
        );
    }
    let profile = metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Footer(FooterAction::Profile))
        .ok_or("profile hit missing")?;
    assert_eq!(
        footer_action_at_with_profile(&view, Some("offline"), area, profile.rect.x, profile.rect.y),
        Some(FooterAction::Profile)
    );
    let text = dump_text(&state, &RenderOptions::default(), 60, 20)?;
    assert!(text.contains("offline"));
    assert!(text.contains("[ local: ]"));
    Ok(())
}

#[test]
fn friendly_model_label_uses_the_same_footer_geometry_as_its_hit_target()
-> Result<(), Box<dyn Error>> {
    let (view, composer) = fixture()?;
    let area = Rect::new(0, 0, 60, 20);
    let state = ScreenState::new(&view, &composer)
        .with_profile_label(Some("demo"))
        .with_model_label(Some("Nemotron 3 Super"));
    let dump = dump_cells(&state, &RenderOptions::default(), area.width, area.height)?;
    let model = dump
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::Footer(FooterAction::Model))
        .ok_or("model hit missing")?;
    assert_eq!(
        footer_action_at_with_labels(
            &view,
            Some("demo"),
            Some("Nemotron 3 Super"),
            area,
            model.rect.x,
            model.rect.y,
        ),
        Some(FooterAction::Model)
    );
    let footer_start = usize::from(model.rect.y).saturating_mul(usize::from(dump.width));
    let footer = dump
        .cells
        .get(footer_start..footer_start.saturating_add(usize::from(dump.width)))
        .ok_or("footer row missing")?;
    let model_cells = footer
        .get(usize::from(model.rect.x)..usize::from(model.rect.right()))
        .ok_or("model label cells missing")?;
    let label = model_cells
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>();
    assert!(label.contains("Nemo"));
    assert!(!label.contains("local:test"));
    assert!(dump_text(&state, &RenderOptions::default(), 60, 20)?.contains("DEMO"));
    assert!(
        !dump_text(
            &ScreenState::new(&view, &composer),
            &RenderOptions::default(),
            60,
            20,
        )?
        .contains("DEMO")
    );
    Ok(())
}

#[test]
fn transcript_tokens_and_code_lines_keep_source_maps_and_semantic_styles()
-> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    let prose = "Visit https://example.com/path, run /plan and ping @workspace.";
    let code = "```rust\nfn main() { let value = 42; }\n```";
    view.push_assistant(format!("{prose}\n{code}"))?;
    let dump = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        100,
        24,
    )?;
    let source_at = |needle: &str| -> Result<usize, Box<dyn Error>> {
        prose
            .find(needle)
            .or_else(|| {
                code.find(needle)
                    .map(|offset| prose.len().saturating_add(1).saturating_add(offset))
            })
            .ok_or_else(|| format!("missing source token {needle}").into())
    };
    let cell_for = |offset: usize| {
        dump.metadata.source_cells.iter().find(|cell| {
            cell.entry_index == 0
                && cell.source_bytes.start <= offset
                && offset < cell.source_bytes.end
        })
    };
    let command = cell_for(source_at("/plan")?).ok_or("command source cell missing")?;
    let command_index = usize::from(command.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(command.rect.x));
    assert_eq!(
        dump.cells.get(command_index).and_then(|cell| cell.fg_rgb),
        Some((148, 205, 64))
    );
    let mention = cell_for(source_at("@workspace")?).ok_or("mention source cell missing")?;
    let mention_index = usize::from(mention.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(mention.rect.x));
    assert_eq!(
        dump.cells.get(mention_index).and_then(|cell| cell.fg_rgb),
        Some((249, 226, 175))
    );
    let link =
        cell_for(source_at("https://example.com/path")?).ok_or("link source cell missing")?;
    let link_index = usize::from(link.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(link.rect.x));
    assert!(
        dump.cells
            .get(link_index)
            .is_some_and(|cell| cell.modifiers.contains(Modifier::UNDERLINED))
    );

    let fn_offset = source_at("fn main")?;
    let fn_cell = cell_for(fn_offset).ok_or("code source cell missing")?;
    let fn_index = usize::from(fn_cell.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(fn_cell.rect.x));
    assert_eq!(
        dump.cells.get(fn_index).and_then(|cell| cell.fg_rgb),
        Some((203, 166, 247)),
        "code cell {:?} source {:?} dump text {:?}",
        dump.cells.get(fn_index).map(|cell| cell.symbol.as_str()),
        fn_cell.source_bytes,
        dump_text(
            &ScreenState::new(&view, &composer),
            &RenderOptions::default(),
            100,
            24,
        )?
    );
    assert!(
        fn_cell.rect.x >= 8,
        "code source should start after its gutter"
    );
    let gutter_x = fn_cell.rect.x.saturating_sub(7);
    let gutter_index = usize::from(fn_cell.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(gutter_x));
    assert_eq!(
        dump.cells
            .get(gutter_index)
            .map(|cell| cell.symbol.as_str()),
        Some(" ")
    );
    assert_eq!(
        dump.cells
            .get(gutter_index.saturating_add(5))
            .map(|cell| cell.symbol.as_str()),
        Some("1")
    );
    Ok(())
}

#[test]
fn transcript_presentation_tools_toggle_real_entry_body_and_map_source_offsets()
-> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("Read a config\nOpened C:/workspace/polaris.toml\nFound 3 settings")?;
    let collapsed_tool = [PresentationTool {
        entry_index: 0,
        expanded: false,
        status: PresentationToolStatus::Completed,
        duration_ms: Some(125),
    }];
    let expanded_tool = [PresentationTool {
        expanded: true,
        ..collapsed_tool[0]
    }];
    let collapsed_state =
        ScreenState::new(&view, &composer).with_presentation_tools(&collapsed_tool);
    let collapsed = dump_cells(&collapsed_state, &RenderOptions::default(), 100, 24)?;
    let collapsed_text = dump_text(&collapsed_state, &RenderOptions::default(), 100, 24)?;
    assert!(collapsed_text.contains("Read a config"));
    assert!(collapsed_text.contains("Ctrl+O expand"));
    assert!(!collapsed_text.contains("Opened C:/workspace/polaris.toml"));
    assert!(
        collapsed
            .metadata
            .hits
            .iter()
            .any(|hit| hit.target == HitTarget::PresentationTool(0))
    );
    assert!(
        collapsed
            .metadata
            .source_cells
            .iter()
            .any(|cell| cell.entry_index == 0 && cell.source_bytes.end <= "Read a config".len())
    );

    let expanded_state = ScreenState::new(&view, &composer).with_presentation_tools(&expanded_tool);
    let expanded = dump_cells(&expanded_state, &RenderOptions::default(), 100, 24)?;
    let expanded_text = dump_text(&expanded_state, &RenderOptions::default(), 100, 24)?;
    assert!(expanded_text.contains("Opened C:/workspace/polaris.toml"));
    assert!(expanded.metadata.source_cells.iter().any(|cell| cell.entry_index == 0 && cell.source_bytes.start >= "Read a config\n".len()));
    let hit = expanded
        .metadata
        .hits
        .iter()
        .find(|hit| hit.target == HitTarget::PresentationTool(0))
        .ok_or("presentation title hit missing")?;
    let hovered = dump_cells(
        &expanded_state.with_hover(Some(HitTarget::PresentationTool(0))),
        &RenderOptions::default(),
        100,
        24,
    )?;
    let cell_index = usize::from(hit.rect.y)
        .saturating_mul(100)
        .saturating_add(usize::from(hit.rect.x));
    assert_eq!(
        expanded.cells.get(cell_index).map(|cell| cell.bg_rgb),
        hovered.cells.get(cell_index).map(|cell| cell.bg_rgb)
    );
    assert_eq!(
        hovered.cells.get(cell_index).and_then(|cell| cell.fg_rgb),
        Some((255, 253, 230))
    );
    Ok(())
}

#[test]
fn warning_entries_use_the_observed_warning_role_without_bold_weight() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_warning("Approval is required")?;
    let dump = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let span = dump
        .metadata
        .source_cells
        .iter()
        .find(|cell| cell.entry_index == 0)
        .ok_or("warning source missing")?;
    let index = usize::from(span.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(span.rect.x));
    let cell = dump.cells.get(index).ok_or("warning cell missing")?;
    assert_eq!(cell.fg_rgb, Some((229, 180, 84)));
    assert!(!cell.modifiers.contains(Modifier::BOLD));
    Ok(())
}

#[test]
fn transcript_grapheme_cells_map_to_source_and_selection_styles_them() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_assistant("one界two")?;
    let selection = TranscriptSelection {
        entry_index: 0,
        source_bytes: 3..6,
    };
    let state = ScreenState::new(&view, &composer).with_transcript_selection(Some(selection));
    let options = RenderOptions::default();
    let prepared = prepare_metadata(&state, &options, ratatui::layout::Rect::new(0, 0, 80, 24));
    let dump = dump_cells(&state, &options, 80, 24)?;
    assert_eq!(prepared.source_cells, dump.metadata.source_cells);
    let mapped = dump
        .metadata
        .source_cells
        .iter()
        .find(|span| span.entry_index == 0 && span.source_bytes == (3..6))
        .ok_or("wide transcript grapheme map missing")?;
    assert_eq!(mapped.rect.width, 2);
    assert_eq!(
        dump.metadata.source_at(mapped.rect.x, mapped.rect.y),
        Some(mapped)
    );
    let cell_index = usize::from(mapped.rect.y)
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(mapped.rect.x));
    assert_eq!(
        dump.cells.get(cell_index).map(|cell| cell.bg_rgb),
        Some(Some((70, 70, 70)))
    );
    assert_eq!(
        dump.cells.get(cell_index).map(|cell| cell.fg_rgb),
        Some(Some((232, 232, 232)))
    );
    Ok(())
}

#[test]
fn transcript_span_selection_crosses_entry_boundaries_and_preserves_source_foreground()
-> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("first")?;
    view.push_assistant("middle")?;
    view.push_assistant("last")?;
    let selection = TranscriptSpanSelection {
        anchor: TranscriptPoint {
            entry_index: 0,
            source_byte: 2,
        },
        head: TranscriptPoint {
            entry_index: 2,
            source_byte: 2,
        },
    };
    let dump = dump_cells(
        &ScreenState::new(&view, &composer).with_transcript_span_selection(Some(selection)),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let bg_for = |entry_index: usize, byte: usize| {
        let span =
            dump.metadata.source_cells.iter().find(|span| {
                span.entry_index == entry_index && span.source_bytes.contains(&byte)
            })?;
        let index = usize::from(span.rect.y)
            .saturating_mul(usize::from(dump.width))
            .saturating_add(usize::from(span.rect.x));
        dump.cells.get(index).map(|cell| (cell.bg_rgb, cell.fg_rgb))
    };
    assert_eq!(bg_for(0, 0), Some((Some((0, 0, 0)), Some((232, 232, 232)))));
    assert_eq!(
        bg_for(0, 2),
        Some((Some((70, 70, 70)), Some((232, 232, 232))))
    );
    assert_eq!(
        bg_for(1, 0),
        Some((Some((70, 70, 70)), Some((232, 232, 232))))
    );
    assert_eq!(
        bg_for(2, 1),
        Some((Some((70, 70, 70)), Some((232, 232, 232))))
    );
    assert_eq!(bg_for(2, 2), Some((Some((0, 0, 0)), Some((232, 232, 232)))));
    Ok(())
}

#[test]
fn transcript_drag_boundaries_use_visible_graphemes_and_stop_at_text_end()
-> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    let message = "A界e\u{301}\tz";
    view.push_assistant(message)?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 80, 24);
    let rendered = dump_cells(&state, &options, area.width, area.height)?;
    let prepared = prepare_metadata(&state, &options, area);
    assert_eq!(prepared.source_spans, rendered.metadata.source_spans);

    let row = rendered
        .metadata
        .source_spans
        .first()
        .ok_or("transcript text row missing")?;
    let wide = rendered
        .metadata
        .source_cells
        .iter()
        .find(|cell| cell.entry_index == row.entry_index && cell.source_bytes == (1..4))
        .ok_or("wide grapheme map missing")?;
    assert_eq!(wide.rect.width, 2);
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(wide.rect.x, wide.rect.y),
        Some(TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: 1,
        })
    );

    let tab = rendered
        .metadata
        .source_cells
        .iter()
        .find(|cell| cell.entry_index == row.entry_index && cell.source_bytes == (7..8))
        .ok_or("tab source map missing")?;
    assert!(tab.rect.width > 0);
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(tab.rect.x.saturating_add(tab.rect.width / 2), tab.rect.y),
        Some(TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: 8,
        }),
        "an expanded tab remains a real selectable source grapheme"
    );
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(wide.rect.right().saturating_sub(1), wide.rect.y),
        Some(TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: 4,
        })
    );

    let text_end = rendered
        .metadata
        .source_cells
        .iter()
        .filter(|cell| cell.entry_index == row.entry_index && cell.rect.y == row.rect.y)
        .map(|cell| cell.rect.right())
        .max()
        .ok_or("transcript cells missing")?;
    assert_eq!(
        rendered.metadata.source_at(text_end, row.rect.y),
        None,
        "blank filler must not count as a source hit"
    );
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(text_end, row.rect.y),
        Some(TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: message.len(),
        }),
        "dragging through row padding should clamp at the actual text end"
    );
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(row.rect.x, rendered.metadata.body.bottom()),
        None,
        "coordinates outside the transcript body must not resolve to text"
    );
    Ok(())
}

#[test]
fn transcript_drag_boundary_stays_on_the_requested_soft_wrapped_row() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    let message = "界".repeat(80);
    view.push_assistant(&message)?;
    let rendered = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let rows: Vec<_> = rendered
        .metadata
        .source_spans
        .iter()
        .filter(|span| span.entry_index == 0)
        .collect();
    let first = rows.first().ok_or("first wrapped source row missing")?;
    let second = rows.get(1).ok_or("second wrapped source row missing")?;
    assert_eq!(first.source_bytes.end, second.source_bytes.start);
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(first.rect.x, first.rect.y),
        Some(TranscriptPoint {
            entry_index: 0,
            source_byte: first.source_bytes.start,
        })
    );
    assert_eq!(
        rendered
            .metadata
            .transcript_boundary_at(second.rect.right().saturating_sub(1), second.rect.y),
        Some(TranscriptPoint {
            entry_index: 0,
            source_byte: second.source_bytes.end,
        })
    );
    Ok(())
}

#[test]
fn prepared_and_cached_metadata_keep_the_same_visible_transcript_rows() -> Result<(), Box<dyn Error>>
{
    let (mut view, composer) = fixture()?;
    view.push_assistant("first wrapped row\nsecond row")?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 80, 24);
    let rendered = dump_cells(&state, &options, area.width, area.height)?;
    let prepared = prepare_metadata(&state, &options, area);
    let mut cache = RenderCache::new();
    let cached = prepare_metadata_with_cache(&state, &options, area, &mut cache);
    assert_eq!(prepared.source_spans, rendered.metadata.source_spans);
    assert_eq!(cached.source_spans, rendered.metadata.source_spans);
    assert_eq!(prepared.source_cells, rendered.metadata.source_cells);
    assert_eq!(cached.source_cells, rendered.metadata.source_cells);
    Ok(())
}

#[test]
fn transcript_selection_does_not_tint_blank_row_padding() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("selected text")?;
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 80, 24);
    let baseline = dump_cells(
        &ScreenState::new(&view, &composer),
        &options,
        area.width,
        area.height,
    )?;
    let row = baseline
        .metadata
        .source_spans
        .first()
        .ok_or("transcript text row missing")?;
    let last_source_cell = baseline
        .metadata
        .source_cells
        .iter()
        .filter(|cell| cell.entry_index == row.entry_index && cell.rect.y == row.rect.y)
        .map(|cell| cell.rect.right())
        .max()
        .ok_or("transcript cells missing")?;
    let padding_index = usize::from(row.rect.y)
        .saturating_mul(usize::from(baseline.width))
        .saturating_add(usize::from(last_source_cell));
    let selected = dump_cells(
        &ScreenState::new(&view, &composer).with_transcript_span_selection(Some(
            TranscriptSpanSelection {
                anchor: TranscriptPoint {
                    entry_index: row.entry_index,
                    source_byte: 0,
                },
                head: TranscriptPoint {
                    entry_index: row.entry_index,
                    source_byte: 1,
                },
            },
        )),
        &options,
        area.width,
        area.height,
    )?;
    assert_eq!(
        selected.cells.get(padding_index).map(|cell| cell.bg_rgb),
        baseline.cells.get(padding_index).map(|cell| cell.bg_rgb),
        "blank filler has no source span and must keep its row background"
    );
    Ok(())
}

#[test]
fn selected_whitespace_uses_neutral_text_tint() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("a b")?;
    let dump = dump_cells(
        &ScreenState::new(&view, &composer).with_transcript_selection(Some(TranscriptSelection {
            entry_index: 0,
            source_bytes: 0..3,
        })),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let whitespace = dump
        .metadata
        .source_cells
        .iter()
        .find(|cell| cell.source_bytes == (1..2))
        .ok_or("selected whitespace map missing")?;
    let index = usize::from(whitespace.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(whitespace.rect.x));
    let cell = dump
        .cells
        .get(index)
        .ok_or("selected whitespace cell missing")?;
    assert_eq!(cell.symbol, " ");
    assert_eq!(cell.bg_rgb, Some((70, 70, 70)));
    assert_eq!(cell.fg_rgb, Some((232, 232, 232)));
    Ok(())
}

#[test]
fn selection_fallbacks_use_ansi_237_and_bold_without_reverse_video() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("selected")?;
    let state =
        ScreenState::new(&view, &composer).with_transcript_selection(Some(TranscriptSelection {
            entry_index: 0,
            source_bytes: 0..8,
        }));
    let ansi = dump_cells(
        &state,
        &RenderOptions {
            colors: ColorMode::Ansi,
            ascii: true,
        },
        80,
        24,
    )?;
    let none = dump_cells(
        &state,
        &RenderOptions {
            colors: ColorMode::NoColor,
            ascii: true,
        },
        80,
        24,
    )?;
    let span = ansi
        .metadata
        .source_cells
        .iter()
        .find(|span| span.source_bytes == (0..1))
        .ok_or("selected source cell missing")?;
    let index = usize::from(span.rect.y)
        .saturating_mul(80)
        .saturating_add(usize::from(span.rect.x));
    assert_eq!(
        ansi.cells.get(index).map(|cell| cell.bg),
        Some(Color::Indexed(237))
    );
    let plain = none
        .cells
        .get(index)
        .ok_or("no-color selected cell missing")?;
    assert!(plain.modifiers.contains(Modifier::BOLD));
    assert!(!plain.modifiers.contains(Modifier::REVERSED));
    Ok(())
}
