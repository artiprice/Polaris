use polaris_kernel_verifier::{EvidenceError, EvidenceStore, FrameCancellationVerifier};
use polaris_sdk::{
    contracts::{Command, CommandBody},
    ids::{Cid, CommandId, FrameId, SessionId},
};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

#[derive(Default)]
struct MemoryEvidence(Mutex<BTreeMap<Cid, Vec<u8>>>);

impl EvidenceStore for MemoryEvidence {
    fn put(&self, bytes: &[u8]) -> Result<Cid, EvidenceError> {
        let cid = Cid::from_bytes(*blake3::hash(bytes).as_bytes());
        let mut records = self.0.lock().map_err(|_| EvidenceError::Backend)?;
        records.insert(cid, bytes.to_vec());
        Ok(cid)
    }
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, EvidenceError> {
        let records = self.0.lock().map_err(|_| EvidenceError::Backend)?;
        Ok(records.get(cid).cloned())
    }
}

#[test]
fn cancellation_authority_is_target_scoped_and_never_execution_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let session = SessionId::new_v7();
    let actor = FrameId::new_v7();
    let target = FrameId::new_v7();
    let other = FrameId::new_v7();
    let verifier = FrameCancellationVerifier::new(session, actor)?;
    let credential = verifier.credential(target, Duration::from_secs(60))?;
    let command = Command::new(
        CommandId::new_v7(),
        actor,
        credential.clone(),
        CommandBody::CancelFrame { frame: target },
    );
    let approved = verifier.verify(command, &MemoryEvidence::default())?;
    assert_eq!(approved.frame(), actor);
    assert_eq!(*approved.body(), CommandBody::CancelFrame { frame: target });

    let wrong_target = Command::new(
        CommandId::new_v7(),
        actor,
        credential,
        CommandBody::CancelFrame { frame: other },
    );
    assert!(
        verifier
            .verify(wrong_target, &MemoryEvidence::default())
            .is_err()
    );

    let token = polaris_security_capability::Token::from_bytes(
        approved.token().as_bytes(),
        &verifier.issuer(),
    )?;
    let execute_resource = polaris_security_capability::ResourcePath::new("tools/execute")?;
    let facts = polaris_security_capability::Facts {
        resource: &execute_resource,
        symbol: None,
        churn_lines: 0,
        touches_unsafe: false,
        diff_blake3: None,
        right: polaris_security_capability::Right::ExecuteTool,
    };
    assert!(matches!(
        polaris_security_capability::authorize(
            &token,
            &facts,
            &verifier.issuer(),
            &polaris_security_capability::SystemClock,
        )?,
        polaris_security_capability::Authorization::Denied { .. }
    ));
    Ok(())
}
