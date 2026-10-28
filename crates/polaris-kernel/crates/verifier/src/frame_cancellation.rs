//! Dedicated exact-target authority for cancelling a frame subtree.
//!
//! This issuer is independent from mutation, human-answer and tool-execution authorities. Its
//! only capability is `Right::CancelFrame` scoped to one session and one target frame.

use crate::{EvidenceError, EvidenceStore};
use polaris_sdk::{
    contracts::{
        ApprovalWitness, ApproverId, Command, CommandBody, TokenBytes, Unverified, Verified,
    },
    ids::{CommandId, FrameId, SessionId},
};
use polaris_security_capability::{
    Authority, Authorization, Facts, PublicKeyBytes, ResourcePath, ResourceScope, Right, Rights,
    Scope, SystemClock, Token, authorize,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Versioned, secret-free approval receipt for one target-specific cancellation command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameCancellationEvidence {
    /// Evidence schema version.
    pub schema: u16,
    /// Session whose frame tree owns the target.
    pub session: SessionId,
    /// Trusted host root that submitted the command.
    pub actor: FrameId,
    /// Exact frame subtree authorized for cancellation.
    pub target: FrameId,
    /// Command id used for idempotency.
    pub command: CommandId,
    /// Digest of the bearer token; the token itself is never persisted.
    pub token_digest: [u8; 32],
}

/// Dedicated host authority for exact-target frame cancellation.
#[derive(Debug)]
pub struct FrameCancellationVerifier {
    authority: Authority,
    session: SessionId,
    actor: FrameId,
}

impl FrameCancellationVerifier {
    /// Create a session-isolated cancellation issuer bound to its trusted host root frame.
    ///
    /// # Errors
    /// Returns an entropy failure if a secure issuer cannot be created.
    pub fn new(session: SessionId, actor: FrameId) -> Result<Self, FrameCancellationVerifierError> {
        Ok(Self {
            authority: Authority::new_session()?,
            session,
            actor,
        })
    }

    /// Mint short-lived authority for exactly one target frame; this grants no execution right.
    ///
    /// # Errors
    /// Rejects zero or over-five-minute lifetimes and token construction failures.
    pub fn credential(
        &self,
        target: FrameId,
        ttl: Duration,
    ) -> Result<TokenBytes, FrameCancellationVerifierError> {
        if ttl.is_zero() || ttl > Duration::from_secs(300) {
            return Err(FrameCancellationVerifierError::Rejected);
        }
        let resource = cancellation_resource(self.session, target)?;
        let scope = Scope {
            resource: ResourceScope::Exact(resource),
            max_churn_lines: 0,
            ttl,
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::CancelFrame]),
        };
        Ok(self
            .authority
            .issue_root(self.actor, &scope, &SystemClock)?
            .to_bytes()
            .clone())
    }

    /// Verify the root actor, exact target scope and signed command, then persist secret-free
    /// digest-bound evidence before promoting it to `Verified`.
    ///
    /// # Errors
    /// Rejects a wrong command/actor/scope, expired credential, or any evidence-store failure.
    pub fn verify(
        &self,
        command: Command<Unverified>,
        evidence: &impl EvidenceStore,
    ) -> Result<Command<Verified>, FrameCancellationVerifierError> {
        let CommandBody::CancelFrame { frame: target } = *command.body() else {
            return Err(FrameCancellationVerifierError::Rejected);
        };
        if command.frame() != self.actor {
            return Err(FrameCancellationVerifierError::Rejected);
        }
        let resource = cancellation_resource(self.session, target)?;
        let token = Token::from_bytes(command.token().as_bytes(), &self.authority.public_key())?;
        let facts = Facts {
            resource: &resource,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
            right: Right::CancelFrame,
        };
        authorize_exact(&token, &facts, &self.authority.public_key())?;
        let receipt = FrameCancellationEvidence {
            schema: 1,
            session: self.session,
            actor: self.actor,
            target,
            command: command.id(),
            token_digest: *blake3::hash(command.token().as_bytes()).as_bytes(),
        };
        let bytes =
            serde_json::to_vec(&receipt).map_err(|_| FrameCancellationVerifierError::Encoding)?;
        let expected = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
        let cid = evidence.put(&bytes)?;
        if cid != expected || evidence.get(&cid)?.as_deref() != Some(bytes.as_slice()) {
            return Err(FrameCancellationVerifierError::Evidence(
                EvidenceError::DigestMismatch,
            ));
        }
        authorize_exact(&token, &facts, &self.authority.public_key())?;
        let approver = ApproverId::new("polaris-frame-cancellation-verifier")
            .map_err(|_| FrameCancellationVerifierError::Encoding)?;
        Ok(command.verify(ApprovalWitness::new(
            approver,
            cid,
            polaris_sdk::contracts::Timestamp::now(),
        )))
    }

    /// Issuer key used only to validate this dedicated cancellation capability.
    #[must_use]
    pub const fn issuer(&self) -> PublicKeyBytes {
        self.authority.public_key()
    }
}

