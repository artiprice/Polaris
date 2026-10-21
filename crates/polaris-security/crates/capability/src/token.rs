//! Key ownership and bounded signature-chain parsing/attenuation.
use crate::{
    CapabilityError, Clock, MAX_TOKEN_BLOCKS, MAX_TOKEN_BYTES, Scope, VOCABULARY_VERSION,
    evaluation, scope::checks,
};
use biscuit_auth::{
    Biscuit, KeyPair, PublicKey, UnverifiedBiscuit,
    builder::{Algorithm, fact, int, string},
};
use polaris_sdk::{contracts::TokenBytes, ids::FrameId};
use std::{
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
};

/// Ed25519 public verification key. Safe to distribute; never private authority material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKeyBytes([u8; 32]);
impl PublicKeyBytes {
    /// Validate and wrap an Ed25519 public key.
    ///
    /// # Errors
    /// `BadSignature` for an invalid compressed public point.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, CapabilityError> {
        let _key = PublicKey::from_bytes(&bytes, Algorithm::Ed25519)
            .map_err(|_| CapabilityError::BadSignature)?;
        Ok(Self(bytes))
    }
    /// Public wire representation.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    fn key(self) -> Result<PublicKey, CapabilityError> {
        PublicKey::from_bytes(&self.0, Algorithm::Ed25519)
            .map_err(|_| CapabilityError::BadSignature)
    }
}
/// Session root keypair held only in the trusted root. No private-key accessor or serialization.
/// Debug is redacted; the pinned dependency zeroizes its signing key on drop.
pub struct Authority {
    key: KeyPair,
    public: PublicKeyBytes,
}
impl fmt::Debug for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Authority(<redacted>)")
    }
}
impl Authority {
    /// Generate with Biscuit's own Ed25519 OS-RNG API, independent of workspace rand's ABI.
    ///
    /// # Errors
    /// `Keygen` if the library's OS entropy path panics/fails.
    pub fn new_session() -> Result<Self, CapabilityError> {
        let key = catch_unwind(KeyPair::new).map_err(|_| CapabilityError::Keygen)?;
        let public = PublicKeyBytes(
            key.public()
                .to_bytes()
                .try_into()
                .map_err(|_| CapabilityError::Keygen)?,
        );
        Ok(Self { key, public })
    }
    /// Public issuer key for verification processes.
    #[must_use]
    pub const fn public_key(&self) -> PublicKeyBytes {
        self.public
    }
    /// Issue a signed bearer root. Frame identity is provenance, not holder binding.
    ///
    /// # Errors
    /// Invalid scope/time, entropy, encoding or size-limit failure.
    pub fn issue_root(
        &self,
        agent: FrameId,
        scope: &Scope,
        clock: &impl Clock,
    ) -> Result<Token, CapabilityError> {
        let mut builder = Biscuit::builder()
            .merge(checks(scope, clock)?)
            .fact(fact(
                "polaris_capability_version",
                &[int(VOCABULARY_VERSION)],
            ))
            .map_err(evaluation)?
            .fact(fact("issued_for_frame", &[string(&agent.to_string())]))
            .map_err(evaluation)?;
        for right in crate::Right::ALL
            .into_iter()
            .filter(|right| scope.rights.contains(*right))
        {
            builder = builder
                .fact(fact("granted_right", &[string(right.name())]))
                .map_err(evaluation)?;
        }
        let biscuit = catch_unwind(AssertUnwindSafe(|| builder.build(&self.key)))
            .map_err(|_| CapabilityError::Keygen)?
            .map_err(evaluation)?;
        Token::encode(biscuit, self.public)
    }
}
/// Signature-verified bounded bearer token. Debug never prints Datalog or bearer bytes.
#[derive(Clone)]
pub struct Token {
    pub(crate) biscuit: Biscuit,
    pub(crate) root: PublicKeyBytes,
    bytes: TokenBytes,
}
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token(<redacted, {} bytes>)", self.bytes.len())
    }
}
impl Token {
    /// Parse bounded protobuf, limit blocks, then verify every signature with the expected issuer.
    ///
    /// # Errors
    /// Malformed/oversized token, excessive chain, or invalid signature.
    pub fn from_bytes(bytes: &[u8], root: &PublicKeyBytes) -> Result<Self, CapabilityError> {
        if bytes.len() > MAX_TOKEN_BYTES {
            return Err(CapabilityError::Limit);
        }
        let unverified = UnverifiedBiscuit::from(bytes).map_err(|_| CapabilityError::Malformed)?;
        if unverified.block_count() > MAX_TOKEN_BLOCKS {
            return Err(CapabilityError::Limit);
        }
        let biscuit = unverified
            .verify(root.key()?)
            .map_err(|_| CapabilityError::BadSignature)?;
        Ok(Self {
            biscuit,
            root: *root,
            bytes: TokenBytes::from(bytes.to_vec()),
        })
    }
    fn encode(biscuit: Biscuit, root: PublicKeyBytes) -> Result<Self, CapabilityError> {
        let bytes = biscuit.to_vec().map_err(|_| CapabilityError::Malformed)?;
        if bytes.len() > MAX_TOKEN_BYTES || biscuit.block_count() > MAX_TOKEN_BLOCKS {
            return Err(CapabilityError::Limit);
        }
        Ok(Self {
            biscuit,
            root,
            bytes: TokenBytes::from(bytes),
        })
    }
    /// Opaque bearer bytes for transport. Treat these as credentials.
    #[must_use]
    pub const fn to_bytes(&self) -> &TokenBytes {
        &self.bytes
    }
    /// Chain length including authority.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.biscuit.block_count()
    }
    /// Signature identifiers for an external revocation store; any ancestor match revokes children.
    #[must_use]
    pub fn revocation_identifiers(&self) -> Vec<Vec<u8>> {
        self.biscuit.revocation_identifiers()
    }
}
/// Add only scope checks in a freshly signed block using the parent bearer delegation key.
/// Wider requests cannot remove parent constraints; no private authority key is required.
///
/// # Errors
/// Invalid scope, exhausted limits, sealed token, cryptographic or entropy failure.
pub fn attenuate(
    parent: &Token,
    child: &Scope,
    clock: &impl Clock,
) -> Result<Token, CapabilityError> {
    if parent.block_count() >= MAX_TOKEN_BLOCKS {
        return Err(CapabilityError::Limit);
    }
    let block = checks(child, clock)?;
    let biscuit = catch_unwind(AssertUnwindSafe(|| parent.biscuit.append(block)))
        .map_err(|_| CapabilityError::Keygen)?
        .map_err(evaluation)?;
    Token::encode(biscuit, parent.root)
}
