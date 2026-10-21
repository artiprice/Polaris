//! Event Conversion (descriptor: specs/descriptors/proto_event_conversion.json).

use std::collections::BTreeSet;
use std::error::Error;

use polaris_proto::{ProtoError, pb};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::error::ParseError;
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::strategy::Union;
use prost::Message;

type TestResult = Result<(), Box<dyn Error>>;

fn cid(n: u8) -> Cid {
    Cid::from_bytes([n; 32])
}

fn frame(n: u128) -> FrameId {
    FrameId::from_u128(n)
}

/// Position of a variant; the exhaustive match makes this fail to compile when the sdk adds one.
const fn variant_index(event: &Event) -> usize {
    match event {
        Event::SessionStarted { .. } => 0,
        Event::DirectiveApplied { .. } => 1,
        Event::DecisionMade { .. } => 2,
        Event::ModelRequestCompleted { .. } => 3,
        Event::ToolExecutionStarted { .. } => 4,
        Event::ToolExecutionCompleted { .. } => 5,
        Event::DiffApproved { .. } => 6,
        Event::DiffRejected { .. } => 7,
        Event::FileLeaseGranted { .. } => 8,
        Event::FileLeaseRevoked { .. } => 9,
        Event::SandboxSnapshotBranchCreated { .. } => 10,
        Event::SandboxExecutionCompleted { .. } => 11,
        Event::ProofOfDonePassed { .. } => 12,
        Event::ProofOfDoneBlocked { .. } => 13,
        Event::WorkerRestarted { .. } => 14,
        Event::CircuitTripped { .. } => 15,
        Event::DeadLettered { .. } => 16,
        Event::SessionClosed => 17,
        Event::SessionConfigured { .. } => 18,
        Event::SessionPaused => 19,
        Event::SessionResumed => 20,
    }
}

const VARIANT_COUNT: usize = 21;

#[test]
fn old_envelope_version_is_preserved_and_new_variants_require_v2() -> TestResult {
    let old = EventRecord::from_versioned(
        1,
        Seq::new(5),
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    )?;
    let decoded = EventRecord::try_from(pb::EventRecord::from(&old))?;
    assert_eq!(
        decoded, old,
        "wire decode must retain v1, not silently upgrade it"
    );
    for event in [
        Event::SessionConfigured {
            configuration: cid(1),
        },
        Event::SessionPaused,
        Event::SessionResumed,
    ] {
        let record = EventRecord::new(
            Seq::new(5),
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            event,
        );
        let mut wire = pb::EventRecord::from(&record);
        wire.schema = 1;
        assert!(
            EventRecord::try_from(wire).is_err(),
            "new lifecycle payload mislabeled v1 must be refused"
        );
    }
    Ok(())
}

fn samples() -> Vec<Event> {
    vec![
        Event::SessionStarted { workspace: cid(1) },
        Event::DirectiveApplied { directive: cid(2) },
        Event::DecisionMade {
            frame: frame(3),
            decision: cid(3),
            latency_us: u64::MAX,
        },
        Event::ModelRequestCompleted {
            frame: frame(4),
            request: cid(4),
            prompt_tokens: TokenCount::new(u32::MAX),
            completion_tokens: TokenCount::new(7),
            cached_tokens: TokenCount::new(0),
            cost: MicroUsd::new(u64::MAX),
            latency_us: 12,
        },
        Event::ToolExecutionStarted {
            frame: frame(5),
            call: cid(5),
        },
        Event::ToolExecutionCompleted {
            frame: frame(6),
            call: cid(6),
            ok: true,
            latency_us: 1,
        },
        Event::ToolExecutionCompleted {
            frame: frame(6),
            call: cid(6),
            ok: false,
            latency_us: 0,
        },
        Event::DiffApproved {
            frame: frame(7),
            diff: cid(7),
        },
        Event::DiffRejected {
            frame: frame(8),
            diff: cid(8),
            reason: cid(9),
        },
        Event::FileLeaseGranted {
            frame: frame(9),
            path: cid(10),
            lease: 99,
        },
        Event::FileLeaseRevoked {
            frame: frame(u128::MAX),
            lease: u64::MAX,
        },
        Event::SandboxSnapshotBranchCreated {
            frame: frame(10),
            parent: cid(11),
            branch: cid(12),
        },
        Event::SandboxExecutionCompleted {
            frame: frame(11),
            exec: cid(13),
            exit_code: i32::MIN,
            latency_us: 5,
        },
        Event::SandboxExecutionCompleted {
            frame: frame(11),
            exec: cid(13),
            exit_code: i32::MAX,
            latency_us: 5,
        },
        Event::ProofOfDonePassed {
            frame: frame(12),
            report: cid(14),
        },
        Event::ProofOfDoneBlocked {
            frame: frame(13),
            report: cid(15),
        },
        Event::WorkerRestarted {
            worker: WorkerKind::S1,
            restarts: 1,
        },
        Event::WorkerRestarted {
            worker: WorkerKind::Ingest,
            restarts: 2,
        },
        Event::WorkerRestarted {
            worker: WorkerKind::Nano,
            restarts: u32::MAX,
        },
        Event::CircuitTripped { breaker: cid(16) },
        Event::DeadLettered { message: cid(17) },
        Event::SessionClosed,
        Event::SessionConfigured {
            configuration: cid(18),
        },
        Event::SessionPaused,
        Event::SessionResumed,
    ]
}

