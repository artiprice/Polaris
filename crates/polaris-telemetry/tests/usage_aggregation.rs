//! Usage Aggregation (descriptor: specs/descriptors/telemetry_usage_aggregation.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, ModelId, Seq, SessionId, TokenCount};
use polaris_telemetry::{MAX_TRACKABLE_US, Telemetry, TelemetryError, TelemetrySnapshot};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn frame() -> FrameId {
    FrameId::from_u128(1)
}

fn cid(byte: u8) -> Cid {
    Cid::from_bytes([byte; 32])
}

fn request(prompt: u32, completion: u32, cached: u32, cost: u64, latency_us: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: frame(),
        request: cid(1),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(completion),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(cost),
        latency_us,
    }
}

fn decision(latency_us: u64) -> Event {
    Event::DecisionMade {
        frame: frame(),
        decision: cid(2),
        latency_us,
    }
}

fn tool_done(ok: bool, latency_us: u64) -> Event {
    Event::ToolExecutionCompleted {
        frame: frame(),
        call: cid(3),
        ok,
        latency_us,
    }
}

fn sandbox_done(exit_code: i32, latency_us: u64) -> Event {
    Event::SandboxExecutionCompleted {
        frame: frame(),
        exec: cid(4),
        exit_code,
        latency_us,
    }
}

fn record_of(session: SessionId, event: Event) -> EventRecord {
    EventRecord::new(Seq::new(1), Timestamp::UNIX_EPOCH, session, event)
}

fn snapshot_of(events: &[Event]) -> Result<TelemetrySnapshot, TelemetryError> {
    let mut telemetry = Telemetry::new(SESSION)?;
    for event in events {
        telemetry.record(event);
    }
    Ok((*telemetry.publish(Timestamp::UNIX_EPOCH)).clone())
}

#[test]
fn model_request_completed_adds_tokens() -> TestResult {
    let s = snapshot_of(&[
        request(100, 20, 30, 500, 1_200),
        request(50, 5, 0, 250, 800),
    ])?;
    assert_eq!(s.usage.requests, 2);
    assert_eq!(s.usage.prompt_tokens, 150);
    assert_eq!(s.usage.completion_tokens, 25);
    assert_eq!(s.cache.cached_tokens, 30);
    assert_eq!(s.cache.prompt_tokens, 150);
    assert_eq!(s.cost.total, MicroUsd::new(750));
    assert_eq!(s.events_seen, 2);
    assert!(!s.saturated);
    Ok(())
}

#[test]
fn cached_tokens_tracked_only_when_reported() -> TestResult {
    let model = ModelId::new("m-a")?;
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_for_model(&model, &record_of(SESSION, request(100, 10, 0, 1, 1)))?;
    let unreported = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert!(!unreported.cache.reported);
    assert_eq!(unreported.cache.hit_pct(), None);
    assert_eq!(
        unreported.models.first().map(|m| m.cached_tokens),
        Some(None)
    );

    telemetry.record_for_model(&model, &record_of(SESSION, request(100, 10, 40, 1, 1)))?;
    let reported = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert!(reported.cache.reported);
    assert_eq!(reported.cache.cached_tokens, 40);
    assert_eq!(reported.cache.hit_pct(), Some(20));
    assert_eq!(
        reported.models.first().map(|m| m.cached_tokens),
        Some(Some(40))
    );
    Ok(())
}

#[test]
fn s1_and_s2_time_split() -> TestResult {
    let s = snapshot_of(&[
        decision(100),
        decision(200),
        decision(300),
        request(1, 1, 0, 0, 1_000),
        request(1, 1, 0, 0, 3_000),
    ])?;
    assert_eq!(s.s1.count, 3);
    assert_eq!(s.s1.total_us, 600);
    assert_eq!(s.s1.p50_us, 200);
    assert_eq!(s.s1.p99_us, 300);
    assert_eq!(s.s1.max_us, 300);
    assert_eq!(s.s2.count, 2);
    assert_eq!(s.s2.total_us, 4_000);
    assert_eq!(s.s2.max_us, 3_000);
    assert_eq!(s.tools.stage.count, 0);
    assert_eq!(s.sandbox.stage.count, 0);
    Ok(())
}

