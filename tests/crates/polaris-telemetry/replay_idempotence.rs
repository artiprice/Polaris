//! Event-log replay contract (descriptor: specs/descriptors/telemetry_snapshot_publication.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};
use polaris_telemetry::{Telemetry, TelemetryError};

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(71);

fn request(prompt_tokens: u32) -> Event {
    Event::ModelRequestCompleted {
        frame: FrameId::from_u128(1),
        request: Cid::from_bytes([1; 32]),
        prompt_tokens: TokenCount::new(prompt_tokens),
        completion_tokens: TokenCount::new(3),
        cached_tokens: TokenCount::new(0),
        cost: MicroUsd::new(5),
        latency_us: 100,
    }
}

fn record(sequence: u64, event: Event) -> EventRecord {
    EventRecord::new(Seq::new(sequence), Timestamp::UNIX_EPOCH, SESSION, event)
}

#[test]
fn replaying_the_latest_record_is_idempotent() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    let first = record(1, request(10));
    telemetry.record_replay_record(&first)?;
    telemetry.record_replay_record(&first)?;

    let snapshot = telemetry
        .aggregates()
        .snapshot(SESSION, Timestamp::UNIX_EPOCH);
    assert_eq!(snapshot.events_seen, 1);
    assert_eq!(snapshot.usage.requests, 1);
    assert_eq!(snapshot.usage.prompt_tokens, 10);
    Ok(())
}

#[test]
fn replay_sequence_conflicts_and_regressions_are_rejected_without_folding() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    let first = record(4, request(10));
    telemetry.record_replay_record(&first)?;

    match telemetry.record_replay_record(&record(4, request(11))) {
        Err(TelemetryError::ConflictingSequence(sequence)) => {
            assert_eq!(sequence, Seq::new(4));
        }
        other => return Err(format!("expected conflicting sequence, got {other:?}").into()),
    }
    match telemetry.record_replay_record(&record(3, request(12))) {
        Err(TelemetryError::OutOfOrder { previous, found }) => {
            assert_eq!(previous, Seq::new(4));
            assert_eq!(found, Seq::new(3));
        }
        other => return Err(format!("expected out-of-order error, got {other:?}").into()),
    }

    let snapshot = telemetry
        .aggregates()
        .snapshot(SESSION, Timestamp::UNIX_EPOCH);
    assert_eq!(snapshot.events_seen, 1);
    assert_eq!(snapshot.usage.prompt_tokens, 10);
    Ok(())
}
