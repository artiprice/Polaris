//! Aggregate Merge (descriptor: specs/descriptors/telemetry_aggregate_merge.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, ModelId, SessionId, TokenCount};
use polaris_telemetry::{Aggregates, TelemetrySnapshot};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn request(prompt: u32, cached: u32, cost: u64, latency_us: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: FrameId::from_u128(1),
        request: Cid::from_bytes([1; 32]),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(3),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(cost),
        latency_us,
    }
}

fn decision(latency_us: u64) -> Event {
    Event::DecisionMade {
        frame: FrameId::from_u128(1),
        decision: Cid::from_bytes([2; 32]),
        latency_us,
    }
}

fn tool_done(ok: bool, latency_us: u64) -> Event {
    Event::ToolExecutionCompleted {
        frame: FrameId::from_u128(1),
        call: Cid::from_bytes([3; 32]),
        ok,
        latency_us,
    }
}

/// Folds `events`; an event with an even index is attributed to `model-even`, odd to `model-odd`.
fn fold(events: &[Event], first_index: usize) -> Result<Aggregates, Box<dyn Error>> {
    let even = ModelId::new("model-even")?;
    let odd = ModelId::new("model-odd")?;
    let mut aggregates = Aggregates::new()?;
    for (offset, event) in events.iter().enumerate() {
        let model = if (first_index + offset) % 2 == 0 {
            &even
        } else {
            &odd
        };
        aggregates.record(Some(model), event);
    }
    Ok(aggregates)
}

fn snap(aggregates: &Aggregates) -> TelemetrySnapshot {
    aggregates.snapshot(SESSION, Timestamp::UNIX_EPOCH)
}

fn sample_events() -> Vec<Event> {
    vec![
        request(100, 0, 500, 1_200),
        decision(150),
        tool_done(false, 40),
        request(60, 20, 250, 900),
        decision(350),
        request(10, 0, 5, 77),
    ]
}

#[test]
fn merge_equals_aggregating_the_concatenation() -> TestResult {
    let events = sample_events();
    let (left, right) = events.split_at(2);
    let mut merged = fold(left, 0)?;
    merged.merge(&fold(right, 2)?)?;
    let whole = fold(&events, 0)?;
    assert_eq!(snap(&merged), snap(&whole));
    assert_eq!(snap(&merged).usage.requests, 3);
    Ok(())
}

#[test]
fn merge_with_empty_is_identity() -> TestResult {
    let events = sample_events();
    let full = fold(&events, 0)?;

    let mut with_empty_right = fold(&events, 0)?;
    with_empty_right.merge(&Aggregates::new()?)?;
    assert_eq!(snap(&with_empty_right), snap(&full));

    let mut with_empty_left = Aggregates::new()?;
    with_empty_left.merge(&full)?;
    assert_eq!(snap(&with_empty_left), snap(&full));
    Ok(())
}

#[test]
fn merge_saturates_and_flags() -> TestResult {
    let mut a = fold(&[request(1, 0, u64::MAX, u64::MAX - 1)], 0)?;
    let b = fold(&[request(1, 0, 10, 10)], 0)?;
    assert!(!snap(&a).saturated);
    a.merge(&b)?;
    let s = snap(&a);
    assert_eq!(s.cost.total, MicroUsd::new(u64::MAX));
    assert_eq!(s.s2.total_us, u64::MAX);
    assert_eq!(s.usage.requests, 2);
    assert!(s.saturated);
    Ok(())
}

#[test]
fn merge_combines_per_model_tables() -> TestResult {
    let only_a = ModelId::new("only-a")?;
    let shared = ModelId::new("shared")?;
    let only_b = ModelId::new("only-b")?;
    let mut a = Aggregates::new()?;
    a.record(Some(&only_a), &request(1, 0, 1, 1));
    a.record(Some(&shared), &request(10, 5, 10, 1));
    let mut b = Aggregates::new()?;
    b.record(Some(&shared), &request(20, 0, 20, 1));
    b.record(Some(&only_b), &request(2, 0, 2, 1));
    a.merge(&b)?;

    let s = snap(&a);
    let names: Vec<&str> = s.models.iter().map(|m| m.model.as_str()).collect();
    assert_eq!(names, ["only-a", "only-b", "shared"]);
    let shared_usage = s
        .models
        .iter()
        .find(|m| m.model == shared)
        .ok_or("shared model missing")?;
    assert_eq!(shared_usage.requests, 2);
    assert_eq!(shared_usage.prompt_tokens, 30);
    assert_eq!(shared_usage.cached_tokens, Some(5));
    assert_eq!(shared_usage.cost, MicroUsd::new(30));
    Ok(())
}

fn event_strategy() -> impl Strategy<Value = Event> {
    (
        0u8..3,
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        any::<bool>(),
    )
        .prop_map(|(kind, a, b, cost, latency, ok)| match kind {
            0 => request(a, b, u64::from(cost), u64::from(latency)),
            1 => decision(u64::from(latency)),
            _ => tool_done(ok, u64::from(latency)),
        })
}

fn fail(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

proptest! {
    #[test]
    fn prop_merge_equals_aggregate_of_concatenation(
        events in proptest::collection::vec(event_strategy(), 0..48),
        split in any::<prop::sample::Index>(),
    ) {
        let at = split.index(events.len() + 1);
        let (left, right) = events.split_at(at);
        let mut merged = fold(left, 0).map_err(fail)?;
        merged.merge(&fold(right, at).map_err(fail)?).map_err(fail)?;
        let whole = fold(&events, 0).map_err(fail)?;
        prop_assert_eq!(snap(&merged), snap(&whole));
    }

    #[test]
    fn prop_merge_is_commutative(
        left in proptest::collection::vec(event_strategy(), 0..32),
        right in proptest::collection::vec(event_strategy(), 0..32),
    ) {
        let mut ab = fold(&left, 0).map_err(fail)?;
        ab.merge(&fold(&right, 0).map_err(fail)?).map_err(fail)?;
        let mut ba = fold(&right, 0).map_err(fail)?;
        ba.merge(&fold(&left, 0).map_err(fail)?).map_err(fail)?;
        prop_assert_eq!(snap(&ab), snap(&ba));
    }
}
