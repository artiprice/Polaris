use polaris_kernel_verifier::{EvidenceError, EvidenceStore, LocalExecutionVerifier};
use polaris_sdk::{contracts::{Command,CommandBody}, ids::{Cid,CommandId,FrameId,SessionId}};
use std::{collections::BTreeMap,sync::Mutex,time::Duration};
#[derive(Default)] struct Store(Mutex<BTreeMap<Cid,Vec<u8>>>);
impl EvidenceStore for Store {
    fn put(&self, bytes:&[u8])->Result<Cid,EvidenceError>{let cid=Cid::from_bytes(*blake3::hash(bytes).as_bytes());self.0.lock().expect("store").insert(cid,bytes.to_vec());Ok(cid)}
    fn get(&self,cid:&Cid)->Result<Option<Vec<u8>>,EvidenceError>{Ok(self.0.lock().expect("store").get(cid).cloned())}
}
#[test] fn execution_approval_is_bound_to_exact_call() {
    let frame=FrameId::from_u128(1);let call=Cid::from_bytes([1;32]);
    let verifier=LocalExecutionVerifier::new(SessionId::from_u128(1),frame).expect("authority");let store=Store::default();
    assert!(verifier.credential(call,Cid::from_bytes([2;32]),Duration::from_secs(30)).is_err());
    let token=verifier.credential(call,call,Duration::from_secs(30)).expect("explicit approval");
    let command=Command::new(CommandId::from_u128(1),frame,token,CommandBody::ExecuteTool{call});
    assert!(verifier.verify(command,&store).is_ok());
}
#[test] fn other_frames_commands_and_authorities_are_rejected() {
    let frame=FrameId::from_u128(1);let call=Cid::from_bytes([1;32]);let session=SessionId::from_u128(1);
    let verifier=LocalExecutionVerifier::new(session,frame).expect("authority");let other=LocalExecutionVerifier::new(session,frame).expect("other authority");let store=Store::default();
    let token=verifier.credential(call,call,Duration::from_secs(30)).expect("approval");
    for (frame,body) in [(FrameId::from_u128(2),CommandBody::ExecuteTool{call}),(frame,CommandBody::ExecuteTool{call:Cid::from_bytes([2;32])}),(frame,CommandBody::ApplyDirective{directive:call})] {
        assert!(verifier.verify(Command::new(CommandId::new_v7(),frame,token.clone(),body),&store).is_err());
    }
    assert!(other.verify(Command::new(CommandId::new_v7(),frame,token,CommandBody::ExecuteTool{call}),&store).is_err());
}
