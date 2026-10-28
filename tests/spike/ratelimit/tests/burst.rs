use polaris_ratelimit::{
    Clock, DenyReason, DualLimiter, Estimate, MonotonicClock, Priority, Quota, RateLimitError,
    ShedPolicy, jittered_backoff,
};
use polaris_sdk::ids::TokenCount;
use std::{
    error::Error,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Default)]
struct FakeClock {
    now_ns: Arc<AtomicU64>,
}

impl FakeClock {
    fn advance(&self, elapsed: Duration) -> Result<(), Box<dyn Error + Send + Sync>> {
        let elapsed_ns = u64::try_from(elapsed.as_nanos())?;
        self.now_ns
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(elapsed_ns)
            })
            .map(|_| ())
            .map_err(|_| "fake clock overflow".into())
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.now_ns.load(Ordering::Acquire))
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    priority: Priority,
    admitted: bool,
    latency_ns: u64,
}

fn token_count(value: u32) -> TokenCount {
    TokenCount::new(value)
}

fn estimate(tokens: u32) -> Estimate {
    Estimate {
        prompt: token_count(tokens),
        max_output: token_count(0),
    }
}

fn quota(
    rpm: u32,
    tpm: u32,
    burst_requests: u32,
    burst_tokens: u32,
) -> Result<Quota, Box<dyn Error + Send + Sync>> {
    Ok(Quota {
        rpm: NonZeroU32::new(rpm).ok_or("rpm must be nonzero")?,
        tpm: NonZeroU32::new(tpm).ok_or("tpm must be nonzero")?,
        burst_requests,
        burst_tokens,
    })
}

fn limiter<C: Clock>(
    quota: Quota,
    clock: C,
    shed: ShedPolicy,
) -> Result<DualLimiter<C>, Box<dyn Error + Send + Sync>> {
    Ok(DualLimiter::new(quota, clock, shed)?)
}

