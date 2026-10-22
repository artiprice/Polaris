//! Signed authorization for P1 local session controls; mutation proposals use the hybrid path.
use crate::{EvidenceError, EvidenceStore};
use polaris_sdk::{
    contracts::{ApprovalWitness, ApproverId, Command, CommandBody, Unverified, Verified},
    ids::{Cid, FrameId, SessionId},
};
use polaris_security_capability::{
    Authority, Authorization, Facts, ResourcePath, ResourceScope, Right, Rights, Scope,
    SystemClock, Token, authorize,
};
use std::time::Duration;

/// Fail-closed control authorization; errors never contain credentials.
#[derive(Debug, thiserror::Error)]
pub enum LocalVerificationError {
    /// Bad signature, expired token, wrong frame, or unsupported command.
    #[error("local session command rejected")]
    Rejected,
    /// Cryptographic or scope construction failure.
    #[error(transparent)]
    Capability(#[from] polaris_security_capability::CapabilityError),
    /// Approval evidence could not be stored and verified.
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    /// The stable evidence document could not be encoded.
    #[error("local approval encoding failed")]
    Encoding,
}

/// One session/frame-scoped issuer owned by the local composition root.
#[derive(Debug)]
pub struct LocalDirectiveVerifier {
    authority: Authority,
    frame: FrameId,
    resource: ResourcePath,
}
impl LocalDirectiveVerifier {
    /// Create an isolated authority; other kernels' credentials cannot authorize it.
    /// # Errors
    /// Entropy or scope construction failure.
    pub fn new(session: SessionId, frame: FrameId) -> Result<Self, LocalVerificationError> {
        Ok(Self {
            authority: Authority::new_session()?,
            frame,
            resource: ResourcePath::new(&format!("sessions/{session}/directives"))?,
        })
    }
    /// Mint a local control credential. It grants no workspace mutation scope.
    /// # Errors
    /// Invalid TTL, entropy or token encoding failure.
    pub fn credential(
        &self,
        ttl: Duration,
    ) -> Result<polaris_sdk::contracts::TokenBytes, LocalVerificationError> {
        let scope = Scope {
            resource: ResourceScope::Exact(self.resource.clone()),
            max_churn_lines: 0,
            ttl,
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::ExecuteTool]),
        };
        Ok(self
            .authority
            .issue_root(self.frame, &scope, &SystemClock)?
            .to_bytes()
            .clone())
    }
    /// Verify signatures, scope and frame; persist digest-bound evidence before promotion.
    /// # Errors
    /// Rejects unsupported commands, expired credentials and all evidence failures.
    pub fn verify(
        &self,
        command: Command<Unverified>,
        evidence: &impl EvidenceStore,
    ) -> Result<Command<Verified>, LocalVerificationError> {
        let token = Token::from_bytes(command.token().as_bytes(), &self.authority.public_key())?;
        if command.frame() != self.frame
            || !matches!(command.body(), CommandBody::ApplyDirective { .. })
        {
            return Err(LocalVerificationError::Rejected);
        }
        let facts = Facts {
            resource: &self.resource,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
            right: Right::ExecuteTool,
        };
        if authorize(&token, &facts, &self.authority.public_key(), &SystemClock)?
            != Authorization::Allowed
        {
            return Err(LocalVerificationError::Rejected);
        }
        let bytes = serde_json::to_vec(&(
            "polaris.local-control.v1",
            command.id(),
            command.frame(),
            command.body(),
        ))
        .map_err(|_| LocalVerificationError::Encoding)?;
        let expected = Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
        let cid = evidence.put(&bytes)?;
        if cid != expected || evidence.get(&cid)?.as_deref() != Some(bytes.as_slice()) {
            return Err(EvidenceError::DigestMismatch.into());
        }
        if authorize(&token, &facts, &self.authority.public_key(), &SystemClock)?
            != Authorization::Allowed
        {
            return Err(LocalVerificationError::Rejected);
        }
        let approver = ApproverId::new("polaris-kernel-verifier")
            .map_err(|_| LocalVerificationError::Encoding)?;
        Ok(command.verify(ApprovalWitness::new(
            approver,
            cid,
            polaris_sdk::contracts::Timestamp::now(),
        )))
    }
}
