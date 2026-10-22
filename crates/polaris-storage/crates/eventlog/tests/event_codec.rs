//! Event Codec (descriptor: specs/descriptors/eventlog_event_codec.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};
use polaris_storage_eventlog::EventLogError;
use polaris_storage_eventlog::codec::{SUPPORTED_SCHEMA, decode_record, encode_record};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn events() -> Vec<Event> {
    let frame = FrameId::from_u128(3);
    let c = |b: u8| Cid::from_bytes([b; 32]);
    vec![
        Event::SessionStarted { workspace: c(1) },
        Event::DirectiveApplied { directive: c(2) },
        Event::DecisionMade {
            frame,
            decision: c(3),
            latency_us: 1,
        },
        Event::ModelRequestCompleted {
            frame,
            request: c(4),
            prompt_tokens: TokenCount::new(1),
            completion_tokens: TokenCount::new(2),
            cached_tokens: TokenCount::new(3),
            cost: MicroUsd::new(4),
            latency_us: 5,
        },
        Event::ToolExecutionStarted { frame, call: c(5) },
        Event::ToolExecutionCompleted {
            frame,
            call: c(5),
            ok: false,
            latency_us: 6,
        },
        Event::DiffApproved { frame, diff: c(6) },
        Event::DiffRejected {
            frame,
            diff: c(6),
            reason: c(7),
        },
        Event::FileLeaseGranted {
            frame,
            path: c(8),
            lease: 1,
        },
        Event::FileLeaseRevoked { frame, lease: 1 },
        Event::SandboxSnapshotBranchCreated {
            frame,
            parent: c(9),
            branch: c(10),
        },
        Event::SandboxExecutionCompleted {
            frame,
            exec: c(11),
            exit_code: -1,
            latency_us: 7,
        },
        Event::ProofOfDonePassed {
            frame,
            report: c(12),
        },
        Event::ProofOfDoneBlocked {
            frame,
            report: c(13),
        },
        Event::WorkerRestarted {
            worker: WorkerKind::Ingest,
            restarts: 3,
        },
        Event::CircuitTripped { breaker: c(14) },
        Event::DeadLettered { message: c(15) },
        Event::SessionClosed,
        Event::SessionConfigured {
            configuration: c(16),
        },
        Event::SessionPaused,
        Event::SessionResumed,
    ]
}

#[test]
fn roundtrip_every_event_variant() -> TestResult {
    for event in events() {
        let rec = EventRecord::new(
            Seq::new(42),
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            event,
        );
        let bytes = encode_record(&rec)?;
        assert_eq!(bytes.get(..2), Some(&SUPPORTED_SCHEMA.to_le_bytes()[..]));
        assert_eq!(decode_record(&bytes)?, rec);
    }
    Ok(())
}

#[test]
fn decode_rejects_short_and_garbage_input() {
    for bad in [
        &b""[..],
        &b"\x01"[..],
        &b"\x01\x00not json"[..],
        &b"\x01\x00{}"[..],
    ] {
        assert!(
            matches!(decode_record(bad), Err(EventLogError::Codec(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn decode_rejects_newer_schema_before_parsing() {
    let mut future = 7u16.to_le_bytes().to_vec();
    future.extend_from_slice(b"anything");
    assert!(matches!(
        decode_record(&future),
        Err(EventLogError::SchemaTooNew {
            found: 7,
            supported: 3
        })
    ));
}

#[test]
fn version_one_prefix_is_preserved_on_encode() -> TestResult {
    let old = EventRecord::from_versioned(
        1,
        Seq::new(1),
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    )?;
    let bytes = encode_record(&old)?;
    assert_eq!(
        bytes.get(..2),
        Some(1u16.to_le_bytes().as_slice()),
        "old envelope keeps old header"
    );
    assert_eq!(decode_record(&bytes)?, old, "old record remains readable");
    Ok(())
}

#[test]
fn mismatched_prefix_and_envelope_are_rejected() -> TestResult {
    let record = EventRecord::new(
        Seq::new(1),
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionPaused,
    );
    let mut bytes = encode_record(&record)?;
    for (slot, byte) in bytes.iter_mut().take(2).zip(1u16.to_le_bytes()) {
        *slot = byte;
    }
    assert!(
        decode_record(&bytes).is_err(),
        "v2 payload cannot hide behind v1 storage prefix"
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode_record(&bytes);
    }
}
