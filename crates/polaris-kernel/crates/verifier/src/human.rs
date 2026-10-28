//! Dedicated host answer authority. This issuer never grants mutation or process execution.
use polaris_sdk::{
    contracts::{
        ApprovalWitness, ApproverId, Command, CommandBody, TokenBytes, Unverified, Verified,
    },
    ids::{FrameId, SessionId},
};
use polaris_security_capability::{
    Authority, Authorization, Facts, PublicKeyBytes, ResourcePath, ResourceScope, Right, Rights,
    Scope, SystemClock, Token, authorize,
};

/// Dedicated verifier for the UI host's human-answer authority.
///
/// Its issuer is separate from local directives and sandbox execution. Credentials are scoped to
/// one session's `human_requests` resource and one root frame; the caller must keep this object in
/// trusted host composition and never expose `credential` to an agent tool or provider.
#[derive(Debug)]
pub struct HumanRequestVerifier {
    authority: Authority,
    frame: FrameId,
    resource: ResourcePath,
}
impl HumanRequestVerifier {
    /// Create a dedicated human-answer authority for a session and root frame.
    /// # Errors
    /// Entropy or resource validation failure.
    pub fn new(session: SessionId, root_frame: FrameId) -> Result<Self, HumanRequestVerifierError> {
        Ok(Self {
            authority: Authority::new_session()?,
            frame: root_frame,
            resource: ResourcePath::new(&format!("sessions/{session}/human_requests"))?,
        })
    }
    /// Issue bounded host-side answer authority after a trusted UI has presented a pending request.
    /// # Errors
    /// Rejects zero or over-five-minute lifetimes and token encoding failures.
    pub fn credential(
        &self,
        ttl: std::time::Duration,
    ) -> Result<TokenBytes, HumanRequestVerifierError> {
        if ttl.is_zero() || ttl > std::time::Duration::from_secs(300) {
            return Err(HumanRequestVerifierError::Rejected(
                "invalid host credential lifetime",
            ));
        }
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
    /// Verify a submitted command under this issuer and promote it after persisting bound evidence.
    /// # Errors
    /// Rejects the wrong frame/body, invalid capability, or CAS evidence failures.
    pub fn verify(
        &self,
        command: Command<Unverified>,
        evidence: &impl crate::EvidenceStore,
    ) -> Result<Command<Verified>, HumanRequestVerifierError> {
        if command.frame() != self.frame
            || !matches!(command.body(), CommandBody::AnswerHumanRequest { .. })
        {
            return Err(HumanRequestVerifierError::Rejected(
                "unsupported answer command",
            ));
        }
        let token = Token::from_bytes(command.token().as_bytes(), &self.authority.public_key())?;
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
            return Err(HumanRequestVerifierError::Rejected(
                "human answer capability denied",
            ));
        }
        let bytes = serde_json::to_vec(&(
            "polaris.human-answer.v1",
            command.id(),
            command.frame(),
            command.body(),
        ))
        .map_err(|_| HumanRequestVerifierError::Encoding)?;
        let digest = crate::verify::cid_for(&bytes);
        let cid = evidence
            .put(&bytes)
            .map_err(|_| HumanRequestVerifierError::Evidence)?;
        if cid != digest
            || evidence
                .get(&cid)
                .map_err(|_| HumanRequestVerifierError::Evidence)?
                .as_deref()
                != Some(bytes.as_slice())
        {
            return Err(HumanRequestVerifierError::Evidence);
        }
        if authorize(&token, &facts, &self.authority.public_key(), &SystemClock)?
            != Authorization::Allowed
        {
            return Err(HumanRequestVerifierError::Rejected(
                "human answer capability expired",
            ));
        }
        let approver = ApproverId::new("polaris-human-request-verifier")
            .map_err(|_| HumanRequestVerifierError::Encoding)?;
        Ok(command.verify(ApprovalWitness::new(
            approver,
            cid,
            polaris_sdk::contracts::Timestamp::now(),
        )))
    }
    /// Public key of this host answer authority, separate from active frame tool authorities.
    #[must_use]
    pub const fn issuer(&self) -> PublicKeyBytes {
        self.authority.public_key()
    }
}

/// Sanitized human-answer authority, command or durable evidence failure.
#[derive(Debug, thiserror::Error)]
pub enum HumanRequestVerifierError {
    /// Command, actor, scope or lifetime is invalid.
    #[error("human answer rejected: {0}")]
    Rejected(&'static str),
    /// Evidence could not be encoded.
    #[error("human answer evidence encoding failed")]
    Encoding,
    /// Content-addressed evidence could not be written and read back exactly.
    #[error("human answer evidence failed")]
    Evidence,
    /// Capability signature or authorization failed.
    #[error(transparent)]
    Capability(#[from] polaris_security_capability::CapabilityError),
}
