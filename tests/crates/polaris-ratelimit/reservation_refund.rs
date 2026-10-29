use polaris_ratelimit::{Clock, DualLimiter, Estimate, Priority, Quota, ShedPolicy};
use polaris_sdk::ids::TokenCount;
use std::{
    error::Error,
    num::NonZeroU32,
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Debug, Clone, Default)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn advance(&self, duration: Duration) -> Result<(), Box<dyn Error>> {
        let ns = u64::try_from(duration.as_nanos())?;
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |now| {
                now.checked_add(ns)
            })
            .map(|_| ())
            .map_err(|_| "fake clock overflow".into())
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(Ordering::Acquire))
    }
}

fn quota(tpm: u32, burst_tokens: u32) -> Result<Quota, Box<dyn Error>> {
    Ok(Quota {
        rpm: NonZeroU32::new(60_000).ok_or("rpm must be nonzero")?,
        tpm: NonZeroU32::new(tpm).ok_or("tpm must be nonzero")?,
        burst_requests: 100,
        burst_tokens,
    })
}

fn estimate(tokens: u32) -> Estimate {
    Estimate {
        prompt: TokenCount::new(tokens),
        max_output: TokenCount::new(0),
    }
}

fn limiter(
    tpm: u32,
    burst_tokens: u32,
    clock: FakeClock,
) -> Result<DualLimiter<FakeClock>, Box<dyn Error>> {
    Ok(DualLimiter::new(
        quota(tpm, burst_tokens)?,
        clock,
        ShedPolicy {
            background_above_pct: 100,
            subagent_above_pct: 100,
            verifier_above_pct: 100,
        },
    )?)
}

#[test]
fn reconciliation_returns_unused_tokens() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut permit = match load.try_acquire(estimate(10), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("initial request denied: {denied:?}").into()),
    };
    permit.mark_dispatched()?;
    let result = permit.reconcile(0)?;
    assert_eq!(result.reserved_tokens, 10);
    assert_eq!(result.actual_tokens, 0);
    assert_eq!(result.refunded_tokens, 10);
    assert_eq!(result.unused_tokens_retained, 0);
    let next = load.try_acquire(estimate(10), Priority::InteractiveUser);
    assert!(
        next.is_ok(),
        "unused ten-token reservation must return immediately"
    );
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the returned five tokens must not exceed the original capacity"
    );
    Ok(())
}

#[test]
fn tail_reconciliation_reports_partial_refund_exactly() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut permit = match load.try_acquire(estimate(10), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("initial request denied: {denied:?}").into()),
    };
    permit.mark_dispatched()?;
    let result = permit.reconcile(4)?;
    assert_eq!(result.refunded_tokens, 6);
    assert_eq!(result.unused_tokens_retained, 0);
    let mut next = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("returned six-token tail denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "partial tail settlement must preserve the four actual tokens plus the new six-token request"
    );
    Ok(())
}

#[test]
fn concurrent_refund_preserves_other_reservations() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut first = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    let mut second = match load.try_acquire(estimate(4), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    let result = first.reconcile(2)?;
    assert!(!result.reservation_overrun);
    assert_eq!(result.refunded_tokens, 0);
    assert_eq!(result.unused_tokens_retained, 4);
    let refunded = load.try_acquire(estimate(4), Priority::InteractiveUser);
    assert!(
        refunded.is_ok(),
        "first permit returns only its four unused tokens"
    );
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "second permit's four-token reservation remains charged"
    );
    drop(second);
    Ok(())
}

#[test]
fn same_time_non_tail_refund_preserves_newer_reservation() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut first = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    let mut second = match load.try_acquire(estimate(4), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    let result = first.reconcile(2)?;
    assert_eq!(result.refunded_tokens, 4);
    assert_eq!(result.unused_tokens_retained, 0);
    let mut replacement = match load.try_acquire(estimate(4), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("returned four-token tail denied: {denied:?}").into()),
    };
    replacement.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the later four-token reservation remains charged alongside actual usage and replacement"
    );
    drop(second);
    Ok(())
}