fn event_roundtrip(event: Event) -> Result<Event, Box<dyn Error>> {
    let wire = pb::Event::from(&event).encode_to_vec();
    let decoded = pb::Event::decode(wire.as_slice())?;
    Ok(Event::try_from(decoded)?)
}

fn record_roundtrip(record: EventRecord) -> Result<EventRecord, Box<dyn Error>> {
    let wire = pb::EventRecord::from(&record).encode_to_vec();
    let decoded = pb::EventRecord::decode(wire.as_slice())?;
    Ok(EventRecord::try_from(decoded)?)
}

#[test]
fn every_event_variant_roundtrips() -> TestResult {
    let all = samples();
    let covered: BTreeSet<usize> = all.iter().map(variant_index).collect();
    assert_eq!(covered.len(), VARIANT_COUNT, "a variant has no sample");
    assert_eq!(covered.last().copied(), Some(VARIANT_COUNT - 1));
    for event in all {
        assert_eq!(event_roundtrip(event)?, event, "{event:?}");
    }
    Ok(())
}

#[test]
fn event_record_roundtrips_with_timestamp_edges() -> TestResult {
    let session = SessionId::from_u128(0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
    let stamps = [
        Timestamp::new(0, 0)?,
        Timestamp::new(1_700_000_000, 123_456_789)?,
        Timestamp::new(-1, 999_999_999)?,
        Timestamp::new(-62_135_596_800, 1)?,
        Timestamp::new(0, -1)?,
        Timestamp::MIN,
        Timestamp::MAX,
    ];
    for at in stamps {
        let record = EventRecord::new(Seq::new(u64::MAX), at, session, Event::SessionClosed);
        let back = record_roundtrip(record)?;
        assert_eq!(back, record, "{at}");
        assert_eq!(back.at(), at);
    }
    Ok(())
}

#[test]
fn missing_oneof_body_is_error() {
    assert_eq!(
        Event::try_from(pb::Event { kind: None }),
        Err(ProtoError::MissingField("event.kind"))
    );

    let valid = || {
        let record = EventRecord::new(
            Seq::new(1),
            Timestamp::MIN,
            SessionId::from_u128(1),
            Event::SessionClosed,
        );
        pb::EventRecord::from(&record)
    };
    let mut no_event = valid();
    no_event.event = None;
    assert_eq!(
        EventRecord::try_from(no_event),
        Err(ProtoError::MissingField("event_record.event"))
    );
    let mut no_session = valid();
    no_session.session = None;
    assert_eq!(
        EventRecord::try_from(no_session),
        Err(ProtoError::MissingField("event_record.session"))
    );
    let mut no_at = valid();
    no_at.at = None;
    assert_eq!(
        EventRecord::try_from(no_at),
        Err(ProtoError::MissingField("event_record.at"))
    );
    let mut no_cid = pb::Event::from(&Event::SessionStarted { workspace: cid(1) });
    no_cid.kind = Some(pb::event::Kind::SessionStarted(pb::event::SessionStarted {
        workspace: None,
    }));
    assert!(matches!(
        Event::try_from(no_cid),
        Err(ProtoError::MissingField(_))
    ));
}

#[test]
fn unknown_enum_value_is_error() {
    for bad in [0, 4, 99, -1, i32::MAX] {
        let wire = pb::Event {
            kind: Some(pb::event::Kind::WorkerRestarted(
                pb::event::WorkerRestarted {
                    worker: bad,
                    restarts: 1,
                },
            )),
        };
        assert_eq!(
            Event::try_from(wire),
            Err(ProtoError::UnknownVariant {
                field: "event.worker_restarted.worker",
                value: bad
            })
        );
    }
}

#[test]
fn malformed_cid_and_schema_are_errors() -> TestResult {
    for len in [0usize, 1, 31, 33, 64] {
        let wire = pb::Event {
            kind: Some(pb::event::Kind::SessionStarted(pb::event::SessionStarted {
                workspace: Some(pb::Cid {
                    digest: vec![1; len],
                }),
            })),
        };
        assert!(
            matches!(
                Event::try_from(wire),
                Err(ProtoError::InvalidLength {
                    expected: 32,
                    actual,
                    ..
                }) if actual == len
            ),
            "digest of {len} bytes"
        );
    }

    let record = EventRecord::new(
        Seq::new(1),
        Timestamp::new(5, 0)?,
        SessionId::from_u128(1),
        Event::SessionClosed,
    );
    let mut zero = pb::EventRecord::from(&record);
    zero.schema = 0;
    assert_eq!(
        EventRecord::try_from(zero),
        Err(ProtoError::Invalid(ParseError::UnsupportedSchema(0)))
    );
    let mut future = pb::EventRecord::from(&record);
    future.schema = 3;
    assert_eq!(
        EventRecord::try_from(future),
        Err(ProtoError::Invalid(ParseError::UnsupportedSchema(3)))
    );
    let mut huge = pb::EventRecord::from(&record);
    huge.schema = u32::from(u16::MAX) + 1;
    assert!(matches!(
        EventRecord::try_from(huge),
        Err(ProtoError::OutOfRange { .. })
    ));

    for nanos in [-1, 1_000_000_000, i32::MAX, i32::MIN] {
        let mut bad = pb::EventRecord::from(&record);
        bad.at = Some(prost_types::Timestamp { seconds: 0, nanos });
        assert!(
            matches!(
                EventRecord::try_from(bad),
                Err(ProtoError::OutOfRange { .. })
            ),
            "nanos {nanos}"
        );
    }
    for seconds in [i64::MAX, i64::MIN, Timestamp::MAX.as_second() + 1] {
        let mut bad = pb::EventRecord::from(&record);
        bad.at = Some(prost_types::Timestamp { seconds, nanos: 0 });
        assert!(
            matches!(
                EventRecord::try_from(bad),
                Err(ProtoError::OutOfRange { .. })
            ),
            "seconds {seconds}"
        );
    }
    Ok(())
}

#[test]
fn conversion_never_panics_on_default_message() {
    assert!(Event::try_from(pb::Event::default()).is_err());
    assert!(EventRecord::try_from(pb::EventRecord::default()).is_err());
    assert!(Event::try_from(pb::Event::decode(&[][..]).unwrap_or_default()).is_err());
}

fn arb_cid() -> impl Strategy<Value = Cid> {
    any::<[u8; 32]>().prop_map(Cid::from_bytes)
}

fn arb_frame() -> impl Strategy<Value = FrameId> {
    any::<u128>().prop_map(FrameId::from_u128)
}

fn arb_worker() -> impl Strategy<Value = WorkerKind> {
    prop_oneof![
        Just(WorkerKind::S1),
        Just(WorkerKind::Ingest),
        Just(WorkerKind::Nano)
    ]
}

fn arb_event() -> impl Strategy<Value = Event> {
    let arms: Vec<BoxedStrategy<Event>> = vec![
        arb_cid()
            .prop_map(|workspace| Event::SessionStarted { workspace })
            .boxed(),
        arb_cid()
            .prop_map(|directive| Event::DirectiveApplied { directive })
            .boxed(),
        (arb_frame(), arb_cid(), any::<u64>())
            .prop_map(|(frame, decision, latency_us)| Event::DecisionMade {
                frame,
                decision,
                latency_us,
            })
            .boxed(),
        (
            (arb_frame(), arb_cid()),
            (any::<u32>(), any::<u32>(), any::<u32>()),
            (any::<u64>(), any::<u64>()),
        )
            .prop_map(
                |((frame, request), (prompt, completion, cached), (cost, latency_us))| {
                    Event::ModelRequestCompleted {
                        frame,
                        request,
                        prompt_tokens: TokenCount::new(prompt),
                        completion_tokens: TokenCount::new(completion),
                        cached_tokens: TokenCount::new(cached),
                        cost: MicroUsd::new(cost),
                        latency_us,
                    }
                },
            )
            .boxed(),
        (arb_frame(), arb_cid())
            .prop_map(|(frame, call)| Event::ToolExecutionStarted { frame, call })
            .boxed(),
        (arb_frame(), arb_cid(), any::<bool>(), any::<u64>())
            .prop_map(
                |(frame, call, ok, latency_us)| Event::ToolExecutionCompleted {
                    frame,
                    call,
                    ok,
                    latency_us,
                },
            )
            .boxed(),
        (arb_frame(), arb_cid())
            .prop_map(|(frame, diff)| Event::DiffApproved { frame, diff })
            .boxed(),
        (arb_frame(), arb_cid(), arb_cid())
            .prop_map(|(frame, diff, reason)| Event::DiffRejected {
                frame,
                diff,
                reason,
            })
            .boxed(),
        (arb_frame(), arb_cid(), any::<u64>())
            .prop_map(|(frame, path, lease)| Event::FileLeaseGranted { frame, path, lease })
            .boxed(),
        (arb_frame(), any::<u64>())
            .prop_map(|(frame, lease)| Event::FileLeaseRevoked { frame, lease })
            .boxed(),
        (arb_frame(), arb_cid(), arb_cid())
            .prop_map(
                |(frame, parent, branch)| Event::SandboxSnapshotBranchCreated {
                    frame,
                    parent,
                    branch,
                },
            )
            .boxed(),
        (arb_frame(), arb_cid(), any::<i32>(), any::<u64>())
            .prop_map(
                |(frame, exec, exit_code, latency_us)| Event::SandboxExecutionCompleted {
                    frame,
                    exec,
                    exit_code,
                    latency_us,
                },
            )
            .boxed(),
        (arb_frame(), arb_cid())
            .prop_map(|(frame, report)| Event::ProofOfDonePassed { frame, report })
            .boxed(),
        (arb_frame(), arb_cid())
            .prop_map(|(frame, report)| Event::ProofOfDoneBlocked { frame, report })
            .boxed(),
        (arb_worker(), any::<u32>())
            .prop_map(|(worker, restarts)| Event::WorkerRestarted { worker, restarts })
            .boxed(),
        arb_cid()
            .prop_map(|breaker| Event::CircuitTripped { breaker })
            .boxed(),
        arb_cid()
            .prop_map(|message| Event::DeadLettered { message })
            .boxed(),
        Just(Event::SessionClosed).boxed(),
        arb_cid()
            .prop_map(|configuration| Event::SessionConfigured { configuration })
            .boxed(),
        Just(Event::SessionPaused).boxed(),
        Just(Event::SessionResumed).boxed(),
    ];
    assert_eq!(arms.len(), VARIANT_COUNT);
    Union::new(arms)
}

proptest! {
    #[test]
    fn prop_event_roundtrip(event in arb_event()) {
        let back = event_roundtrip(event).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, event);
    }

    #[test]
    fn prop_event_record_roundtrip(
        event in arb_event(),
        seq in any::<u64>(),
        session in any::<u128>(),
        secs in Timestamp::MIN.as_second()..=Timestamp::MAX.as_second(),
        nanos in 0i32..1_000_000_000,
    ) {
        let at = Timestamp::new(secs, nanos).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let record = EventRecord::new(Seq::new(seq), at, SessionId::from_u128(session), event);
        let back = record_roundtrip(record).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, record);
    }

    #[test]
    fn prop_garbage_bytes_never_panic(
        raw in vec(any::<u8>(), 0..512),
        event in arb_event(),
        flips in vec((any::<usize>(), any::<u8>()), 0..4),
    ) {
        // Fully random input.
        if let Ok(msg) = pb::Event::decode(raw.as_slice()) {
            if let Ok(parsed) = Event::try_from(msg) {
                let back = Event::try_from(pb::Event::from(&parsed))
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(back, parsed);
            }
        }
        if let Ok(msg) = pb::EventRecord::decode(raw.as_slice()) {
            if let Ok(parsed) = EventRecord::try_from(msg) {
                let back = EventRecord::try_from(pb::EventRecord::from(&parsed))
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(back, parsed);
            }
        }

        // A valid message with a few corrupted bytes reaches deeper into the converters.
        let mut bytes = pb::Event::from(&event).encode_to_vec();
        for (pos, value) in flips {
            let len = bytes.len();
            if let Some(slot) = bytes.get_mut(pos % len.max(1)) {
                *slot = value;
            }
        }
        if let Ok(msg) = pb::Event::decode(bytes.as_slice()) {
            if let Ok(parsed) = Event::try_from(msg) {
                let back = Event::try_from(pb::Event::from(&parsed))
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                prop_assert_eq!(back, parsed);
            }
        }
    }
}
