//! Signed, exact-call approval for the keyless P2 execution profile.
use crate::{EvidenceError, EvidenceStore, LocalVerificationError};
use polaris_sdk::{contracts::{ApprovalWitness, ApproverId, Command, CommandBody, TokenBytes, Unverified, Verified}, ids::{Cid, FrameId, SessionId}};
use polaris_security_capability::{Authority, Authorization, Facts, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock, Token, authorize};
use std::time::Duration;

/// An isolated session/frame execution issuer. Only trusted human-facing composition code may
/// mint credentials; model callers receive the command bus, never this issuer.
#[derive(Debug)]
pub struct LocalExecutionVerifier { authority: Authority, frame: FrameId, resource: ResourcePath }
impl LocalExecutionVerifier {
    /// Create a distinct authority, unrelated to directive and mutation credentials.
    /// # Errors
    /// Entropy or resource construction failed.
    pub fn new(session: SessionId, frame: FrameId) -> Result<Self, LocalVerificationError> {
        Ok(Self {authority: Authority::new_session()?, frame, resource: ResourcePath::new(&format!("sessions/{session}/executions"))?})
    }
    /// Mint short-lived execution authority for the explicitly accepted review digest.
    ///
    /// `accepted_digest` must be supplied by a trusted approval interface after it displays the
    /// immutable review, including tree, sandbox policy, argv, timeout and blast report. Equality
    /// here establishes binding, not the provenance of a human response. The composition owner
    /// must additionally reject stale trees and persist single-use consumption before execution.
    /// # Errors
    /// Digest mismatch, invalid lifetime, entropy or capability encoding failure.
    pub fn credential(&self, call: Cid, accepted_digest: Cid, ttl: Duration) -> Result<TokenBytes, LocalVerificationError> {
        if call != accepted_digest || ttl.is_zero() || ttl > Duration::from_secs(300) { return Err(LocalVerificationError::Rejected); }
        let scope = Scope {resource:ResourceScope::Exact(self.resource.clone()),max_churn_lines:0,ttl,allow_unsafe:false,diff_blake3:Some(call),rights:Rights::new(&[Right::ExecuteTool])};
        Ok(self.authority.issue_root(self.frame,&scope,&SystemClock)?.to_bytes().clone())
    }
    /// Verify signature, frame, right, exact payload digest and durable approval evidence.
    /// # Errors
    /// Unsupported command, invalid capability or failed evidence persistence/read-back.
    pub fn verify(&self, command: Command<Unverified>, evidence: &impl EvidenceStore) -> Result<Command<Verified>, LocalVerificationError> {
        let CommandBody::ExecuteTool {call} = *command.body() else { return Err(LocalVerificationError::Rejected); };
        if command.frame() != self.frame { return Err(LocalVerificationError::Rejected); }
        let token=Token::from_bytes(command.token().as_bytes(),&self.authority.public_key())?;
        let facts=Facts {resource:&self.resource,symbol:None,churn_lines:0,touches_unsafe:false,diff_blake3:Some(call),right:Right::ExecuteTool};
        if authorize(&token,&facts,&self.authority.public_key(),&SystemClock)? != Authorization::Allowed { return Err(LocalVerificationError::Rejected); }
        let bytes=serde_json::to_vec(&("polaris.execution-approval.v1",command.id(),command.frame(),command.body())).map_err(|_|LocalVerificationError::Encoding)?;
        let cid=evidence.put(&bytes)?;
        if cid != Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) || evidence.get(&cid)?.as_deref()!=Some(bytes.as_slice()) {return Err(EvidenceError::DigestMismatch.into());}
        if authorize(&token,&facts,&self.authority.public_key(),&SystemClock)? != Authorization::Allowed {return Err(LocalVerificationError::Rejected);}
        let approver=ApproverId::new("polaris-kernel-verifier").map_err(|_|LocalVerificationError::Encoding)?;
        Ok(command.verify(ApprovalWitness::new(approver,cid,polaris_sdk::contracts::Timestamp::now())))
    }
}
