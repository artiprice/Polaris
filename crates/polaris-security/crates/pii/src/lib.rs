//! # polaris-security-pii
//!
//! Bounded, session-local PII masking using the pinned security automata DFA and entropy scanner,
//! with optional separately supplied semantic spans from the local PII companion model. The
//! semantic path consumes validated raw UTF-8 byte offsets only and does not receive or persist
//! model labels. Unavailable, timed-out, or malformed detection falls back to deterministic
//! masking and reports degraded coverage; this is not a complete DLP guarantee.
//!
//! Unmasking requires an injected [`SandboxUnmaskAuthorizer`] and returns zeroizing plaintext.
//! The default authorizer always denies. Bounded sealed map export/import requires an explicit
//! authenticated sealer; the kernel supplies actual AEAD and private CAS snapshot storage. Keys
//! are never generated, persisted or recovered implicitly by this crate.

mod error;
mod vault;

pub use error::PiiError;
pub use vault::{
    DenySandboxUnmask, Masked, PiiKind, PiiLimits, PiiVault, SandboxUnmaskAuthorizer,
    SanitizeReport, SealedBlob, Sealer,
};
