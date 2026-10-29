//! Ring Push Zero Alloc (descriptor: specs/descriptors/log_ring_push_zero_alloc.json; PLAN ADR-003,
//! hot-path member "log enqueue"). One integration-test binary per zero-alloc check: dhat allows
//! one profiler per process.

use std::time::SystemTime;

use polaris_log::{Level, LogRecordRef, RingSink};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn push_does_not_allocate() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let ring = RingSink::with_line_limit(64, 256);
    let long = "x".repeat(1000);
    let levels = [Level::ERROR, Level::WARN, Level::INFO, Level::DEBUG];

    let rec = |i: usize, message: &'static str| LogRecordRef {
        time: SystemTime::UNIX_EPOCH,
        level: levels.get(i % 4).copied().unwrap_or(Level::INFO),
        target: "polaris::hot",
        message,
    };
    // Warm-up fills the ring once so every slot has been written (and wrapped) before measuring.
    for i in 0..200 {
        ring.push(&rec(i, "warm-up line"));
    }

    let before = dhat::HeapStats::get();
    for i in 0..1000 {
        // Short lines, over-long lines (truncated) and wrap-around overwrites.
        ring.push(&rec(i, "short line"));
        ring.push(&LogRecordRef {
            time: SystemTime::UNIX_EPOCH,
            level: Level::INFO,
            target: "polaris::hot",
            message: &long,
        });
    }
    let after = dhat::HeapStats::get();

    assert_eq!(
        after.total_blocks, before.total_blocks,
        "RingSink::push allocated"
    );
    assert!(ring.dropped() >= 2000, "the overwrite path was exercised");
    Ok(())
}
