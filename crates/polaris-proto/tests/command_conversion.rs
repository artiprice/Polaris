//! Command Conversion (descriptor: specs/descriptors/proto_command_conversion.json).

use std::collections::BTreeSet;
use std::error::Error;

use polaris_proto::{ProtoError, pb};
use polaris_sdk::contracts::{Command, CommandBody, CommandId, TokenBytes, Unverified};
use polaris_sdk::effort::Effort;
use polaris_sdk::human::HumanRequestId;
use polaris_sdk::ids::{AgentRole, Cid, FrameId, Seq};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::strategy::Union;
use prost::Message;

type TestResult = Result<(), Box<dyn Error>>;

fn cid(n: u8) -> Cid {
    Cid::from_bytes([n; 32])
}

/// Position of a variant; the exhaustive match makes this fail to compile when the sdk adds one.
const fn body_index(body: &CommandBody) -> usize {
    match body {
        CommandBody::SubmitUserInput { .. } => 0,
        CommandBody::ProposeDiff { .. } => 1,
        CommandBody::AcquireAstScope { .. } => 2,
        CommandBody::ReleaseAstScope { .. } => 3,
        CommandBody::SpawnSubagent { .. } => 4,
        CommandBody::ExecuteTool { .. } => 5,
        CommandBody::RollbackTo { .. } => 6,
        CommandBody::CancelFrame { .. } => 7,
        CommandBody::ApplyDirective { .. } => 8,
        CommandBody::AnswerHumanRequest { .. } => 9,
    }
}

const VARIANT_COUNT: usize = 10;

fn bodies() -> Vec<CommandBody> {
    let mut all = vec![
        CommandBody::SubmitUserInput { text: cid(1) },
        CommandBody::ProposeDiff { diff: cid(2) },
        CommandBody::AcquireAstScope {
            path: cid(3),
            symbol: cid(4),
        },
        CommandBody::ReleaseAstScope { lease: u64::MAX },
        CommandBody::ExecuteTool { call: cid(5) },
        CommandBody::RollbackTo {
            seq: Seq::new(u64::MAX),
        },
        CommandBody::CancelFrame {
            frame: FrameId::from_u128(u128::MAX),
        },
        CommandBody::ApplyDirective { directive: cid(6) },
        CommandBody::AnswerHumanRequest {
            request_id: HumanRequestId::from_u128(7),
            revision: cid(8),
            answer: cid(9),
        },
    ];
    for role in [
        AgentRole::Orchestrator,
        AgentRole::SubAgent,
        AgentRole::Verifier,
        AgentRole::Researcher,
        AgentRole::Hacker,
        AgentRole::Watchdog,
    ] {
        for effort in Effort::ALL {
            all.push(CommandBody::SpawnSubagent {
                role,
                effort,
                brief: cid(7),
            });
        }
    }
    all
}

fn command(body: CommandBody) -> Command<Unverified> {
    Command::new(
        CommandId::from_u128(0xAAAA_BBBB),
        FrameId::from_u128(0xCCCC_DDDD),
        TokenBytes::from(vec![1, 2, 3, 4]),
        body,
    )
}

fn roundtrip(cmd: &Command<Unverified>) -> Result<Command<Unverified>, Box<dyn Error>> {
    let wire = pb::Command::from(cmd).encode_to_vec();
    let decoded = pb::Command::decode(wire.as_slice())?;
    Ok(Command::<Unverified>::try_from(decoded)?)
}

#[test]
fn every_command_body_roundtrips() -> TestResult {
    let all = bodies();
    let covered: BTreeSet<usize> = all.iter().map(body_index).collect();
    assert_eq!(covered.len(), VARIANT_COUNT, "a variant has no sample");
    assert_eq!(covered.last().copied(), Some(VARIANT_COUNT - 1));
    for body in all {
        let cmd = command(body);
        assert_eq!(roundtrip(&cmd)?, cmd, "{body:?}");
    }
    Ok(())
}

fn valid_wire() -> pb::Command {
    pb::Command::from(&command(CommandBody::ApplyDirective { directive: cid(1) }))
}

#[test]
fn missing_fields_are_errors() {
    let mut no_id = valid_wire();
    no_id.id = None;
    assert_eq!(
        Command::<Unverified>::try_from(no_id),
        Err(ProtoError::MissingField("command.id"))
    );
    let mut no_frame = valid_wire();
    no_frame.frame = None;
    assert_eq!(
        Command::<Unverified>::try_from(no_frame),
        Err(ProtoError::MissingField("command.frame"))
    );
    let mut no_body = valid_wire();
    no_body.body = None;
    assert_eq!(
        Command::<Unverified>::try_from(no_body),
        Err(ProtoError::MissingField("command.body"))
    );
    let mut no_kind = valid_wire();
    no_kind.body = Some(pb::CommandBody { kind: None });
    assert_eq!(
        Command::<Unverified>::try_from(no_kind),
        Err(ProtoError::MissingField("command_body.kind"))
    );
    let mut no_cid = valid_wire();
    no_cid.body = Some(pb::CommandBody {
        kind: Some(pb::command_body::Kind::ProposeDiff(
            pb::command_body::ProposeDiff { diff: None },
        )),
    });
    assert!(matches!(
        Command::<Unverified>::try_from(no_cid),
        Err(ProtoError::MissingField(_))
    ));
}

