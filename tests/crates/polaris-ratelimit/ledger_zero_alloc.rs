//! Zero-allocation evidence for the limiter admission and reconciliation path (ADR-003).

use polaris_ratelimit::{Clock, DualLimiter, Estimate, Priority, Quota, ShedPolicy};
use polaris_sdk::ids::TokenCount;
use std::{num::NonZeroU32, sync::atomic::AtomicU64, time::Duration};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[derive(Debug, Default)]
struct FixedClock(AtomicU64);

impl Clock for FixedClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(std::sync::atomic::Ordering::Acquire))
    }
}

#[test]
fn reserve_dispatch_and_reconcile_do_not_allocate() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let limiter = DualLimiter::new(
        Quota {
            rpm: NonZeroU32::new(60_000).ok_or("rpm must be nonzero")?,
            tpm: NonZeroU32::new(60_000).ok_or("tpm must be nonzero")?,
            burst_requests: 2_000,
            burst_tokens: 2_000,
        },
        FixedClock::default(),
        ShedPolicy {
            background_above_pct: 100,
            subagent_above_pct: 100,
            verifier_above_pct: 100,
        },
    )?;
    let estimate = Estimate {
        prompt: TokenCount::new(1),
        max_output: TokenCount::new(0),
    };

    // Warm the atomic snapshot pool and intent slots before taking the allocation baseline.
    for _ in 0..16 {
        let Ok(mut permit) = limiter.try_acquire(estimate, Priority::InteractiveUser) else {
            return Err("warmup admission was denied".into());
        };
        permit.mark_dispatched()?;
        let _ = permit.reconcile(1)?;
    }

    let before = dhat::HeapStats::get();
    let mut successes = 0_u32;
    for _ in 0..512 {
        if let Ok(mut permit) = limiter.try_acquire(estimate, Priority::InteractiveUser) {
            if permit.mark_dispatched().is_ok() && permit.reconcile(1).is_ok() {
                successes = successes.saturating_add(1);
            }
        }
    }
    let after = dhat::HeapStats::get();

    assert_eq!(
        successes, 512,
        "all warmed-up operations should be admitted"
    );
    assert_eq!(
        after.total_blocks, before.total_blocks,
        "quota hot path allocated blocks"
    );
    assert_eq!(
        after.total_bytes, before.total_bytes,
        "quota hot path allocated bytes"
    );
    Ok(())
}
