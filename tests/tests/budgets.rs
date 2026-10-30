//! PLAN §11.4 performance budgets (P10-3), measured through the real crate APIs. Run optimized and
//! one at a time so the numbers are not skewed by the debug build or by other tests:
//!
//! `cargo test --profile budget -p polaris-tests --test budgets -- --nocapture --test-threads 1`
//!
//! Each test prints `budget <area>: <measurement> (target <target>)`; the budgets are targets, so
//! the tests only check that the measurement ran. Bus publish→subscribe and S1 decisions are
//! measured by their spikes (`polaris-spike-bus`, `polaris-spike-s1`); allocation budgets live in
//! `budget_alloc.rs`, which needs its own allocator.

use std::error::Error;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use polaris_nebius_cloud_token_factory::{SseEventRef, SseParser};
use polaris_ratelimit::{DualLimiter, Estimate, MonotonicClock, Priority, Quota, ShedPolicy};
use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{ModelId, TokenCount};
use polaris_security_automata::{DfaScanner, ScanBuf};
use polaris_storage_sync::{HashMode, IgnoreRules, plan, scan};
use polaris_ui_cli::screen::{RenderOptions, ScreenState, render};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::view::ViewState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

type TestResult = Result<(), Box<dyn Error>>;

fn quantile(samples: &mut [Duration], q: f64) -> Duration {
    samples.sort_unstable();
    let index = ((samples.len() as f64 - 1.0) * q).round() as usize;
    samples.get(index).copied().unwrap_or_default()
}

fn report(area: &str, measurement: &str, target: &str) {
    eprintln!("budget {area}: {measurement} (target {target})");
}

/// Source-like text without secrets or destructive commands, so the scan does no match work.
fn corpus(bytes: usize) -> Vec<u8> {
    let line = b"    let total = values.iter().map(|value| value.wrapping_mul(31)).sum::<u64>(); // accumulate\n";
    line.iter().copied().cycle().take(bytes).collect()
}

#[test]
fn dfa_scan_throughput() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let data = corpus(64 * 1024 * 1024);
    let mut matches = ScanBuf::with_capacity(4_096);
    // Warm up on a slice, then time the whole buffer in 1 MiB chunks.
    scanner.scan(data.get(..1024 * 1024).unwrap_or_default(), &mut matches)?;
    let started = Instant::now();
    for chunk in data.chunks(1024 * 1024) {
        scanner.scan(chunk, &mut matches)?;
    }
    let elapsed = started.elapsed();
    let mb_per_s = data.len() as f64 / 1e6 / elapsed.as_secs_f64();
    report(
        "DFA scan",
        &format!("{mb_per_s:.0} MB/s single core over 64 MiB, default pack (secrets, PII, shell)"),
        "≥ 500 MB/s",
    );
    assert!(mb_per_s > 0.0);
    Ok(())
}

#[test]
fn gcra_check_latency() -> TestResult {
    let large = NonZeroU32::new(u32::MAX / 2).ok_or("quota")?;
    let limiter = DualLimiter::new(
        Quota {
            rpm: large,
            tpm: large,
            burst_requests: u32::MAX / 4,
            burst_tokens: u32::MAX / 4,
        },
        MonotonicClock::default(),
        ShedPolicy::default(),
    )?;
    let estimate = Estimate {
        prompt: TokenCount::new(64),
        max_output: TokenCount::new(64),
    };
    let mut samples = Vec::with_capacity(100_000);
    for _ in 0..1_000 {
        let _warm = limiter.try_acquire(estimate, Priority::InteractiveUser);
    }
    for _ in 0..100_000 {
        let started = Instant::now();
        let permit = limiter.try_acquire(estimate, Priority::InteractiveUser);
        samples.push(started.elapsed());
        drop(permit);
    }
    let p50 = quantile(&mut samples, 0.50);
    let p99 = quantile(&mut samples, 0.99);
    report(
        "GCRA check",
        &format!(
            "try_acquire p50 {} ns, p99 {} ns over 100k admitted calls (RPM + TPM cells)",
            p50.as_nanos(),
            p99.as_nanos()
        ),
        "< 1 µs, zero alloc",
    );
    Ok(())
}