fn spawn(role: i32, effort: i32) -> pb::Command {
    let mut wire = valid_wire();
    wire.body = Some(pb::CommandBody {
        kind: Some(pb::command_body::Kind::SpawnSubagent(
            pb::command_body::SpawnSubagent {
                role,
                effort,
                brief: Some(pb::Cid {
                    digest: vec![0; 32],
                }),
            },
        )),
    });
    wire
}

#[test]
fn unknown_role_and_effort_are_errors() {
    assert!(Command::<Unverified>::try_from(spawn(1, 1)).is_ok());
    for bad in [0, 7, 100, -1, i32::MIN] {
        assert_eq!(
            Command::<Unverified>::try_from(spawn(bad, 1)),
            Err(ProtoError::UnknownVariant {
                field: "command_body.spawn_subagent.role",
                value: bad
            })
        );
    }
    for bad in [0, 6, 100, -5, i32::MAX] {
        assert_eq!(
            Command::<Unverified>::try_from(spawn(1, bad)),
            Err(ProtoError::UnknownVariant {
                field: "command_body.spawn_subagent.effort",
                value: bad
            })
        );
    }
}

#[test]
fn conversion_never_panics_on_default_message() {
    assert!(Command::<Unverified>::try_from(pb::Command::default()).is_err());
    assert!(polaris_sdk::contracts::CommandBody::try_from(pb::CommandBody::default()).is_err());
}

fn arb_cid() -> impl Strategy<Value = Cid> {
    any::<[u8; 32]>().prop_map(Cid::from_bytes)
}

fn arb_role() -> impl Strategy<Value = AgentRole> {
    prop_oneof![
        Just(AgentRole::Orchestrator),
        Just(AgentRole::SubAgent),
        Just(AgentRole::Verifier),
        Just(AgentRole::Researcher),
        Just(AgentRole::Hacker),
        Just(AgentRole::Watchdog),
    ]
}

fn arb_effort() -> impl Strategy<Value = Effort> {
    prop::sample::select(Effort::ALL.to_vec())
}

fn arb_body() -> impl Strategy<Value = CommandBody> {
    let arms: Vec<BoxedStrategy<CommandBody>> = vec![
        arb_cid()
            .prop_map(|text| CommandBody::SubmitUserInput { text })
            .boxed(),
        arb_cid()
            .prop_map(|diff| CommandBody::ProposeDiff { diff })
            .boxed(),
        (arb_cid(), arb_cid())
            .prop_map(|(path, symbol)| CommandBody::AcquireAstScope { path, symbol })
            .boxed(),
        any::<u64>()
            .prop_map(|lease| CommandBody::ReleaseAstScope { lease })
            .boxed(),
        (arb_role(), arb_effort(), arb_cid())
            .prop_map(|(role, effort, brief)| CommandBody::SpawnSubagent {
                role,
                effort,
                brief,
            })
            .boxed(),
        arb_cid()
            .prop_map(|call| CommandBody::ExecuteTool { call })
            .boxed(),
        any::<u64>()
            .prop_map(|seq| CommandBody::RollbackTo { seq: Seq::new(seq) })
            .boxed(),
        any::<u128>()
            .prop_map(|frame| CommandBody::CancelFrame {
                frame: FrameId::from_u128(frame),
            })
            .boxed(),
        arb_cid()
            .prop_map(|directive| CommandBody::ApplyDirective { directive })
            .boxed(),
        (any::<u128>(), arb_cid(), arb_cid())
            .prop_map(
                |(request_id, revision, answer)| CommandBody::AnswerHumanRequest {
                    request_id: HumanRequestId::from_u128(request_id),
                    revision,
                    answer,
                },
            )
            .boxed(),
    ];
    assert_eq!(arms.len(), VARIANT_COUNT);
    Union::new(arms)
}

proptest! {
    #[test]
    fn prop_command_roundtrip(
        body in arb_body(),
        id in any::<u128>(),
        frame in any::<u128>(),
        token in vec(any::<u8>(), 0..256),
    ) {
        let cmd = Command::new(
            CommandId::from_u128(id),
            FrameId::from_u128(frame),
            TokenBytes::from(token),
            body,
        );
        let back = roundtrip(&cmd).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, cmd);
    }

    #[test]
    fn prop_garbage_bytes_never_panic(
        raw in vec(any::<u8>(), 0..512),
        body in arb_body(),
        flips in vec((any::<usize>(), any::<u8>()), 0..4),
    ) {
        if let Ok(msg) = pb::Command::decode(raw.as_slice())
            && let Ok(parsed) = Command::<Unverified>::try_from(msg)
        {
            let back = roundtrip(&parsed).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(back, parsed);
        }
        let mut bytes = pb::Command::from(&command(body)).encode_to_vec();
        for (pos, value) in flips {
            let len = bytes.len();
            let position = pos.checked_rem(len.max(1))
                .ok_or_else(|| TestCaseError::fail("nonzero mutation span invariant"))?;
            if let Some(slot) = bytes.get_mut(position) {
                *slot = value;
            }
        }
        if let Ok(msg) = pb::Command::decode(bytes.as_slice())
            && let Ok(parsed) = Command::<Unverified>::try_from(msg)
        {
            let back = roundtrip(&parsed).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(back, parsed);
        }
    }
}
