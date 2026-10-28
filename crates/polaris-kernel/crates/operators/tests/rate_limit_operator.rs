use polaris_kernel_operators::{
    RateLimitInput, RateLimitOp, RateLimitOpConfigError, RateLimitOpError,
};
use polaris_ratelimit::{
    DualLimiter, Estimate, MonotonicClock, Priority, Quota, RateLimitError, ShedPolicy,
};
use polaris_sdk::{ids::TokenCount, registry::FunctionalOperator};
use std::{num::NonZeroU32, time::Duration};
use tokio_util::sync::CancellationToken;

fn quota(burst_requests: u32, burst_tokens: u32) -> Quota {
    Quota {
        rpm: NonZeroU32::MIN,
        tpm: NonZeroU32::MIN,
        burst_requests,
        burst_tokens,
    }
}

fn estimate(prompt: u32, output: u32) -> Estimate {
    Estimate {
        prompt: TokenCount::new(prompt),
        max_output: TokenCount::new(output),
    }
}

fn limiter(quota: Quota, shed: ShedPolicy) -> Result<DualLimiter<MonotonicClock>, RateLimitError> {
    DualLimiter::new(quota, MonotonicClock::default(), shed)
}

#[tokio::test]
async fn admits_and_carries_single_use_reservation() -> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(quota(4, 128), ShedPolicy::default())?;
    let operator = RateLimitOp::<_, String>::new(limiter, Duration::from_secs(1))?;
    let input = RateLimitInput::new(
        String::from("request"),
        estimate(12, 20),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );
    let mut admitted = operator.apply(input).await?;

    assert_eq!(
        admitted.payload(),
        "request",
        "admission preserves the payload"
    );
    assert_eq!(
        admitted.reserved_tokens(),
        32,
        "prompt and output are reserved"
    );
    admitted.mark_dispatched()?;
    let (payload, reconciliation) = admitted.reconcile(27)?;

    assert_eq!(payload, "request", "reconciliation returns the payload");
    assert_eq!(
        reconciliation.reserved_tokens, 32,
        "reservation is reported"
    );
    assert_eq!(
        reconciliation.actual_tokens, 27,
        "provider usage is reported"
    );
    assert!(
        !reconciliation.reservation_overrun,
        "usage was within the estimate"
    );
    Ok(())
}

#[tokio::test]
async fn denies_background_when_shed_by_pressure() -> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(
        quota(4, 128),
        ShedPolicy {
            background_above_pct: 0,
            subagent_above_pct: 0,
            verifier_above_pct: 0,
        },
    )?;
    let operator = RateLimitOp::<_, String>::new(limiter, Duration::from_secs(1))?;
    let input = RateLimitInput::new(
        String::from("background"),
        estimate(1, 1),
        Priority::Background,
        CancellationToken::new(),
    );

    assert!(
        matches!(operator.apply(input).await, Err(RateLimitOpError::Shed(_))),
        "a real priority shedding denial must not become successful admission"
    );

    let interactive = RateLimitInput::new(
        String::from("interactive"),
        estimate(1, 1),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );
    let admitted = operator.apply(interactive).await?;
    assert_eq!(
        admitted.payload(),
        "interactive",
        "interactive requests are exempt from priority shedding"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_estimates_above_token_burst() -> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(quota(4, 8), ShedPolicy::default())?;
    let operator = RateLimitOp::<_, String>::new(limiter, Duration::from_secs(1))?;
    let input = RateLimitInput::new(
        String::from("too large"),
        estimate(5, 4),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );

    assert!(
        matches!(
            operator.apply(input).await,
            Err(RateLimitOpError::Admission(RateLimitError::ExceedsBurst))
        ),
        "an estimate over token capacity must fail closed"
    );
    Ok(())
}

#[tokio::test]
async fn wait_obeys_cancellation_and_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(quota(1, 128), ShedPolicy::default())?;
    let cancellation_limiter = limiter.clone();
    let operator = RateLimitOp::<_, String>::new(limiter, Duration::from_millis(10))?;
    let first = RateLimitInput::new(
        String::from("first"),
        estimate(1, 1),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );
    let _admitted = operator.apply(first).await?;
    let waiting = RateLimitInput::new(
        String::from("second"),
        estimate(1, 1),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );

    assert!(
        matches!(
            operator.apply(waiting).await,
            Err(RateLimitOpError::Timeout)
        ),
        "waiting on exhausted request capacity must respect the finite deadline"
    );

    let cancellation = CancellationToken::new();
    let cancellation_operator =
        RateLimitOp::<_, String>::new(cancellation_limiter, Duration::from_secs(1))?;
    let cancellation_trigger = cancellation.clone();
    let cancellation_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2)).await;
        cancellation_trigger.cancel();
    });
    let canceled = RateLimitInput::new(
        String::from("canceled"),
        estimate(1, 1),
        Priority::InteractiveUser,
        cancellation,
    );
    let cancellation_result = cancellation_operator.apply(canceled).await;
    cancellation_task.await?;
    assert!(
        matches!(cancellation_result, Err(RateLimitOpError::Cancelled)),
        "canceling while waiting returns no permit and drops the wait"
    );
    Ok(())
}

#[tokio::test]
async fn reservation_requires_dispatch_before_reconciliation()
-> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(quota(4, 128), ShedPolicy::default())?;
    let operator = RateLimitOp::<_, String>::new(limiter, Duration::from_secs(1))?;
    let input = RateLimitInput::new(
        String::from("request"),
        estimate(4, 4),
        Priority::InteractiveUser,
        CancellationToken::new(),
    );
    let admitted = operator.apply(input).await?;

    assert!(
        matches!(admitted.reconcile(3), Err(RateLimitError::Undispatched)),
        "provider usage cannot reconcile before transport dispatch"
    );
    Ok(())
}

#[test]
fn zero_timeout_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let limiter = limiter(quota(4, 128), ShedPolicy::default())?;
    assert!(
        matches!(
            RateLimitOp::<_, String>::new(limiter, Duration::ZERO),
            Err(RateLimitOpConfigError)
        ),
        "zero timeout must be rejected during operator construction"
    );
    Ok(())
}
