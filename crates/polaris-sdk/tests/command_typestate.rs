//! Command Typestate (descriptor: specs/descriptors/sdk_command_typestate.json).
//! `handler_requires_verified_command` and `verified_command_cannot_be_deserialized` are
//! compile_fail doctests on `polaris_sdk::contracts::Command`.

use std::error::Error;

use polaris_sdk::contracts::{
    ApprovalWitness, ApproverId, Command, CommandBody, CommandId, Timestamp, TokenBytes,
    Unverified, Verified,
};
use polaris_sdk::ids::{Cid, FrameId};

type TestResult = Result<(), Box<dyn Error>>;

const SECRET: &[u8] = b"biscuit-secret-token-bytes";

fn command() -> Command<Unverified> {
    Command::new(
        CommandId::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0001),
        FrameId::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0002),
        TokenBytes::from(SECRET.to_vec()),
        CommandBody::ProposeDiff {
            diff: Cid::from_bytes([7; 32]),
        },
    )
}

fn executes(cmd: &Command<Verified>) -> CommandId {
    cmd.id()
}

#[test]
fn verified_requires_approval_witness() -> TestResult {
    let at: Timestamp = "2026-10-20T12:00:00Z".parse()?;
    let witness = ApprovalWitness::new(
        ApproverId::new("polaris-kernel-verifier")?,
        Cid::from_bytes([9; 32]),
        at,
    );
    let unverified = command();
    let id = unverified.id();
    let verified = unverified.verify(witness);
    assert_eq!(executes(&verified), id);
    assert_eq!(
        verified.witness().approver().as_str(),
        "polaris-kernel-verifier"
    );
    assert_eq!(verified.witness().evidence(), Cid::from_bytes([9; 32]));
    assert_eq!(verified.witness().at(), at);
    assert!(matches!(verified.body(), CommandBody::ProposeDiff { .. }));
    assert!(ApproverId::new("").is_err());
    assert!(ApproverId::new("bad\nname").is_err());
    Ok(())
}

#[test]
fn command_serde_roundtrip() -> TestResult {
    let cmd = command();
    let json = serde_json::to_string(&cmd)?;
    let back: Command<Unverified> = serde_json::from_str(&json)?;
    assert_eq!(back, cmd);
    assert_eq!(back.token().as_bytes(), SECRET);
    Ok(())
}

#[test]
fn token_bytes_debug_is_redacted() {
    let token = TokenBytes::from(SECRET.to_vec());
    let shown = format!("{token:?} {:?}", command());
    assert!(!shown.contains("biscuit"), "{shown}");
    assert!(
        !shown.contains("98, 105, 115"),
        "decimal byte list leaked: {shown}"
    );
    assert!(!shown.contains("626973"), "hex bytes leaked: {shown}");
    assert!(shown.contains("redacted"), "{shown}");
}
