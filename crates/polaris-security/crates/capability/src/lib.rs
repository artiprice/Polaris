//! Signed bearer capabilities with monotone Biscuit attenuation and default-deny authorization.
//!
//! Generated blocks contain checks and authority grants/provenance, never ambient mutation facts
//! or rules. Checks explicitly trust authority (which also includes their own block and the
//! authorizer in Biscuit 6); descendant facts cannot affect ancestor checks. The allow policy
//! trusts authority grants. This bounds actions routed through this
//! API; it neither detects prompt injection nor validates intent or actual filesystem effects.
//!
//! Paths are lexical workspace-relative identities. The mutation owner must resolve symlinks and
//! verify workspace containment before constructing facts. Tokens are bearer credentials: the
//! issuing frame is an audit fact, not a non-transferable identity. Clock rollback before issuance
//! is denied; expiry uses conservative microsecond precision. Authorization is blocking CPU work.
//!
//! Private key access is absent:
//! ```compile_fail
//! use polaris_security_capability::Authority;
//! fn extract(authority: Authority) { let _secret = authority.key; }
//! ```
//! Token internals cannot be replaced by a caller:
//! ```compile_fail
//! use polaris_security_capability::Token;
//! fn replace(token: &mut Token) { token.bytes = Default::default(); }
//! ```

mod authorize;
mod scope;
mod token;

pub use authorize::{Authorization, CheckId, Facts, authorize, authorize_with_revocations};
pub use scope::{ResourcePath, ResourceScope, Right, Rights, Scope};
pub use token::{Authority, PublicKeyBytes, Token, attenuate};

use polaris_sdk::{
    contracts::Timestamp,
    error::{ErrorKind, HasKind},
};

/// Datalog vocabulary revision required from the authority block.
pub const VOCABULARY_VERSION: i64 = 1;
/// Maximum serialized token accepted before protobuf parsing.
pub const MAX_TOKEN_BYTES: usize = 64 * 1024;
/// Maximum signature blocks accepted before cryptographic verification.
pub const MAX_TOKEN_BLOCKS: usize = 32;

/// Trusted injectable wall clock; production uses [`SystemClock`].
pub trait Clock {
    /// Current UTC timestamp.
    fn now(&self) -> Timestamp;
}
/// Real UTC wall clock. Time jumps affect expiry; no artificial fixed epoch is used.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// Fail-closed capability failures. Diagnostics never include token bytes or Datalog source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// Invalid chain or wrong expected issuer key.
    #[error("capability signature verification failed")]
    BadSignature,
    /// Invalid protobuf or unsupported token encoding.
    #[error("malformed capability")]
    Malformed,
    /// Invalid lexical resource, empty rights, or unusable lifetime.
    #[error("invalid capability scope")]
    InvalidScope,
    /// Token size, block count, or Datalog evaluation limit exceeded.
    #[error("capability resource limit exceeded")]
    Limit,
    /// Datalog construction/execution failed; never an approval.
    #[error("capability evaluator failed")]
    Evaluation,
    /// Underlying cryptographic key generation failed or panicked on unavailable entropy.
    #[error("capability key generation failed")]
    Keygen,
    /// Expiration cannot be represented safely.
    #[error("capability timestamp overflow")]
    Clock,
}
impl HasKind for CapabilityError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::BadSignature | Self::Limit => ErrorKind::Policy,
            Self::Malformed | Self::InvalidScope => ErrorKind::Permanent,
            Self::Keygen => ErrorKind::Transient,
            Self::Evaluation | Self::Clock => ErrorKind::Fatal,
        }
    }
}
fn evaluation(error: biscuit_auth::error::Token) -> CapabilityError {
    match error {
        biscuit_auth::error::Token::RunLimit(_) => CapabilityError::Limit,
        _ => CapabilityError::Evaluation,
    }
}
