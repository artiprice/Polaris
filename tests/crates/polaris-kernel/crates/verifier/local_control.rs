//! Local-control authorization (descriptor: specs/descriptors/kernel_submit_user_input.json).
use polaris_kernel_verifier::{
    EvidenceError, EvidenceStore, LocalDirectiveVerifier, LocalVerificationError,
};
use polaris_sdk::{
    contracts::{Command, CommandBody},
    ids::{Cid, CommandId, FrameId, SessionId},
};
use std::{collections::BTreeMap, error::Error, sync::Mutex, time::Duration};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Default)]
struct MemoryEvidence(Mutex<BTreeMap<Cid, Vec<u8>>>);

impl EvidenceStore for MemoryEvidence {
    fn put(&self, bytes: &[u8]) -> Result<Cid, EvidenceError> {
        let cid = Cid::from_bytes(*blake3::hash(bytes).as_bytes());
        self.0
            .lock()
            .map_err(|_| EvidenceError::Backend)?
            .insert(cid, bytes.to_vec());
        Ok(cid)
    }

    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, EvidenceError> {
        self.0
            .lock()
            .map(|values| values.get(cid).cloned())
            .map_err(|_| EvidenceError::Backend)
    }
}

fn frame() -> FrameId {
    FrameId::from_u128(1)
}

fn verifier() -> Result<LocalDirectiveVerifier, LocalVerificationError> {
    LocalDirectiveVerifier::new(SessionId::from_u128(1), frame())
}

#[test]
fn local_control_accepts_submit_user_input() -> TestResult {
    let verifier = verifier()?;
    let evidence = MemoryEvidence::default();
    let token = verifier.credential(Duration::from_secs(30))?;
    let body = CommandBody::SubmitUserInput {
        text: Cid::from_bytes([4; 32]),
    };
    let verified = verifier.verify(
        Command::new(CommandId::from_u128(1), frame(), token, body),
        &evidence,
    )?;
    assert_eq!(*verified.body(), body, "the verified body is unchanged");
    let stored = evidence
        .get(&verified.witness().evidence())?
        .ok_or("evidence document missing")?;
    assert_eq!(
        Cid::from_bytes(*blake3::hash(&stored).as_bytes()),
        verified.witness().evidence(),
        "evidence digest binds the approved command"
    );
    Ok(())
}

#[test]
fn local_control_rejects_submit_user_input_for_wrong_frame() -> TestResult {
    let verifier = verifier()?;
    let evidence = MemoryEvidence::default();
    let token = verifier.credential(Duration::from_secs(30))?;
    let result = verifier.verify(
        Command::new(
            CommandId::from_u128(1),
            FrameId::from_u128(2),
            token,
            CommandBody::SubmitUserInput {
                text: Cid::from_bytes([4; 32]),
            },
        ),
        &evidence,
    );
    assert!(
        matches!(result, Err(LocalVerificationError::Rejected)),
        "a different frame is rejected"
    );
    Ok(())
}

#[test]
fn local_control_still_rejects_other_bodies() -> TestResult {
    let verifier = verifier()?;
    let evidence = MemoryEvidence::default();
    let token = verifier.credential(Duration::from_secs(30))?;
    for body in [
        CommandBody::ProposeDiff {
            diff: Cid::from_bytes([5; 32]),
        },
        CommandBody::ReleaseAstScope { lease: 3 },
    ] {
        let result = verifier.verify(
            Command::new(CommandId::from_u128(1), frame(), token.clone(), body),
            &evidence,
        );
        assert!(
            matches!(result, Err(LocalVerificationError::Rejected)),
            "{body:?} must stay outside the local control profile"
        );
    }
    Ok(())
}

#[test]
fn local_control_rejects_a_garbled_credential_for_user_input() -> TestResult {
    let verifier = verifier()?;
    let evidence = MemoryEvidence::default();
    let result = verifier.verify(
        Command::new(
            CommandId::from_u128(1),
            frame(),
            polaris_sdk::contracts::TokenBytes::from(vec![1]),
            CommandBody::SubmitUserInput {
                text: Cid::from_bytes([4; 32]),
            },
        ),
        &evidence,
    );
    assert!(result.is_err(), "invalid signatures fail closed");
    Ok(())
}
