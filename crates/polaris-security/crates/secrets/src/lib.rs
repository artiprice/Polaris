//! # polaris-security-secrets
//!
//! Secret vault: keyring storage (Windows Credential Manager), secrecy/zeroize handling, ChaCha20-Poly1305 RAM sealing.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L1 — foundations |
//! | Kind | lib |
//! | Phase | P3 |
//! | Parent facade | `polaris-security` |
//! | Status | implemented |
//!
//! ## Responsibilities
//!
//! - Load NEBIUS_API_KEY / TAVILY_API_KEY from keyring or env into `SecretBox` values.
//! - Seal secrets at rest in RAM; decrypt only for the duration of a call; zeroize on drop.
//! - Keep key and ciphertext in separate allocations (chatlog correction).
//!
//! ## Security boundary
//!
//! The keyring adapter and environment reader implement the SDK's [`SecretSource`] registry
//! contract. Environment values are read only; writes and deletes are rejected. The in-memory
//! default session key is ephemeral; explicit recovery reads a host-configured key source without
//! writing or replacing it. The vault's encryption only protects values from accidental
//! plaintext retention; it does not defend against an attacker executing in the same process.

mod dotenv;
mod error;
mod source;
mod vault;

pub use dotenv::{DotenvError, DotenvSecretSource, MAX_DOTENV_BYTES};
pub use error::{CapacityResource, SecretsError};
pub use polaris_sdk::secrets::{EnvName, KeyringLocation, SecretLocation, SecretSource};
pub use source::SystemSecretSource;
pub use vault::{
    MAX_KEYRING_SECRET_BYTES, MAX_SECRET_BYTES, SealedBlob, SecretName, SecretVault,
    SessionSealKey, VaultLimits,
};
