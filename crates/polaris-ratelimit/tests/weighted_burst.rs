//! Exact weighted admission boundaries with a synthetic monotonic clock.
use polaris_ratelimit::{Clock, DenyReason, DualLimiter, Estimate, Priority, Quota, ShedPolicy};
use polaris_sdk::ids::TokenCount;
use std::{
    error::Error,
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
#[derive(Clone, Default)]
struct FixedClock(Arc<AtomicU64>);
impl Clock for FixedClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(Ordering::Acquire))
    }
}
fn limiter(clock: FixedClock) -> Result<DualLimiter<FixedClock>, Box<dyn Error>> {
    Ok(DualLimiter::new(
        Quota {
            rpm: NonZeroU32::new(60_000).ok_or("rpm")?,
            tpm: NonZeroU32::new(60).ok_or("tpm")?,
            burst_requests: 100,
            burst_tokens: 10,
        },
        clock,
        ShedPolicy::default(),
    )?)
}
fn estimate(cost: u32) -> Estimate {
    Estimate {
        prompt: TokenCount::new(cost),
        max_output: TokenCount::new(0),
    }
}
#[test]
fn weighted_request_cannot_overfill_remaining_burst() -> Result<(), Box<dyn Error>> {
    let limiter = limiter(FixedClock::default())?;
    let mut first = limiter
        .try_acquire(estimate(8), Priority::InteractiveUser)
        .map_err(|e| format!("eight denied: {e:?}"))?;
    first.mark_dispatched()?;
    let excess = limiter.try_acquire(estimate(3), Priority::InteractiveUser);
    assert!(
        excess.is_err(),
        "a three-token request must not occupy the two-token remainder"
    );
    let mut remainder = limiter
        .try_acquire(estimate(2), Priority::InteractiveUser)
        .map_err(|e| format!("remaining two denied: {e:?}"))?;
    remainder.mark_dispatched()?;
    assert!(
        limiter
            .try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "exact capacity is exhausted"
    );
    Ok(())
}
#[test]
fn weighted_retry_accounts_for_the_entire_request() -> Result<(), Box<dyn Error>> {
    let clock = FixedClock::default();
    let limiter = limiter(clock.clone())?;
    let mut first = limiter
        .try_acquire(estimate(8), Priority::InteractiveUser)
        .map_err(|e| format!("eight denied: {e:?}"))?;
    first.mark_dispatched()?;
    let denied = match limiter.try_acquire(estimate(5), Priority::InteractiveUser) {
        Err(value) => value,
        Ok(_) => return Err("five admitted into two-token remainder".into()),
    };
    assert_eq!(denied.reason, DenyReason::TpmExhausted);
    assert_eq!(denied.retry_after, Duration::from_secs(3));
    clock.0.store(2_999_999_999, Ordering::Release);
    assert!(
        limiter
            .try_acquire(estimate(5), Priority::InteractiveUser)
            .is_err(),
        "one nanosecond before weighted capacity is ready"
    );
    clock.0.store(3_000_000_000, Ordering::Release);
    let mut ready = limiter
        .try_acquire(estimate(5), Priority::InteractiveUser)
        .map_err(|e| format!("ready weighted admission denied: {e:?}"))?;
    ready.mark_dispatched()?;
    assert!(
        limiter
            .try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "weighted admission leaves no spare debt"
    );
    Ok(())
}
