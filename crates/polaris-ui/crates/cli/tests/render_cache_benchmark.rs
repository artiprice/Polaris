use std::error::Error;
use std::time::Instant;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use polaris_ui_cli::screen::{RenderCache, RenderOptions, ScreenState, render, render_with_cache};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::view::ViewState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
#[ignore = "manual CPU-only cache benchmark; run with --ignored --nocapture"]
fn reports_render_cache_latency_percentiles_at_200_by_60() -> Result<(), Box<dyn Error>> {
    let mut view = ViewState::new(ModelId::new("local:benchmark")?, Effort::Low);
    for entry in 0..56 {
        let content = format!("entry {entry}\n{}", "word ".repeat(512));
        view.push_assistant(content)?;
    }
    let composer = Composer::new();
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let mut terminal = Terminal::new(TestBackend::new(200, 60))?;
    let mut cache = RenderCache::new();
    let mut uncached_samples = Vec::with_capacity(64);
    let mut cold_samples = Vec::with_capacity(64);
    let mut warm_samples = Vec::with_capacity(64);

    for _ in 0..64 {
        let start = Instant::now();
        terminal.draw(|frame| {
            let _metadata = render(frame, &state, &options);
        })?;
        uncached_samples.push(start.elapsed().as_nanos());
    }
    for _ in 0..64 {
        cache.clear();
        let start = Instant::now();
        terminal.draw(|frame| {
            let _metadata = render_with_cache(frame, &state, &options, &mut cache);
        })?;
        cold_samples.push(start.elapsed().as_nanos());
    }
    for _ in 0..8 {
        terminal.draw(|frame| {
            let _metadata = render_with_cache(frame, &state, &options, &mut cache);
        })?;
    }
    for _ in 0..64 {
        let start = Instant::now();
        terminal.draw(|frame| {
            let _metadata = render_with_cache(frame, &state, &options, &mut cache);
        })?;
        warm_samples.push(start.elapsed().as_nanos());
    }

    let uncached = latency_summary(&mut uncached_samples)?;
    let cold = latency_summary(&mut cold_samples)?;
    let warm = latency_summary(&mut warm_samples)?;
    let stats = cache.stats();
    eprintln!(
        "render-cache benchmark viewport=200x60 entries=56 iterations=64\n\
         uncached_ns p50={} p95={} p99={} max={}\n\
         cold_ns p50={} p95={} p99={} max={}\n\
         warm_ns p50={} p95={} p99={} max={}\n\
         cache entries={} hits={} misses={} evictions={} resident_bytes={} source_snapshot_bytes={}",
        uncached.0,
        uncached.1,
        uncached.2,
        uncached.3,
        cold.0,
        cold.1,
        cold.2,
        cold.3,
        warm.0,
        warm.1,
        warm.2,
        warm.3,
        stats.entries,
        stats.hits,
        stats.misses,
        stats.evictions,
        stats.resident_bytes,
        stats.source_snapshot_bytes,
    );
    Ok(())
}

fn latency_summary(samples: &mut [u128]) -> Result<(u128, u128, u128, u128), Box<dyn Error>> {
    samples.sort_unstable();
    let maximum = samples.last().copied().ok_or("benchmark had no samples")?;
    let p50 = percentile(samples, 50).ok_or("p50 sample missing")?;
    let p95 = percentile(samples, 95).ok_or("p95 sample missing")?;
    let p99 = percentile(samples, 99).ok_or("p99 sample missing")?;
    Ok((p50, p95, p99, maximum))
}

fn percentile(samples: &[u128], percentile: usize) -> Option<u128> {
    let last_index = samples.len().checked_sub(1)?;
    let index = last_index.saturating_mul(percentile) / 100;
    samples.get(index).copied()
}
