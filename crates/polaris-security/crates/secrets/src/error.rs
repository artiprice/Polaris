//! Sanitized errors returned by the secret vault and its system adapters.

use polaris_sdk::error::{ErrorKind, HasKind};

use crate::SecretName;

/// Secret operation failures. No variant carries plaintext or operating-system error strings.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// The explicitly configured recovery key is missing or is not exactly 32 encoded bytes.
    #[error("sealing recovery key is missing or invalid")]
    InvalidSealKey,
    /// The requested value is not loaded into the vault.
    #[error("secret not found: {0:?}")]
    NotFound(SecretName),
    /// The requested environment variable is missing.
    #[error("environment secret is missing: {0}")]
    EnvMissing(String),
    /// A provider operation failed with a sanitized SDK error.
    #[error("secret source failure: {0}")]
    Source(#[from] polaris_sdk::secrets::SecretSourceError),
    /// The source returned a value that is not valid UTF-8.
    #[error("secret source value is not valid UTF-8")]
    Utf8,
    /// A secret exceeds the vault's fixed size bound.
    #[error("secret size limit exceeded: {bytes} bytes (limit {limit})")]
    TooLarge {
        /// Actual secret size in bytes.
        bytes: usize,
        /// Maximum accepted secret size in bytes.
        limit: usize,
    },
    /// Configured in-memory vault capacity would be exceeded.
    #[error("secret vault {resource:?} capacity exceeded (limit {limit})")]
    Capacity {
        /// Capacity being enforced.
        resource: CapacityResource,
        /// Maximum permitted count or byte total.
        limit: usize,
    },
    /// Secure random number generation failed.
    #[error("secure entropy unavailable")]
    Entropy,
    /// Encryption failed.
    #[error("secret sealing failed")]
    Seal,
    /// Ciphertext authentication or decryption failed.
    #[error("secret opening failed")]
    Open,
    /// A requested keyring reference was malformed.
    #[error("invalid keyring location")]
    InvalidKeyringLocation,
    /// A requested in-memory secret name was malformed.
    #[error("invalid secret name")]
    InvalidName,
}

/// Resource dimension whose configured vault capacity was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityResource {
    /// Maximum stored secret count.
    Entries,
    /// Maximum aggregate plaintext byte count represented by stored entries.
    PlaintextBytes,
}

impl HasKind for SecretsError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFound(_)
            | Self::EnvMissing(_)
            | Self::TooLarge { .. }
            | Self::Capacity { .. }
            | Self::Utf8
            | Self::InvalidKeyringLocation => ErrorKind::Permanent,
            Self::InvalidName | Self::InvalidSealKey => ErrorKind::Permanent,
            Self::Source(source) => source.kind(),
            Self::Entropy | Self::Seal | Self::Open => ErrorKind::Fatal,
        }
    }
}
