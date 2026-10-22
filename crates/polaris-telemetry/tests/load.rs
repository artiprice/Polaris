//! Throughput evidence for the P1 event-fold path.

use std::time::Instant;

use polaris_sdk::contracts::{Event, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, SessionId};
use polaris_telemetry::Telemetry;

const SESSION: SessionId = SessionId::from_u128(88);
const EVENT_COUNT: u64 = 1_000_000;

#[test]
fn folds_one_million_decision_events_and_reports_elapsed_time()
-> Result<(), Box<dyn std::error::Error>> {
    let event = Event::DecisionMade {
        frame: FrameId::from_u128(1),
        decision: Cid::from_bytes([5; 32]),
        latency_us: 1_250,
    };
    let mut telemetry = Telemetry::new(SESSION)?;
    let started = Instant::now();
    for _ in 0..EVENT_COUNT {
        telemetry.record(&event);
    }
    let elapsed = started.elapsed();
    let snapshot = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(snapshot.events_seen, EVENT_COUNT);
    assert_eq!(snapshot.s1.count, EVENT_COUNT);
    eprintln!("telemetry load: {EVENT_COUNT} events folded in {elapsed:?}");
    Ok(())
}
