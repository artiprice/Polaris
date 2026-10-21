//! Cost Accounting (descriptor: specs/descriptors/telemetry_cost_accounting.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, ModelId, Seq, SessionId, TokenCount};
use polaris_telemetry::{CostReport, Telemetry};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn request(cost: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: FrameId::from_u128(1),
        request: Cid::from_bytes([1; 32]),
        prompt_tokens: TokenCount::new(10),
        completion_tokens: TokenCount::new(10),
        cached_tokens: TokenCount::new(0),
        cost: MicroUsd::new(cost),
        latency_us: 1,
    }
}

fn record_of(event: Event) -> EventRecord {
    EventRecord::new(Seq::new(1), Timestamp::UNIX_EPOCH, SESSION, event)
}

#[test]
fn total_equals_sum_of_event_costs() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    for cost in [500, 0, 250, 1] {
        telemetry.record(&request(cost));
    }
    let s = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(s.cost.total, MicroUsd::new(751));
    assert_eq!(s.usage.requests, 4);
    Ok(())
}

#[test]
fn per_request_average_rounds_down() {
    assert_eq!(
        CostReport::from_totals(MicroUsd::new(10), 3).per_request_avg,
        MicroUsd::new(3)
    );
    assert_eq!(
        CostReport::from_totals(MicroUsd::new(7), 1).per_request_avg,
        MicroUsd::new(7)
    );
    assert_eq!(
        CostReport::from_totals(MicroUsd::new(u64::MAX), 2).per_request_avg,
        MicroUsd::new(u64::MAX / 2)
    );
    assert_eq!(
        CostReport::from_totals(MicroUsd::new(10), 3).total,
        MicroUsd::new(10)
    );
}

#[test]
fn no_requests_average_is_zero() -> TestResult {
    let report = CostReport::from_totals(MicroUsd::new(0), 0);
    assert_eq!(report.total, MicroUsd::new(0));
    assert_eq!(report.per_request_avg, MicroUsd::new(0));

    let empty = Telemetry::new(SESSION)?.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(empty.cost.per_request_avg, MicroUsd::new(0));
    Ok(())
}

#[test]
fn per_model_cost_sums_to_total_when_all_attributed() -> TestResult {
    let a = ModelId::new("model-a")?;
    let b = ModelId::new("model-b")?;
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_for_model(&a, &record_of(request(100)))?;
    telemetry.record_for_model(&b, &record_of(request(40)))?;
    telemetry.record_for_model(&a, &record_of(request(7)))?;
    let s = telemetry.publish(Timestamp::UNIX_EPOCH);

    let by_model: Vec<(&str, u64)> = s
        .models
        .iter()
        .map(|m| (m.model.as_str(), m.cost.get()))
        .collect();
    assert_eq!(by_model, [("model-a", 107), ("model-b", 40)]);
    assert_eq!(s.cost.total, MicroUsd::new(147));
    Ok(())
}

#[test]
fn cost_overflow_saturates_and_flags() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record(&request(u64::MAX - 1));
    assert!(!telemetry.publish(Timestamp::UNIX_EPOCH).saturated);
    telemetry.record(&request(5));
    let s = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(s.cost.total, MicroUsd::new(u64::MAX));
    assert!(s.saturated);
    assert_eq!(s.cost.per_request_avg, MicroUsd::new(u64::MAX / 2));
    Ok(())
}

proptest! {
    #[test]
    fn prop_cost_equals_sum_of_per_model_costs(
        costs in proptest::collection::vec((any::<u32>(), 0u8..3), 0..64)
    ) {
        let models = [
            ModelId::new("m-0").map_err(|e| TestCaseError::fail(e.to_string()))?,
            ModelId::new("m-1").map_err(|e| TestCaseError::fail(e.to_string()))?,
            ModelId::new("m-2").map_err(|e| TestCaseError::fail(e.to_string()))?,
        ];
        let mut telemetry = Telemetry::new(SESSION).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for &(cost, which) in &costs {
            let model = models.get(usize::from(which)).ok_or_else(|| TestCaseError::fail("model index"))?;
            telemetry
                .record_for_model(model, &record_of(request(u64::from(cost))))
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        let s = telemetry.publish(Timestamp::UNIX_EPOCH);
        let input_sum: u64 = costs.iter().map(|&(c, _)| u64::from(c)).sum();
        let model_sum: u64 = s.models.iter().map(|m| m.cost.get()).sum();
        prop_assert_eq!(s.cost.total.get(), input_sum);
        prop_assert_eq!(model_sum, input_sum);
        let requests = costs.len() as u64;
        let expected_avg = if requests == 0 { 0 } else { input_sum / requests };
        prop_assert_eq!(s.cost.per_request_avg.get(), expected_avg);
    }
}