#[test]
fn sse_frame_parse_rate() -> TestResult {
    let frame = b"data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello world\"},\"finish_reason\":null}]}\n\n";
    let stream: Vec<u8> = frame
        .iter()
        .copied()
        .cycle()
        .take(frame.len() * 100_000)
        .collect();
    let mut parser = SseParser::new(64 * 1024, 1_000_000)?;
    let mut events = 0_u64;
    let started = Instant::now();
    for chunk in stream.chunks(4_096) {
        parser.feed_with(chunk, |event| {
            if let SseEventRef::Data(_) = event {
                events += 1;
            }
            Ok(())
        })?;
    }
    let elapsed = started.elapsed();
    report(
        "SSE frame parse",
        &format!(
            "{} frames in {:.1} ms ({:.0} ns/frame, {:.0} MB/s); allocations: see budget_alloc",
            events,
            elapsed.as_secs_f64() * 1e3,
            elapsed.as_nanos() as f64 / events.max(1) as f64,
            stream.len() as f64 / 1e6 / elapsed.as_secs_f64()
        ),
        "zero alloc per frame",
    );
    assert_eq!(events, 100_000);
    Ok(())
}

#[test]
fn sync_no_change_manifest_diff_10k_files() -> TestResult {
    let root = tempfile::tempdir()?;
    for index in 0..10_000 {
        let directory = root.path().join(format!("src/m{:03}", index / 100));
        if index % 100 == 0 {
            std::fs::create_dir_all(&directory)?;
        }
        std::fs::write(
            directory.join(format!("f{index:05}.rs")),
            format!("pub fn f{index}() -> u64 {{ {index} }}\n"),
        )?;
    }
    let rules = IgnoreRules::new(root.path(), None)?;
    let first = scan(root.path(), &rules, None, HashMode::Fast)?;
    let started = Instant::now();
    let second = scan(root.path(), &rules, Some(&first.cache), HashMode::Fast)?;
    let diff = plan(&second.manifest, &first.manifest);
    let elapsed = started.elapsed();
    report(
        "Sync 10k files, no changes",
        &format!(
            "rescan with cache + manifest diff in {:.0} ms ({} files, {} actions)",
            elapsed.as_secs_f64() * 1e3,
            second.manifest.entries.len(),
            diff.upload.len() + diff.delete.len()
        ),
        "< 2 s",
    );
    assert_eq!(second.manifest.entries.len(), 10_000);
    assert!(diff.is_empty(), "nothing changed, nothing to sync");
    Ok(())
}

#[test]
fn tui_frame_render_200x60() -> TestResult {
    let mut view = ViewState::new(ModelId::new("local:budget")?, Effort::Medium);
    for turn in 0..40 {
        view.push_user(format!(
            "question {turn}: why does the parser reject this input?"
        ))?;
        view.push_assistant(format!(
            "Answer {turn}: the grammar requires a closing parenthesis.\n\n```rust\nfn parse(input: &str) -> Result<Ast, Error> {{\n    let tokens = lex(input)?;\n    Parser::new(&tokens).expression()\n}}\n```\n\nSee `src/parser.rs:{}`.",
            40 + turn
        ))?;
    }
    let composer = Composer::new();
    let state = ScreenState::new(&view, &composer);
    let options = RenderOptions::default();
    let mut terminal = Terminal::new(TestBackend::new(200, 60))?;
    for _ in 0..20 {
        terminal.draw(|frame| {
            let _metadata = render(frame, &state, &options);
        })?;
    }
    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = Instant::now();
        terminal.draw(|frame| {
            let _metadata = render(frame, &state, &options);
        })?;
        samples.push(started.elapsed());
    }
    let p50 = quantile(&mut samples, 0.50);
    let p99 = quantile(&mut samples, 0.99);
    report(
        "TUI frame render",
        &format!(
            "200×60 with 80 transcript entries, uncached: p50 {:.2} ms, p99 {:.2} ms (TestBackend, includes buffer diff)",
            p50.as_secs_f64() * 1e3,
            p99.as_secs_f64() * 1e3
        ),
        "< 8 ms",
    );
    Ok(())
}

#[test]
fn local_search_cold_and_warm_on_this_repository() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("repository root")?
        .to_path_buf();
    let started = Instant::now();
    let cold =
        polaris_ui_session::local_search::search(&root, "verify human approval mutation", 10)?;
    let cold_time = started.elapsed();
    let started = Instant::now();
    let warm = polaris_ui_session::local_search::search(&root, "named pipe client token", 10)?;
    let warm_time = started.elapsed();
    report(
        "Local search (/search-local, $rag)",
        &format!(
            "this repository: first query {:.0} ms (indexing), next query {:.0} ms (cached index); {} and {} hits",
            cold_time.as_secs_f64() * 1e3,
            warm_time.as_secs_f64() * 1e3,
            cold.len(),
            warm.len()
        ),
        "no PLAN budget; interactive",
    );
    assert!(!cold.is_empty() && !warm.is_empty());
    Ok(())
}