#[test]
fn staggered_non_tail_refund_preserves_later_admission_floor() -> Result<(), Box<dyn Error>> {
    let clock = FakeClock::default();
    let load = limiter(60, 10, clock.clone())?;
    let mut first = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    clock.advance(Duration::from_secs(3))?;
    let mut second = match load.try_acquire(estimate(4), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    let result = first.reconcile(2)?;
    assert_eq!(result.refunded_tokens, 3);
    assert_eq!(result.unused_tokens_retained, 1);
    let mut next = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("floor-preserving request denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the later admission-time floor and both live costs remain charged"
    );
    drop(second);
    Ok(())
}

#[test]
fn out_of_order_same_time_refunds_recover_only_settled_capacity() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut first = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    let mut second = match load.try_acquire(estimate(2), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    let mut third = match load.try_acquire(estimate(2), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("third reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    third.mark_dispatched()?;
    let second_result = second.reconcile(0)?;
    assert_eq!(second_result.refunded_tokens, 2);
    let first_result = first.reconcile(2)?;
    assert_eq!(first_result.refunded_tokens, 4);
    assert_eq!(first_result.unused_tokens_retained, 0);
    let mut next = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("recovered-capacity request denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the unsettled third reservation remains charged"
    );
    drop(third);
    Ok(())
}

#[test]
fn earlier_refund_respects_multiple_later_time_floors() -> Result<(), Box<dyn Error>> {
    let clock = FakeClock::default();
    let load = limiter(60, 10, clock.clone())?;
    let mut first = match load.try_acquire(estimate(6), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    let mut second = match load.try_acquire(estimate(2), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    clock.advance(Duration::from_secs(3))?;
    let mut third = match load.try_acquire(estimate(2), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("third reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    third.mark_dispatched()?;
    let result = first.reconcile(0)?;
    assert_eq!(result.refunded_tokens, 5);
    assert_eq!(result.unused_tokens_retained, 1);
    let mut next = match load.try_acquire(estimate(8), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("floor-preserving request denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the intervening request and later time floor remain charged"
    );
    drop(second);
    drop(third);
    Ok(())
}

#[test]
fn expired_debt_does_not_report_refund() -> Result<(), Box<dyn Error>> {
    let clock = FakeClock::default();
    let load = limiter(60, 10, clock.clone())?;
    let mut first = match load.try_acquire(estimate(4), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    let mut second = match load.try_acquire(estimate(2), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    clock.advance(Duration::from_secs(10))?;
    let result = first.reconcile(0)?;
    assert_eq!(result.refunded_tokens, 0);
    assert_eq!(result.unused_tokens_retained, 4);
    assert!(!result.settlement_deferred);
    drop(second);
    Ok(())
}

#[test]
fn concurrent_snapshot_admissions_never_exceed_burst() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    // 24 worker threads plus this coordinating thread rendezvous together.
    let barrier = Arc::new(Barrier::new(25));
    let mut handles = Vec::new();
    for _ in 0..24 {
        let clone = load.clone();
        let start = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            start.wait();
            for _ in 0..64 {
                match clone.try_acquire(estimate(1), Priority::InteractiveUser) {
                    Ok(mut permit) => {
                        let mut dispatched = false;
                        for _ in 0..64 {
                            match permit.mark_dispatched() {
                                Ok(()) => {
                                    dispatched = true;
                                    break;
                                }
                                Err(polaris_ratelimit::RateLimitError::Busy) => {
                                    thread::yield_now();
                                }
                                Err(_) => return false,
                            }
                        }
                        if !dispatched {
                            return false;
                        }
                        if permit.reconcile(1).is_err() {
                            return false;
                        }
                        return true;
                    }
                    Err(denied) if denied.reason == polaris_ratelimit::DenyReason::Busy => {
                        thread::yield_now();
                    }
                    Err(_) => return false,
                }
            }
            false
        }));
    }
    barrier.wait();
    let mut admitted = 0_u32;
    for handle in handles {
        match handle.join() {
            Ok(true) => admitted = admitted.checked_add(1).ok_or("admission count overflow")?,
            Ok(false) => {}
            Err(_) => return Err("worker thread panicked".into()),
        }
    }
    assert_eq!(
        admitted, 10,
        "only the configured token burst may be admitted"
    );
    Ok(())
}

#[test]
fn undispatched_drop_refunds_reservation() -> Result<(), Box<dyn Error>> {
    let load = DualLimiter::new(
        Quota {
            rpm: NonZeroU32::new(1).ok_or("rpm must be nonzero")?,
            tpm: NonZeroU32::new(60).ok_or("tpm must be nonzero")?,
            burst_requests: 1,
            burst_tokens: 10,
        },
        FakeClock::default(),
        ShedPolicy {
            background_above_pct: 100,
            subagent_above_pct: 100,
            verifier_above_pct: 100,
        },
    )?;
    let permit = match load.try_acquire(estimate(10), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("initial reservation denied: {denied:?}").into()),
    };
    drop(permit);
    let mut next = match load.try_acquire(estimate(10), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => {
            return Err(
                format!("undispatched rollback did not restore both cells: {denied:?}").into(),
            );
        }
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "the dispatched follow-up retains both its RPM and TPM reservations"
    );
    Ok(())
}

#[test]
fn dispatched_drop_keeps_reservation_charged() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut permit = match load.try_acquire(estimate(10), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("initial reservation denied: {denied:?}").into()),
    };
    permit.mark_dispatched()?;
    drop(permit);
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "possibly dispatched permit drop retains the full reservation"
    );
    Ok(())
}

#[test]
fn overrun_charges_actual_usage_before_return() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 20, FakeClock::default())?;
    let mut permit = match load.try_acquire(estimate(8), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("initial reservation denied: {denied:?}").into()),
    };
    permit.mark_dispatched()?;
    let result = permit.reconcile(12)?;
    assert!(result.reservation_overrun);
    assert_eq!(result.refunded_tokens, 0);
    assert_eq!(result.unused_tokens_retained, 0);
    let mut next = match load.try_acquire(estimate(8), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("post-overrun request denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    let after_overrun = load.try_acquire(estimate(1), Priority::InteractiveUser);
    assert!(
        after_overrun.is_err(),
        "four-token overrun must be charged before reconcile returns; result: {:?}",
        after_overrun.err()
    );
    Ok(())
}

#[test]
fn late_refund_never_erases_another_request_charge() -> Result<(), Box<dyn Error>> {
    let clock = FakeClock::default();
    let load = limiter(60, 10, clock.clone())?;
    let mut first = match load.try_acquire(estimate(9), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    clock.advance(Duration::from_secs(3))?;
    let mut second = match load.try_acquire(estimate(9), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    let result = first.reconcile(0)?;
    assert_eq!(result.actual_tokens, 0);
    assert_eq!(result.refunded_tokens, 0);
    assert_eq!(result.unused_tokens_retained, 9);
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_ok(),
        "late refund should admit from A's unused capacity"
    );
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "B's latest debit must remain in the GCRA tail; stale scalar subtraction would admit more"
    );
    drop(second);
    Ok(())
}

#[test]
fn expired_older_reservation_cannot_refund_newer_live_debt() -> Result<(), Box<dyn Error>> {
    let clock = FakeClock::default();
    let load = limiter(60, 10, clock.clone())?;
    let mut first = match load.try_acquire(estimate(9), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    clock.advance(Duration::from_secs(3))?;
    let mut second = match load.try_acquire(estimate(9), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    second.mark_dispatched()?;
    clock.advance(Duration::from_secs(6))?;
    let result = first.reconcile(0)?;
    assert_eq!(result.refunded_tokens, 0);
    assert_eq!(result.unused_tokens_retained, 9);
    let mut next = match load.try_acquire(estimate(1), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first tail request denied: {denied:?}").into()),
    };
    next.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "an expired first reservation cannot erase the newer reservation's final unit of debt"
    );
    drop(second);
    Ok(())
}

#[test]
fn aba_after_zero_usage_settlement_leaves_no_other_debt() -> Result<(), Box<dyn Error>> {
    let load = limiter(60, 10, FakeClock::default())?;
    let mut first = match load.try_acquire(estimate(9), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("first reservation denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    let mut second = match load.try_acquire(estimate(1), Priority::InteractiveUser) {
        Ok(value) => value,
        Err(denied) => return Err(format!("second reservation denied: {denied:?}").into()),
    };
    second.mark_dispatched()?;
    let second_result = second.reconcile(0)?;
    assert_eq!(second_result.refunded_tokens, 1);
    let result = first.reconcile(0)?;
    assert_eq!(result.refunded_tokens, 9);
    assert_eq!(result.unused_tokens_retained, 0);
    assert!(
        load.try_acquire(estimate(10), Priority::InteractiveUser)
            .is_ok(),
        "the ABA restored only a zero-usage tail; no other reservation debt remains"
    );
    Ok(())
}

#[test]
fn naive_scalar_refund_erases_fresh_request_debt() -> Result<(), Box<dyn Error>> {
    // At 60 TPM, one token is one second of GCRA debt. A reserves eight at t=0;
    // at t=3, B adds two, so aggregate TAT becomes 10. Removing A's eight seconds
    // from that scalar yields max(3, 2) = 3, while B alone should leave TAT at 5.
    let now_ns = 3_000_000_000_u64;
    let a_delta_ns = 8_000_000_000_u64;
    let aggregate_tat_ns = 10_000_000_000_u64;
    let b_only_tat_ns = now_ns
        .checked_add(2_000_000_000)
        .ok_or("expected TAT overflow")?;
    let naive_refunded_tat_ns = aggregate_tat_ns.saturating_sub(a_delta_ns).max(now_ns);
    assert_eq!(naive_refunded_tat_ns, 3_000_000_000);
    assert_eq!(b_only_tat_ns, 5_000_000_000);
    assert_ne!(
        naive_refunded_tat_ns, b_only_tat_ns,
        "a late scalar subtraction cannot preserve a newer request's debt"
    );
    Ok(())
}
