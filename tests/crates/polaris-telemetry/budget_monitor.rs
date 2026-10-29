//! Budget Monitor (descriptor: specs/descriptors/telemetry_budget_monitor.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, SessionId, TokenCount};
use polaris_telemetry::{Budget, BudgetReport, BudgetStatus, Telemetry, TelemetryError};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn request(prompt: u32, completion: u32, cached: u32, cost: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: FrameId::from_u128(1),
        request: Cid::from_bytes([1; 32]),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(completion),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(cost),
        latency_us: 1,
    }
}

fn usd_budget(limit: u64) -> Budget {
    Budget::new(Some(MicroUsd::new(limit)), None)
}

fn spent(cost: u64) -> Result<Telemetry, TelemetryError> {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record(&request(1, 1, 0, cost));
    Ok(telemetry)
}

#[test]
fn unlimited_budget_is_ok() -> TestResult {
    let telemetry = spent(u64::MAX)?;
    let unlimited = Budget::new(None, None);
    assert_eq!(telemetry.budget(&unlimited), BudgetStatus::Ok);
    assert_eq!(
        telemetry.budget_report(&unlimited),
        BudgetReport {
            usd: BudgetStatus::Ok,
            tokens: BudgetStatus::Ok
        }
    );
    Ok(())
}

#[test]
fn below_threshold_is_ok() -> TestResult {
    assert_eq!(spent(0)?.budget(&usd_budget(1_000)), BudgetStatus::Ok);
    assert_eq!(spent(500)?.budget(&usd_budget(1_000)), BudgetStatus::Ok);
    Ok(())
}

#[test]
fn at_80_percent_warns() -> TestResult {
    assert_eq!(spent(799)?.budget(&usd_budget(1_000)), BudgetStatus::Ok);
    assert_eq!(
        spent(800)?.budget(&usd_budget(1_000)),
        BudgetStatus::Warn { used_pct: 80 }
    );
    assert_eq!(
        spent(999)?.budget(&usd_budget(1_000)),
        BudgetStatus::Warn { used_pct: 99 }
    );
    Ok(())
}

#[test]
fn at_100_percent_exceeded() -> TestResult {
    assert_eq!(
        spent(1_000)?.budget(&usd_budget(1_000)),
        BudgetStatus::Exceeded
    );
    assert_eq!(
        spent(5_000)?.budget(&usd_budget(1_000)),
        BudgetStatus::Exceeded
    );
    Ok(())
}

#[test]
fn token_and_usd_limits_checked_independently() -> TestResult {
    let budget = Budget::new(Some(MicroUsd::new(1_000)), Some(10_000));

    let mut money_heavy = Telemetry::new(SESSION)?;
    money_heavy.record(&request(50, 50, 0, 900));
    assert_eq!(
        money_heavy.budget_report(&budget),
        BudgetReport {
            usd: BudgetStatus::Warn { used_pct: 90 },
            tokens: BudgetStatus::Ok
        }
    );
    assert_eq!(
        money_heavy.budget(&budget),
        BudgetStatus::Warn { used_pct: 90 }
    );

    let mut token_heavy = Telemetry::new(SESSION)?;
    token_heavy.record(&request(10_000, 1, 0, 10));
    assert_eq!(
        token_heavy.budget_report(&budget),
        BudgetReport {
            usd: BudgetStatus::Ok,
            tokens: BudgetStatus::Exceeded
        }
    );
    assert_eq!(token_heavy.budget(&budget), BudgetStatus::Exceeded);
    Ok(())
}

#[test]
fn tokens_are_prompt_plus_completion_without_double_counting_cache() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record(&request(500, 300, 400, 0));
    let budget = Budget::new(None, Some(1_000));
    assert_eq!(
        telemetry.budget(&budget),
        BudgetStatus::Warn { used_pct: 80 }
    );
    Ok(())
}

#[test]
fn custom_warn_percent_is_honoured() -> TestResult {
    let half = usd_budget(1_000).with_warn_pct(50)?;
    assert_eq!(spent(499)?.budget(&half), BudgetStatus::Ok);
    assert_eq!(
        spent(500)?.budget(&half),
        BudgetStatus::Warn { used_pct: 50 }
    );
    assert_eq!(
        usd_budget(1_000).with_warn_pct(0),
        Err(TelemetryError::InvalidWarnPercent(0))
    );
    assert_eq!(
        usd_budget(1_000).with_warn_pct(101),
        Err(TelemetryError::InvalidWarnPercent(101))
    );
    assert!(usd_budget(1_000).with_warn_pct(100).is_ok());
    Ok(())
}

#[test]
fn zero_limit_is_exceeded() -> TestResult {
    let telemetry = Telemetry::new(SESSION)?;
    assert_eq!(telemetry.budget(&usd_budget(0)), BudgetStatus::Exceeded);
    assert_eq!(
        telemetry.budget(&Budget::new(None, Some(0))),
        BudgetStatus::Exceeded
    );
    Ok(())
}

#[test]
fn snapshot_budget_matches_live_budget() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record(&request(500, 300, 400, 900));
    let budget = Budget::new(Some(MicroUsd::new(1_000)), Some(1_000));
    let snapshot = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(
        snapshot.budget_report(&budget),
        telemetry.budget_report(&budget)
    );
    assert_eq!(
        snapshot.budget_report(&budget),
        BudgetReport {
            usd: BudgetStatus::Warn { used_pct: 90 },
            tokens: BudgetStatus::Warn { used_pct: 80 }
        }
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_status_is_monotonic_in_usage(
        limit in 1u64..=1_000_000_000,
        warn in 1u8..=100,
        a in 0u64..=2_000_000_000,
        b in 0u64..=2_000_000_000,
    ) {
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        let budget = usd_budget(limit)
            .with_warn_pct(warn)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let status_low = budget.check(MicroUsd::new(low), 0).usd;
        let status_high = budget.check(MicroUsd::new(high), 0).usd;
        prop_assert!(status_low <= status_high);
        if let BudgetStatus::Warn { used_pct } = status_high {
            prop_assert!(used_pct < 100);
            prop_assert!(used_pct >= warn);
        }
        prop_assert_eq!(status_high == BudgetStatus::Exceeded, high >= limit);
        prop_assert_eq!(budget.check(MicroUsd::new(limit), 0).usd, BudgetStatus::Exceeded);
        prop_assert_ne!(budget.check(MicroUsd::new(limit - 1), 0).usd, BudgetStatus::Exceeded);
    }
}
