//! Log Enqueue Zero Alloc (descriptor: specs/descriptors/log_log_enqueue_zero_alloc.json; PLAN ADR-003,
//! hot-path member "log enqueue"). One integration-test binary per zero-alloc check: dhat allows
//! one profiler per process.
//!
//! Measured path: `tracing::info!` -> subscriber dispatch -> `RedactionLayer::on_event` (format into
//! the reused thread-local buffers, `Redactor::redact_into`, sanitise) -> TUI ring, telemetry ring
//! and file `DrainQueue` (no consumer attached, so the queue overflows and the drop path is
//! measured too).
//!
//! The drain thread and spdlog-rs run elsewhere by design and are not part of this check.

use std::sync::Arc;

use polaris_log::{DrainQueue, LogCounters, RedactionLayer, RingSink, Sinks};
use polaris_sdk::registry::{RedactStats, Redactor};
use tracing::Dispatch;
use tracing_subscriber::Registry;
use tracing_subscriber::layer::SubscriberExt;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const NEEDLE: &[u8] = b"sk-fake-secret";

/// Allocation-free masking redactor (the real one is DFA based and also allocation-free).
struct Mask;

impl Redactor for Mask {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let mut rest = input;
        let mut replacements = 0u32;
        while let Some(pos) = rest.windows(NEEDLE.len()).position(|w| w == NEEDLE) {
            out.extend_from_slice(rest.get(..pos).unwrap_or_default());
            out.extend_from_slice(b"[REDACTED]");
            rest = rest.get(pos + NEEDLE.len()..).unwrap_or_default();
            replacements += 1;
        }
        out.extend_from_slice(rest);
        RedactStats {
            bytes_in: input.len(),
            replacements,
        }
    }
}

#[derive(Debug)]
struct Point {
    x: i32,
    y: i32,
}

fn emit(i: u64, point: &Point) {
    tracing::info!(
        round = i,
        flag = true,
        name = "worker",
        point = ?point,
        ratio = 0.5,
        "request done with sk-fake-secret in the text"
    );
    tracing::debug!("plain message without fields");
}

#[test]
fn layer_reuses_buffer() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let counters = LogCounters::new();
    let tui = Arc::new(RingSink::with_line_limit(64, 512));
    let telemetry = Arc::new(RingSink::with_line_limit(64, 512));
    let queue = Arc::new(DrainQueue::new(8, 512));
    let sinks = Sinks::new(Arc::clone(&counters))
        .with_tui(Arc::clone(&tui))
        .with_telemetry(Arc::clone(&telemetry))
        .with_file(Arc::clone(&queue));
    let dispatch = Dispatch::new(Registry::default().with(RedactionLayer::new(Mask, sinks)));
    let point = Point { x: 3, y: -4 };

    let (before, after) = tracing::dispatcher::with_default(&dispatch, || {
        // Warm-up: callsite registration, thread-local buffers and their first growth, ring wrap.
        for i in 0..200 {
            emit(i, &point);
        }
        let before = dhat::HeapStats::get();
        for i in 0..1000 {
            emit(i, &point);
        }
        (before, dhat::HeapStats::get())
    });

    assert_eq!(
        after.total_blocks, before.total_blocks,
        "the log enqueue path allocated"
    );
    assert!(
        counters.redactions() >= 1200,
        "redaction ran on every record"
    );
    assert_eq!(counters.redact_failures(), 0);
    assert!(queue.dropped() > 0, "the overflow path was measured too");
    assert_eq!(tui.len(), 64);
    let line = tui.snapshot().into_iter().next().ok_or("empty ring")?;
    assert!(!line.message.contains("sk-fake-secret"));
    assert_eq!(point.x + point.y, -1);
    Ok(())
}
