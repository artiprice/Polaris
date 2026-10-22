//! Event Envelope (descriptor: specs/descriptors/sdk_event_envelope.json).

use std::error::Error;

use polaris_sdk::contracts::{EVENT_SCHEMA_VERSION, Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};

type TestResult = Result<(), Box<dyn Error>>;

fn cid(b: u8) -> Cid {
    Cid::from_bytes([b; 32])
}

fn all_events() -> Vec<Event> {
    let frame = FrameId::from_u128(0x0192_0000_0000_7000_8000_0000_0000_00f1);
    vec![
        Event::SessionStarted { workspace: cid(1) },
        Event::DirectiveApplied { directive: cid(2) },
        Event::DecisionMade {
            frame,
            decision: cid(3),
            latency_us: 363_000,
        },
        Event::ModelRequestCompleted {
            frame,
            request: cid(4),
            prompt_tokens: TokenCount::new(1200),
            completion_tokens: TokenCount::new(300),
            cached_tokens: TokenCount::new(1024),
            cost: MicroUsd::new(450),
            latency_us: 2_300_000,
        },
        Event::ToolExecutionStarted {
            frame,
            call: cid(5),
        },
        Event::ToolExecutionCompleted {
            frame,
            call: cid(5),
            ok: true,
            latency_us: 74_000,
        },
        Event::DiffApproved {
            frame,
            diff: cid(6),
        },
        Event::DiffRejected {
            frame,
            diff: cid(6),
            reason: cid(7),
        },
        Event::FileLeaseGranted {
            frame,
            path: cid(8),
            lease: 42,
        },
        Event::FileLeaseRevoked { frame, lease: 42 },
        Event::SandboxSnapshotBranchCreated {
            frame,
            parent: cid(9),
            branch: cid(10),
        },
        Event::SandboxExecutionCompleted {
            frame,
            exec: cid(11),
            exit_code: 7,
            latency_us: 112_000,
        },
        Event::ProofOfDonePassed {
            frame,
            report: cid(12),
        },
        Event::ProofOfDoneBlocked {
            frame,
            report: cid(13),
        },
        Event::WorkerRestarted {
            worker: WorkerKind::S1,
            restarts: 2,
        },
        Event::CircuitTripped { breaker: cid(14) },
        Event::DeadLettered { message: cid(15) },
        Event::SessionClosed,
        Event::SessionConfigured {
            configuration: cid(16),
        },
        Event::SessionPaused,
        Event::SessionResumed,
        Event::BlastRadiusAlert {
            frame,
            report: cid(17),
        },
    ]
}

fn record(event: Event) -> Result<EventRecord, Box<dyn Error>> {
    let at: Timestamp = "2026-10-20T12:00:00Z".parse()?;
    Ok(EventRecord::new(
        Seq::new(7),
        at,
        SessionId::from_u128(0x0192_0000_0000_7000_8000_0000_0000_00a1),
        event,
    ))
}

#[test]
fn event_record_json_roundtrip() -> TestResult {
    for event in all_events() {
        let rec = record(event)?;
        let json = serde_json::to_string(&rec)?;
        let back: EventRecord = serde_json::from_str(&json)?;
        assert_eq!(back, rec, "{json}");
    }
    Ok(())
}

#[test]
fn schema_version_is_current() -> TestResult {
    assert_eq!(EVENT_SCHEMA_VERSION, 3);
    assert_eq!(record(Event::SessionClosed)?.schema(), 3);
    Ok(())
}

#[test]
fn event_is_fixed_size_copy() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<Event>();
    let size = core::mem::size_of::<Event>();
    assert!(size <= 96, "Event is {size} bytes");
}

#[test]
fn event_record_json_snapshot() -> TestResult {
    let frame = FrameId::from_u128(0x0192_0000_0000_7000_8000_0000_0000_00f1);
    let rec = record(Event::ModelRequestCompleted {
        frame,
        request: cid(4),
        prompt_tokens: TokenCount::new(1200),
        completion_tokens: TokenCount::new(300),
        cached_tokens: TokenCount::new(1024),
        cost: MicroUsd::new(450),
        latency_us: 2_300_000,
    })?;
    let legacy_record =
        EventRecord::from_versioned(1, rec.seq(), rec.at(), rec.session(), *rec.event())?;
    insta::assert_json_snapshot!("event_record_v1_model_request_completed", legacy_record);
    Ok(())
}
