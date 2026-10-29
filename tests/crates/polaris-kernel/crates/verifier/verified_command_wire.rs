//! Verified-command wire contract relocated intact from polaris-proto. Only the SDK/verifier
//! boundary may construct ApprovalWitness; the witness-policy gate is unchanged.
use polaris_proto::pb;
use polaris_sdk::contracts::{
    ApprovalWitness, ApproverId, Command, CommandBody, CommandId, Timestamp, TokenBytes,
    Unverified, Verified,
};
use polaris_sdk::ids::{Cid, FrameId};
use prost::Message;
use std::error::Error;
type TestResult = Result<(), Box<dyn Error>>;
fn cid(n: u8) -> Cid {
    Cid::from_bytes([n; 32])
}
fn command(body: CommandBody) -> Command<Unverified> {
    Command::new(
        CommandId::from_u128(0xAAAA_BBBB),
        FrameId::from_u128(0xCCCC_DDDD),
        TokenBytes::from(vec![1, 2, 3, 4]),
        body,
    )
}
#[test]
fn verified_command_serialises_without_witness() -> TestResult {
    let unverified = command(CommandBody::ProposeDiff { diff: cid(9) });
    let witness = ApprovalWitness::new(
        ApproverId::new("polaris-kernel-verifier")?,
        cid(1),
        Timestamp::new(1_700_000_000, 0)?,
    );
    let verified: Command<Verified> = unverified.clone().verify(witness);
    let from_verified = pb::Command::from(&verified).encode_to_vec();
    let from_unverified = pb::Command::from(&unverified).encode_to_vec();
    assert_eq!(from_verified, from_unverified);
    let decoded = Command::<Unverified>::try_from(pb::Command::decode(from_verified.as_slice())?)?;
    // The decoded value has the Unverified state: it is the unverified twin, not a verified command.
    let same_state: Command<Unverified> = decoded;
    assert_eq!(same_state, unverified);
    Ok(())
}
