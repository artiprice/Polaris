use std::error::Error;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use polaris_ui_cli::screen::{
    ColorMode, MAX_RENDER_CACHE_SOURCE_BYTES, PresentationTool, PresentationToolStatus,
    RenderCache, RenderOptions, ScreenState, dump_cells, prepare_metadata_with_cache, render,
    render_with_cache,
};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::view::ViewState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn fixture() -> Result<(ViewState, Composer), Box<dyn Error>> {
    let view = ViewState::new(ModelId::new("local:test")?, Effort::Low);
    Ok((view, Composer::new()))
}

#[test]
fn cache_hits_for_identical_bounded_transcript_entries() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_user("first prompt")?;
    view.push_assistant("first answer with enough content to wrap")?;
    view.push_user("second prompt")?;
    view.push_assistant("second answer with enough content to wrap")?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 200, 60);
    let mut cache = RenderCache::new();

    let _first = prepare_metadata_with_cache(&state, &options, area, &mut cache);
    let first = cache.stats();
    let _second = prepare_metadata_with_cache(&state, &options, area, &mut cache);
    let second = cache.stats();

    assert!(first.misses > 0, "first pass must populate bounded entries");
    assert!(second.hits > first.hits, "second pass should reuse entries");
    Ok(())
}

#[test]
fn cache_invalidates_on_content_width_and_tool_expansion_changes() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_assistant("tool title\nbody line that is shown only when expanded")?;
    let tools = [PresentationTool {
        entry_index: 0,
        expanded: false,
        status: PresentationToolStatus::Completed,
        duration_ms: Some(12),
    }];
    let options = RenderOptions::default();
    let mut cache = RenderCache::new();

    let collapsed = ScreenState::new(&view, &composer).with_presentation_tools(&tools);
    let _collapsed =
        prepare_metadata_with_cache(&collapsed, &options, Rect::new(0, 0, 100, 30), &mut cache);
    let first = cache.stats();

    let _other_width =
        prepare_metadata_with_cache(&collapsed, &options, Rect::new(0, 0, 101, 30), &mut cache);
    let after_width = cache.stats();
    assert!(after_width.misses > first.misses);

    let expanded_tools = [PresentationTool {
        entry_index: 0,
        expanded: true,
        status: PresentationToolStatus::Completed,
        duration_ms: Some(12),
    }];
    let expanded = ScreenState::new(&view, &composer).with_presentation_tools(&expanded_tools);
    let _expanded =
        prepare_metadata_with_cache(&expanded, &options, Rect::new(0, 0, 100, 30), &mut cache);
    let after_expand = cache.stats();
    assert!(after_expand.misses > after_width.misses);

    if let Some(entry) = view.conversation.get_mut(0) {
        entry.text = entry.text.replacen("tool", "cool", 1);
    }
    let changed = ScreenState::new(&view, &composer).with_presentation_tools(&tools);
    let _changed =
        prepare_metadata_with_cache(&changed, &options, Rect::new(0, 0, 100, 30), &mut cache);
    assert!(cache.stats().misses > after_expand.misses);
    Ok(())
}

#[test]
fn cache_evicts_within_configured_byte_budget() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    for index in 0..8 {
        view.push_assistant(format!("entry {index}: {}", "x".repeat(96)))?;
    }
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let area = Rect::new(0, 0, 200, 60);
    let mut cache = RenderCache::with_byte_budget(8_192);

    let _metadata = prepare_metadata_with_cache(&state, &options, area, &mut cache);
    let stats = cache.stats();

    assert!(
        stats.evictions > 0,
        "the small cache budget must force eviction"
    );
    assert!(stats.resident_bytes <= 8_192);
    assert!(stats.resident_bytes <= RenderCache::MAX_BYTE_BUDGET);
    assert!(stats.source_snapshot_bytes <= MAX_RENDER_CACHE_SOURCE_BYTES);
    let capped = RenderCache::with_byte_budget(usize::MAX);
    assert_eq!(capped.stats().byte_budget, RenderCache::MAX_BYTE_BUDGET);
    Ok(())
}

#[test]
fn source_snapshot_validation_stays_below_its_separate_limit() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    for _ in 0..5 {
        view.push_assistant("x".repeat(55_000))?;
    }
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let mut cache = RenderCache::new();

    for width in 100_u16..105_u16 {
        let area = Rect::new(0, 0, width, 30);
        let _metadata = prepare_metadata_with_cache(&state, &options, area, &mut cache);
    }

    let stats = cache.stats();
    assert!(stats.evictions > 0);
    assert!(stats.source_snapshot_bytes <= MAX_RENDER_CACHE_SOURCE_BYTES);
    assert!(stats.resident_bytes <= RenderCache::MAX_BYTE_BUDGET);
    Ok(())
}

#[test]
fn cached_renderer_matches_uncached_render_output() -> Result<(), Box<dyn Error>> {
    let (mut view, composer) = fixture()?;
    view.push_user("prompt with a link https://example.test/path")?;
    view.push_assistant("```rust\nfn main() { let answer = 42; }\n```")?;
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions {
        colors: ColorMode::TrueColor,
        ascii: false,
    };
    let area = Rect::new(0, 0, 200, 60);
    let mut uncached = Terminal::new(TestBackend::new(area.width, area.height))?;
    let mut cached = Terminal::new(TestBackend::new(area.width, area.height))?;
    let mut cache = RenderCache::new();
    let mut uncached_metadata = None;
    let mut cached_metadata = None;

    uncached.draw(|frame| uncached_metadata = Some(render(frame, &state, &options)))?;
    cached.draw(|frame| {
        cached_metadata = Some(render_with_cache(frame, &state, &options, &mut cache));
    })?;

    let uncached_metadata = uncached_metadata.ok_or("uncached render returned no metadata")?;
    let cached_metadata = cached_metadata.ok_or("cached render returned no metadata")?;
    let uncached_cells = dump_cells(&state, &options, area.width, area.height)?;
    let rendered_cached_cells = cached
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| (cell.symbol().to_owned(), cell.fg, cell.bg, cell.modifier))
        .collect::<Vec<_>>();
    let dumped_uncached_cells = uncached_cells
        .cells
        .into_iter()
        .map(|cell| (cell.symbol, cell.fg, cell.bg, cell.modifiers))
        .collect::<Vec<_>>();

    assert_eq!(rendered_cached_cells, dumped_uncached_cells);
    assert_eq!(cached_metadata, uncached_metadata);
    assert_eq!(cached_metadata.source_cells, uncached_metadata.source_cells);
    assert_eq!(cached_metadata.hits, uncached_metadata.hits);
    Ok(())
}