#[test]
fn counters_saturate_not_wrap() -> TestResult {
    let before = snapshot_of(&[request(1, 1, 0, 5, 5)])?;
    assert!(!before.saturated);

    let s = snapshot_of(&[
        request(1, 1, 0, u64::MAX, u64::MAX),
        request(1, 1, 0, 1, u64::MAX),
        request(1, 1, 0, 1, 1),
    ])?;
    assert_eq!(s.cost.total, MicroUsd::new(u64::MAX));
    assert_eq!(s.s2.total_us, u64::MAX);
    assert_eq!(s.s2.count, 3);
    assert_eq!(s.usage.requests, 3);
    assert!(s.saturated);
    Ok(())
}

#[test]
fn per_model_usage_attributed_only_when_model_given() -> TestResult {
    let a = ModelId::new("a-model")?;
    let b = ModelId::new("b-model")?;
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record(&request(10, 1, 0, 100, 1));
    telemetry.record_for_model(&b, &record_of(SESSION, request(20, 2, 0, 200, 1)))?;
    telemetry.record_for_model(&a, &record_of(SESSION, request(5, 3, 0, 50, 1)))?;
    let s = telemetry.publish(Timestamp::UNIX_EPOCH);

    assert_eq!(s.usage.requests, 3);
    assert_eq!(s.usage.prompt_tokens, 35);
    let names: Vec<&str> = s.models.iter().map(|m| m.model.as_str()).collect();
    assert_eq!(names, ["a-model", "b-model"]);
    let prompts: Vec<u64> = s.models.iter().map(|m| m.prompt_tokens).collect();
    assert_eq!(prompts, [5, 20]);
    let requests: Vec<u64> = s.models.iter().map(|m| m.requests).collect();
    assert_eq!(requests, [1, 1]);
    Ok(())
}

#[test]
fn tool_and_sandbox_failures_counted() -> TestResult {
    let s = snapshot_of(&[
        Event::ToolExecutionStarted {
            frame: frame(),
            call: cid(3),
        },
        tool_done(true, 10),
        tool_done(false, 20),
        tool_done(false, 30),
        sandbox_done(0, 100),
        sandbox_done(1, 200),
        sandbox_done(-1, 300),
        Event::SandboxSnapshotBranchCreated {
            frame: frame(),
            parent: cid(5),
            branch: cid(6),
        },
    ])?;
    assert_eq!(s.tools.stage.count, 3);
    assert_eq!(s.tools.stage.total_us, 60);
    assert_eq!(s.tools.failed, 2);
    assert_eq!(s.sandbox.stage.count, 3);
    assert_eq!(s.sandbox.stage.total_us, 600);
    assert_eq!(s.sandbox.failed, 2);
    assert_eq!(s.sandbox.snapshot_branches, 1);
    Ok(())
}

#[test]
fn outcome_events_counted() -> TestResult {
    let events = [
        Event::SessionStarted { workspace: cid(1) },
        Event::DirectiveApplied { directive: cid(2) },
        decision(1),
        request(1, 1, 0, 0, 1),
        Event::ToolExecutionStarted {
            frame: frame(),
            call: cid(3),
        },
        tool_done(true, 1),
        Event::DiffApproved {
            frame: frame(),
            diff: cid(4),
        },
        Event::DiffRejected {
            frame: frame(),
            diff: cid(4),
            reason: cid(5),
        },
        Event::FileLeaseGranted {
            frame: frame(),
            path: cid(6),
            lease: 1,
        },
        Event::FileLeaseRevoked {
            frame: frame(),
            lease: 1,
        },
        Event::SandboxSnapshotBranchCreated {
            frame: frame(),
            parent: cid(7),
            branch: cid(8),
        },
        sandbox_done(0, 1),
        Event::ProofOfDonePassed {
            frame: frame(),
            report: cid(9),
        },
        Event::ProofOfDoneBlocked {
            frame: frame(),
            report: cid(9),
        },
        Event::WorkerRestarted {
            worker: WorkerKind::S1,
            restarts: 1,
        },
        Event::WorkerRestarted {
            worker: WorkerKind::Nano,
            restarts: 2,
        },
        Event::CircuitTripped { breaker: cid(10) },
        Event::DeadLettered { message: cid(11) },
        Event::SessionClosed,
    ];
    let s = snapshot_of(&events)?;
    assert_eq!(s.events_seen, 19);
    assert_eq!(s.outcomes.diffs_approved, 1);
    assert_eq!(s.outcomes.diffs_rejected, 1);
    assert_eq!(s.outcomes.proof_passed, 1);
    assert_eq!(s.outcomes.proof_blocked, 1);
    assert_eq!(s.outcomes.worker_restarts, 2);
    assert_eq!(s.outcomes.circuits_tripped, 1);
    assert_eq!(s.outcomes.dead_letters, 1);
    Ok(())
}

