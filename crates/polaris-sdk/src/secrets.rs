//! Validated secret-source locations and the SDK credential-provider contract.

use std::sync::Arc;

use secrecy::SecretString;

use crate::error::{ErrorKind, HasKind, ParseError};

/// Maximum accepted length for an environment variable identifier.
pub const MAX_ENV_NAME_BYTES: usize = 64;
/// Maximum accepted length for each keyring identifier.
pub const MAX_KEYRING_PART_BYTES: usize = 128;

/// A validated environment variable name. This contains a location, never its value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvName(Box<str>);

impl EnvName {
    /// Validates an environment variable name in `[A-Z0-9_]{1,64}`.
    ///
    /// # Errors
    /// [`ParseError::InvalidValue`] if the name is empty, oversized, or outside the allowed
    /// alphabet.
    pub fn new(name: &str) -> Result<Self, ParseError> {
        if name.is_empty() {
            return Err(ParseError::InvalidValue {
                what: "environment variable name",
                reason: "empty",
            });
        }
        if name.len() > MAX_ENV_NAME_BYTES {
            return Err(ParseError::InvalidValue {
                what: "environment variable name",
                reason: "too long",
            });
        }
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ParseError::InvalidValue {
                what: "environment variable name",
                reason: "contains a disallowed character",
            });
        }
        Ok(Self(name.into()))
    }

    /// Returns the validated variable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated keyring service and account pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyringLocation {
    service: Box<str>,
    account: Box<str>,
}

impl KeyringLocation {
    /// Validates a keyring service/account pair. Each component is 1–128 ASCII alphanumeric,
    /// period, underscore, or hyphen characters.
    ///
    /// # Errors
    /// [`ParseError::InvalidValue`] if either component is empty, oversized, or contains a
    /// disallowed character.
    pub fn new(service: &str, account: &str) -> Result<Self, ParseError> {
        validate_keyring_part("keyring service", service)?;
        validate_keyring_part("keyring account", account)?;
        Ok(Self {
            service: service.into(),
            account: account.into(),
        })
    }

    /// Returns the validated service identifier.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Returns the validated account identifier.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }
}

fn validate_keyring_part(what: &'static str, value: &str) -> Result<(), ParseError> {
    if value.is_empty() {
        return Err(ParseError::InvalidValue {
            what,
            reason: "empty",
        });
    }
    if value.len() > MAX_KEYRING_PART_BYTES {
        return Err(ParseError::InvalidValue {
            what,
            reason: "too long",
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ParseError::InvalidValue {
            what,
            reason: "contains a disallowed character",
        });
    }
    Ok(())
}

/// Where a secret is retrieved. Its fields are validated identifier wrappers, never values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretLocation {
    /// A value stored in the platform credential manager.
    Keyring(KeyringLocation),
    /// A value read from the process environment.
    Environment(EnvName),
}

/// Sanitized failures from a secret-source adapter.
///
/// Variants intentionally carry no backend error strings, identifiers, or credential values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretSourceError {
    /// The credential provider is temporarily unavailable.
    #[error("secret source unavailable")]
    Unavailable,
    /// The host denied access to the requested secret operation.
    #[error("secret source permission denied")]
    PermissionDenied,
    /// The requested operation is not supported by this source (for example, writing an env var).
    #[error("secret source operation unsupported")]
    Unsupported,
    /// The source rejected a validated location due to backend-specific constraints.
    #[error("secret source location rejected")]
    InvalidLocation,
    /// The retrieved source value exceeds the caller's fixed maximum.
    #[error("secret source value exceeds the configured size limit")]
    TooLarge,
    /// The source failed for another reason; details are deliberately discarded.
    #[error("secret source operation failed")]
    Failed,
}

impl HasKind for SecretSourceError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable => ErrorKind::Transient,
            Self::PermissionDenied => ErrorKind::Policy,
            Self::Unsupported | Self::InvalidLocation | Self::TooLarge => ErrorKind::Permanent,
            Self::Failed => ErrorKind::Transient,
        }
    }
}

/// Registry seam for credential managers and process-environment access.
///
/// Implementations must not include secret values or operating-system error text in failures.
/// Environment implementations may load values but must reject store and delete operations;
/// mutating process environment is unsafe in a multithreaded application.
pub trait SecretSource: Send + Sync {
    /// Reads a secret. Missing entries return `Ok(None)`.
    ///
    /// # Errors
    /// [`SecretSourceError`] on provider, permission, or location failures.
    fn load(&self, location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError>;

    /// Stores a value in a keyring location. Environment locations must return
    /// [`SecretSourceError::Unsupported`].
    ///
    /// # Errors
    /// [`SecretSourceError`] on provider, permission, unsupported-operation, or location failures.
    fn store(
        &self,
        location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError>;

    /// Deletes a keyring value, returning `false` when it did not exist. Environment locations
    /// must return [`SecretSourceError::Unsupported`].
    ///
    /// # Errors
    /// [`SecretSourceError`] on provider, permission, unsupported-operation, or location failures.
    fn delete(&self, location: &SecretLocation) -> Result<bool, SecretSourceError>;
}

impl<T: SecretSource + ?Sized> SecretSource for Arc<T> {
    fn load(&self, location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        (**self).load(location)
    }

    fn store(
        &self,
        location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        (**self).store(location, value)
    }

    fn delete(&self, location: &SecretLocation) -> Result<bool, SecretSourceError> {
        (**self).delete(location)
    }
}