/// Validate a stored receipt against the exact host actor, target and command identity.
///
/// # Errors
/// Rejects corrupt, mismatched, unknown-schema or noncanonical approval evidence.
pub fn validate_cancellation_evidence(
    evidence: &impl EvidenceStore,
    cid: polaris_sdk::ids::Cid,
    session: SessionId,
    actor: FrameId,
    target: FrameId,
    command: CommandId,
    token: &TokenBytes,
) -> Result<(), FrameCancellationVerifierError> {
    let receipt = load_cancellation_evidence(evidence, cid, session, actor, target, command)?;
    if receipt.token_digest != *blake3::hash(token.as_bytes()).as_bytes() {
        return Err(FrameCancellationVerifierError::Rejected);
    }
    Ok(())
}

/// Read and validate the immutable, secret-free cancellation approval receipt by identity.
///
/// This is used during restart replay, when the original bearer token must not be retained.
///
/// # Errors
/// Rejects corrupt, mismatched, unknown-schema or noncanonical approval evidence.
pub fn load_cancellation_evidence(
    evidence: &impl EvidenceStore,
    cid: polaris_sdk::ids::Cid,
    session: SessionId,
    actor: FrameId,
    target: FrameId,
    command: CommandId,
) -> Result<FrameCancellationEvidence, FrameCancellationVerifierError> {
    let bytes = evidence
        .get(&cid)?
        .ok_or(FrameCancellationVerifierError::Evidence(
            EvidenceError::DigestMismatch,
        ))?;
    let expected = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
    if expected != cid {
        return Err(FrameCancellationVerifierError::Evidence(
            EvidenceError::DigestMismatch,
        ));
    }
    let receipt: FrameCancellationEvidence =
        serde_json::from_slice(&bytes).map_err(|_| FrameCancellationVerifierError::Encoding)?;
    if receipt.schema != 1
        || receipt.session != session
        || receipt.actor != actor
        || receipt.target != target
        || receipt.command != command
    {
        return Err(FrameCancellationVerifierError::Rejected);
    }
    Ok(receipt)
}

fn cancellation_resource(
    session: SessionId,
    target: FrameId,
) -> Result<ResourcePath, FrameCancellationVerifierError> {
    Ok(ResourcePath::new(&format!(
        "sessions/{session}/frames/{target}/cancel"
    ))?)
}

fn authorize_exact(
    token: &Token,
    facts: &Facts<'_>,
    issuer: &PublicKeyBytes,
) -> Result<(), FrameCancellationVerifierError> {
    if authorize(token, facts, issuer, &SystemClock)? == Authorization::Allowed {
        Ok(())
    } else {
        Err(FrameCancellationVerifierError::Rejected)
    }
}

/// Sanitized failure from the host frame-cancellation authorization boundary.
#[derive(Debug, thiserror::Error)]
pub enum FrameCancellationVerifierError {
    /// Actor, command kind, target, capability scope or lifetime is invalid.
    #[error("frame cancellation rejected")]
    Rejected,
    /// The signed capability could not be decoded or authorized.
    #[error(transparent)]
    Capability(#[from] polaris_security_capability::CapabilityError),
    /// Approval evidence could not be persisted and read back exactly.
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    /// Evidence or a trusted receipt could not be encoded or decoded.
    #[error("frame cancellation evidence encoding failed")]
    Encoding,
}