#[test]
fn latency_beyond_range_is_clamped() -> TestResult {
    let s = snapshot_of(&[decision(u64::MAX), decision(1_000)])?;
    assert_eq!(s.s1.count, 2);
    assert_eq!(s.s1.max_us, u64::MAX);
    assert_eq!(s.s1.total_us, u64::MAX);
    assert!(s.saturated);
    let tolerance = MAX_TRACKABLE_US / 1000;
    assert!(s.s1.p99_us >= MAX_TRACKABLE_US - tolerance);
    assert!(s.s1.p99_us <= MAX_TRACKABLE_US + tolerance);
    assert_eq!(s.s1.p50_us, 1_000);
    Ok(())
}

#[test]
fn record_record_rejects_other_session() -> TestResult {
    let other = SessionId::from_u128(8);
    let mut telemetry = Telemetry::new(SESSION)?;
    let before = telemetry.publish(Timestamp::UNIX_EPOCH);
    let result = telemetry.record_record(&record_of(other, request(9, 9, 9, 9, 9)));
    assert_eq!(
        result,
        Err(TelemetryError::SessionMismatch {
            expected: SESSION,
            found: other
        })
    );
    let after = telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(*before, *after);
    assert!(
        telemetry
            .record_record(&record_of(SESSION, request(1, 1, 0, 1, 1)))
            .is_ok()
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_totals_equal_sum_of_inputs(
        inputs in proptest::collection::vec(
            (any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>()),
            0..64,
        )
    ) {
        let events: Vec<Event> = inputs
            .iter()
            .map(|&(p, c, k, cost, lat)| request(p, c, k, u64::from(cost), u64::from(lat)))
            .collect();
        let s = snapshot_of(&events).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(s.usage.requests, inputs.len() as u64);
        prop_assert_eq!(s.usage.prompt_tokens, inputs.iter().map(|i| u64::from(i.0)).sum::<u64>());
        prop_assert_eq!(s.usage.completion_tokens, inputs.iter().map(|i| u64::from(i.1)).sum::<u64>());
        prop_assert_eq!(s.cache.cached_tokens, inputs.iter().map(|i| u64::from(i.2)).sum::<u64>());
        prop_assert_eq!(s.cost.total.get(), inputs.iter().map(|i| u64::from(i.3)).sum::<u64>());
        prop_assert_eq!(s.s2.count, inputs.len() as u64);
        prop_assert_eq!(s.s2.total_us, inputs.iter().map(|i| u64::from(i.4)).sum::<u64>());
        prop_assert!(!s.saturated);
    }

    #[test]
    fn prop_counters_never_decrease_and_never_panic(
        raw in proptest::collection::vec(
            (0u8..8, any::<u32>(), any::<u32>(), any::<u32>(), any::<u64>(), any::<u64>(), any::<bool>(), any::<i32>()),
            0..48,
        )
    ) {
        let mut telemetry = Telemetry::new(SESSION).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut last = counters(&telemetry.publish(Timestamp::UNIX_EPOCH));
        for (kind, a, b, c, cost, latency, ok, code) in raw {
            let event = match kind {
                0 => request(a, b, c, cost, latency),
                1 => decision(latency),
                2 => tool_done(ok, latency),
                3 => sandbox_done(code, latency),
                4 => Event::DiffApproved { frame: frame(), diff: cid(1) },
                5 => Event::WorkerRestarted { worker: WorkerKind::S1, restarts: a },
                6 => Event::CircuitTripped { breaker: cid(2) },
                _ => Event::SessionClosed,
            };
            telemetry.record(&event);
            let now = counters(&telemetry.publish(Timestamp::UNIX_EPOCH));
            prop_assert!(now.iter().zip(last.iter()).all(|(n, l)| n >= l));
            last = now;
        }
    }
}

fn counters(s: &TelemetrySnapshot) -> Vec<u64> {
    vec![
        s.events_seen,
        s.usage.requests,
        s.usage.prompt_tokens,
        s.usage.completion_tokens,
        s.cache.cached_tokens,
        s.cost.total.get(),
        s.s1.count,
        s.s1.total_us,
        s.s2.count,
        s.s2.total_us,
        s.tools.stage.count,
        s.tools.failed,
        s.sandbox.stage.count,
        s.sandbox.failed,
        s.outcomes.diffs_approved,
        s.outcomes.worker_restarts,
        s.outcomes.circuits_tripped,
    ]
}
