//! # polaris-security-pii
//!
//! Bounded, session-local PII masking using the pinned security automata DFA and entropy
//! scanner. The crate does not claim semantic name detection: the SDK decision engine exposes
//! text classification but no trusted entity offsets, so NER coverage is unavailable here.
//! Masking reports DFA-plus-entropy-only degraded coverage and never treats that as complete DLP.
//!
//! Unmasking requires an injected [`SandboxUnmaskAuthorizer`] and returns zeroizing plaintext.
//! The default authorizer always denies. This crate does not persist vault contents; a sealed
//! resume adapter remains deferred until a real sealer is wired by the composition root.

mod error;
mod vault;

pub use error::PiiError;
pub use vault::{
    DenySandboxUnmask, Masked, PiiKind, PiiLimits, PiiVault, SandboxUnmaskAuthorizer,
    SanitizeReport, SealedBlob, Sealer,
};