// Workload samples model dispatched requests. An undispatched permit now correctly refunds on drop.
fn dispatched_admission<C: Clock>(
    load: &DualLimiter<C>,
    priority: Priority,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
    match load.try_acquire(estimate(1), priority) {
        Ok(mut permit) => {
            permit.mark_dispatched()?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

fn nearest_rank(samples: &[u64], percentile: u32) -> Option<u64> {
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let len = u64::try_from(ordered.len()).ok()?;
    let rank = len
        .checked_mul(u64::from(percentile))?
        .checked_add(99)?
        .checked_div(100)?;
    let rank = usize::try_from(rank.checked_sub(1)?).ok()?;
    ordered.iter().copied().nth(rank)
}

fn observed_elapsed_ns(started: Instant) -> Result<u64, std::num::TryFromIntError> {
    let capped = started.elapsed().as_nanos().min(u128::from(u64::MAX));
    u64::try_from(capped)
}

fn boxed_error<E: Error + Send + Sync + 'static>(error: E) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}

fn report_path(scenario: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = root.join("../../../target/spike");
    fs::create_dir_all(&target)?;
    Ok(target.join(format!("ratelimit-{scenario}.json")))
}

fn write_report(
    scenario: &str,
    samples: &[Sample],
    retry_spread_ns: Option<u64>,
    canceled: usize,
    outstanding: usize,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let admitted = samples.iter().filter(|sample| sample.admitted).count();
    let shed = samples.iter().filter(|sample| !sample.admitted).count();
    let latencies = samples
        .iter()
        .map(|sample| sample.latency_ns)
        .collect::<Vec<_>>();
    let p50 =
        nearest_rank(&latencies, 50).map_or_else(|| "null".to_owned(), |value| value.to_string());
    let p99 =
        nearest_rank(&latencies, 99).map_or_else(|| "null".to_owned(), |value| value.to_string());
    let priorities = samples
        .iter()
        .map(|sample| format!("\"{:?}\"", sample.priority))
        .collect::<Vec<_>>()
        .join(",");
    let retry_spread_ns =
        retry_spread_ns.map_or_else(|| "null".to_owned(), |value| value.to_string());
    let body = format!(
        "{{\"admitted\":{admitted},\"shed\":{shed},\"canceled_waiters\":{canceled},\"outstanding_tasks\":{outstanding},\"retry_spread_ns\":{retry_spread_ns},\"latency_p50_ns\":{p50},\"latency_p99_ns\":{p99},\"sample_priorities\":[{priorities}]}}"
    );
    fs::write(report_path(scenario)?, body)?;
    Ok(())
}

#[test]
fn fifty_agents_priority_order_and_rate_bound() -> Result<(), Box<dyn Error + Send + Sync>> {
    let clock = FakeClock::default();
    let load = limiter(
        quota(60, 100_000, 24, 100_000)?,
        clock.clone(),
        ShedPolicy {
            background_above_pct: 100,
            subagent_above_pct: 100,
            verifier_above_pct: 100,
        },
    )?;
    let mut priorities = (0..50)
        .map(|index| match index % 4 {
            0 => Priority::Background,
            1 => Priority::SubAgent,
            2 => Priority::Verifier,
            _ => Priority::InteractiveUser,
        })
        .collect::<Vec<_>>();
    priorities.sort_by(|left, right| right.cmp(left));

    let samples = priorities
        .iter()
        .copied()
        .map(|priority| {
            let started = Instant::now();
            let admitted = dispatched_admission(&load, priority)?;
            let latency_ns = observed_elapsed_ns(started).map_err(boxed_error)?;
            Ok(Sample {
                priority,
                admitted,
                latency_ns,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error + Send + Sync>>>()?;

    let mut saw_denial = false;
    let mut last_admitted_priority = Priority::InteractiveUser;
    for sample in &samples {
        if sample.admitted {
            assert!(
                !saw_denial,
                "a lower-ranked request was admitted after a higher-priority denial"
            );
            assert!(
                sample.priority <= last_admitted_priority,
                "admission order must be descending priority"
            );
            last_admitted_priority = sample.priority;
        } else {
            saw_denial = true;
        }
    }
    let admitted = samples.iter().filter(|sample| sample.admitted).count();
    let rpm_plus_burst = 60_u64.checked_add(24).ok_or("quota sum overflow")?;
    assert!(
        u64::try_from(admitted)? <= rpm_plus_burst,
        "observed request count exceeded RPM plus configured burst"
    );
    assert_eq!(
        admitted, 24,
        "configured GCRA burst is the maximum same-tick admission count"
    );
    assert!(
        samples
            .iter()
            .filter(|sample| sample.admitted)
            .all(|sample| sample.priority >= Priority::Verifier),
        "InteractiveUser and Verifier must be admitted before lower priorities"
    );
    let latencies = samples
        .iter()
        .map(|sample| sample.latency_ns)
        .collect::<Vec<_>>();
    assert!(
        nearest_rank(&latencies, 50).is_some(),
        "p50 must come from observed samples"
    );
    assert!(
        nearest_rank(&latencies, 99).is_some(),
        "p99 must come from observed samples"
    );
    write_report("priority-burst", &samples, None, 0, 0)?;
    Ok(())
}

#[test]
fn background_shed_first_under_pressure() -> Result<(), Box<dyn Error + Send + Sync>> {
    let clock = FakeClock::default();
    let load = limiter(
        quota(60, 60, 4, 10)?,
        clock,
        ShedPolicy {
            background_above_pct: 50,
            subagent_above_pct: 75,
            verifier_above_pct: 95,
        },
    )?;
    let first = load.try_acquire(estimate(1), Priority::InteractiveUser);
    assert!(
        first.is_ok(),
        "interactive admission should succeed before pressure"
    );
    let second_interactive = load.try_acquire(estimate(1), Priority::InteractiveUser);
    assert!(
        second_interactive.is_ok(),
        "interactive traffic may consume the RPM burst"
    );
    let second = load.try_acquire(estimate(1), Priority::Background);
    assert!(
        matches!(second, Err(denied) if denied.reason == DenyReason::Shed(Priority::Background)),
        "background should shed at configured pressure"
    );
    Ok(())
}

#[test]
fn retry_after_storm_respects_floor_and_shared_penalty() -> Result<(), Box<dyn Error + Send + Sync>>
{
    let clock = FakeClock::default();
    let load = limiter(
        quota(60, 60_000, 60, 60_000)?,
        clock.clone(),
        ShedPolicy::default(),
    )?;
    let mut retries = Vec::new();
    let mut seconds = 1_u64;
    while seconds <= 30 {
        let floor = Duration::from_secs(seconds);
        load.on_response(429, Some(floor))?;
        let retry = jittered_backoff(u32::try_from(seconds)?, Some(floor));
        assert!(
            retry >= floor,
            "jittered retry must not precede Retry-After"
        );
        retries.push(retry.as_nanos());
        let denied = load.try_acquire(estimate(1), Priority::InteractiveUser);
        assert!(
            matches!(denied, Err(value) if value.reason == DenyReason::PenaltyActive),
            "shared limiter must reject during Retry-After penalty"
        );
        clock.advance(Duration::from_secs(2))?;
        seconds = seconds.checked_add(1).ok_or("storm iteration overflow")?;
    }
    clock.advance(Duration::from_secs(31))?;
    let admitted = load.try_acquire(estimate(1), Priority::InteractiveUser);
    assert!(
        admitted.is_ok(),
        "admission must recover after the final shared penalty expires"
    );
    let min = retries.iter().min().copied().ok_or("missing retries")?;
    let max = retries.iter().max().copied().ok_or("missing retries")?;
    let spread = u64::try_from(max.checked_sub(min).ok_or("retry spread underflow")?)?;
    assert!(
        spread >= 29_000_000_000,
        "1 to 30 second Retry-After floors must produce at least 29 seconds of observed spread"
    );
    write_report("retry-storm", &[], Some(spread), 0, 0)?;
    Ok(())
}

#[test]
fn tpm_reservation_reconciliation_and_overshoot() -> Result<(), Box<dyn Error + Send + Sync>> {
    let clock = FakeClock::default();
    let load = limiter(quota(60, 10, 4, 8)?, clock.clone(), ShedPolicy::default())?;
    let mut reserved = match load.try_acquire(estimate(8), Priority::InteractiveUser) {
        Ok(permit) => permit,
        Err(denied) => return Err(format!("initial TPM reservation denied: {denied:?}").into()),
    };
    assert_eq!(
        reserved.reserved_tokens(),
        8,
        "reservation must use the requested upper bound"
    );
    reserved.mark_dispatched()?;
    let low_usage = reserved.reconcile(3)?;
    assert_eq!(low_usage.actual_tokens, 3, "actual usage must be recorded");
    assert!(
        !low_usage.reservation_overrun,
        "usage below reservation must not be an overrun"
    );
    assert_eq!(
        low_usage.refunded_tokens, 5,
        "exact tail returns unused TPM"
    );
    assert_eq!(
        low_usage.unused_tokens_retained, 0,
        "tail settlement retains no unused TPM"
    );
    let mut reused = load
        .try_acquire(estimate(5), Priority::InteractiveUser)
        .map_err(|denied| format!("refunded budget unavailable: {denied:?}"))?;
    reused.mark_dispatched()?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "refund must preserve the three actual and five newly dispatched tokens"
    );

    clock.advance(Duration::from_secs(60))?;
    let mut overshoot = match load.try_acquire(estimate(8), Priority::InteractiveUser) {
        Ok(permit) => permit,
        Err(denied) => return Err(format!("overshoot reservation denied: {denied:?}").into()),
    };
    overshoot.mark_dispatched()?;
    let result = overshoot.reconcile(12)?;
    assert!(
        result.reservation_overrun,
        "actual usage above estimate must be flagged"
    );
    assert_eq!(
        result.actual_tokens, 12,
        "reported actual usage must be retained"
    );
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_err(),
        "overrun must charge TPM before returning"
    );
    clock.advance(Duration::from_secs(120))?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_ok(),
        "later request proceeds after the token debt decays"
    );
    Ok(())
}

#[test]
fn retry_after_flood_is_status_gated_and_checked() -> Result<(), Box<dyn Error + Send + Sync>> {
    let clock = FakeClock::default();
    let load = limiter(
        quota(60, 60_000, 60, 60_000)?,
        clock.clone(),
        ShedPolicy::default(),
    )?;
    load.on_response(500, Some(Duration::from_secs(30)))?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_ok(),
        "non-429 Retry-After must not create a penalty"
    );
    load.on_response(429, Some(Duration::from_secs(30)))?;
    load.on_response(429, Some(Duration::from_secs(1)))?;
    let denied = load.try_acquire(estimate(1), Priority::InteractiveUser);
    assert!(
        matches!(denied, Err(value) if value.reason == DenyReason::PenaltyActive && value.retry_after >= Duration::from_secs(30)),
        "short contradictory duplicate must not shorten the active penalty"
    );
    assert!(
        matches!(
            load.on_response(429, Some(Duration::MAX)),
            Err(RateLimitError::Accounting)
        ),
        "unrepresentable parsed duration must be a typed accounting error"
    );
    clock.advance(Duration::from_secs(30))?;
    assert!(
        load.try_acquire(estimate(1), Priority::InteractiveUser)
            .is_ok(),
        "penalty expires at its monotonic deadline"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceled_waiters_are_drained_without_task_leaks()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let clock = FakeClock::default();
    let load = limiter(
        quota(60_000, 100, 4, 100)?,
        clock.clone(),
        ShedPolicy::default(),
    )?;
    for _ in 0..4 {
        assert!(
            dispatched_admission(&load, Priority::InteractiveUser)?,
            "initial dispatched requests consume the immediate RPM burst"
        );
    }
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let clone = load.clone();
        tasks.push(tokio::spawn(async move {
            let started = Instant::now();
            let permit = clone
                .acquire(estimate(1), Priority::InteractiveUser)
                .await
                .map_err(boxed_error)?;
            let latency_ns = observed_elapsed_ns(started).map_err(boxed_error)?;
            Ok::<_, Box<dyn Error + Send + Sync>>((permit, latency_ns))
        }));
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    for task in tasks.iter().take(4) {
        task.abort();
    }
    clock.advance(Duration::from_secs(60))?;
    let mut canceled = 0_usize;
    let mut completed = 0_usize;
    let mut completion_samples = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Ok((permit, latency_ns))) => {
                drop(permit);
                completed = completed
                    .checked_add(1)
                    .ok_or("completion count overflow")?;
                completion_samples.push(Sample {
                    priority: Priority::InteractiveUser,
                    admitted: true,
                    latency_ns,
                });
            }
            Ok(Err(error)) => return Err(error),
            Err(error) if error.is_cancelled() => {
                canceled = canceled
                    .checked_add(1)
                    .ok_or("cancellation count overflow")?;
            }
            Err(error) => {
                let failure: Box<dyn Error + Send + Sync> = Box::new(error);
                return Err(failure);
            }
        }
    }
    assert_eq!(
        canceled, 4,
        "exactly half of the waiters should be canceled"
    );
    assert_eq!(completed, 4, "remaining waiters should drain successfully");
    let outstanding = 8_usize
        .checked_sub(canceled.checked_add(completed).ok_or("task sum overflow")?)
        .ok_or("task count mismatch")?;
    write_report(
        "cancellation",
        &completion_samples,
        None,
        canceled,
        outstanding,
    )?;
    Ok(())
}

#[test]
fn report_latency_samples_are_observed_and_real_clock_can_be_sampled()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let load = limiter(
        quota(6_000, 6_000_000, 100, 100_000)?,
        MonotonicClock::default(),
        ShedPolicy::default(),
    )?;
    let mut samples = Vec::new();
    for index in 0_u32..50 {
        let priority = if index % 2 == 0 {
            Priority::InteractiveUser
        } else {
            Priority::Verifier
        };
        let started = Instant::now();
        let admitted = dispatched_admission(&load, priority)?;
        let latency_ns = observed_elapsed_ns(started)?;
        samples.push(Sample {
            priority,
            admitted,
            latency_ns,
        });
    }
    assert!(
        samples.iter().all(|sample| sample.admitted),
        "configured quota should admit all 50 observed samples"
    );
    let latencies = samples
        .iter()
        .map(|sample| sample.latency_ns)
        .collect::<Vec<_>>();
    assert!(
        nearest_rank(&latencies, 50).is_some(),
        "p50 derives from measured calls"
    );
    assert!(
        nearest_rank(&latencies, 99).is_some(),
        "p99 derives from measured calls"
    );
    write_report("real-clock", &samples, None, 0, 0)?;
    Ok(())
}
