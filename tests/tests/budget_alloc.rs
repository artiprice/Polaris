//! PLAN §11.4 allocation budgets (P10-3, ADR-003 hot path): the SSE parser's borrowed path and the
//! GCRA admission check must not allocate per call once warm. dhat's testing mode counts every
//! heap block, so this binary has its own global allocator and a single test.
//!
//! `cargo test --profile budget -p polaris-tests --test budget_alloc -- --nocapture`

use std::error::Error;
use std::num::NonZeroU32;

use polaris_nebius_cloud_token_factory::{SseEventRef, SseParser};
use polaris_ratelimit::{DualLimiter, Estimate, MonotonicClock, Priority, Quota, ShedPolicy};
use polaris_sdk::ids::TokenCount;

#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

#[test]
fn warm_hot_paths_do_not_allocate() -> Result<(), Box<dyn Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();

    // SSE: borrowed events from the parser's bounded frame buffer.
    let frame = b"data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello world\"},\"finish_reason\":null}]}\n\n";
    let mut parser = SseParser::new(64 * 1024, 1_000_000)?;
    let mut events = 0_u64;
    for _ in 0..16 {
        parser.feed_with(frame, |_| Ok(()))?;
    }
    let before = dhat::HeapStats::get();
    for _ in 0..10_000 {
        parser.feed_with(frame, |event| {
            if let SseEventRef::Data(_) = event {
                events += 1;
            }
            Ok(())
        })?;
    }
    let after = dhat::HeapStats::get();
    let sse_blocks = after.total_blocks - before.total_blocks;
    eprintln!(
        "budget SSE frame parse allocations: {sse_blocks} blocks over {events} warm frames (target zero alloc per frame)"
    );

    // GCRA: admission on the RPM and TPM cells.
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
    for _ in 0..100 {
        drop(limiter.try_acquire(estimate, Priority::InteractiveUser));
    }
    let before = dhat::HeapStats::get();
    for _ in 0..10_000 {
        drop(limiter.try_acquire(estimate, Priority::InteractiveUser));
    }
    let after = dhat::HeapStats::get();
    let gcra_blocks = after.total_blocks - before.total_blocks;
    eprintln!(
        "budget GCRA check allocations: {gcra_blocks} blocks over 10000 warm admissions (target zero alloc)"
    );

    assert_eq!(events, 10_000);
    assert_eq!(sse_blocks, 0, "the borrowed SSE path allocates");
    assert_eq!(gcra_blocks, 0, "GCRA admission allocates");
    Ok(())
}
