//! Secret references. Configuration never holds secret values, only where to find them.

use crate::error::{ConfigError, invalid};
use polaris_sdk::{
    error::ParseError,
    secrets::{EnvName, KeyringLocation, SecretLocation},
};

/// Where a secret lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretRef {
    /// OS credential store entry (`keyring:service/user`).
    Keyring {
        /// Service name.
        service: String,
        /// User / account name.
        user: String,
    },
    /// Environment variable (`env:NAME`, `NAME` in `[A-Z0-9_]+`).
    Env(String),
}

impl SecretRef {
    /// Convert a configuration reference into a validated SDK provider location.
    ///
    /// This validates even manually constructed public enum variants; it never retrieves a value.
    /// # Errors
    /// Returns a sanitized parse error for malformed or oversized identifiers.
    pub fn location(&self) -> Result<SecretLocation, ParseError> {
        match self {
            Self::Env(name) => EnvName::new(name).map(SecretLocation::Environment),
            Self::Keyring { service, user } => {
                KeyringLocation::new(service, user).map(SecretLocation::Keyring)
            }
        }
    }
    /// Parses a reference; the error never contains the value (it may be a leaked secret).
    ///
    /// # Errors
    /// [`ConfigError::SecretLiteral`] for anything that is not a `keyring:` / `env:` reference,
    /// [`ConfigError::Invalid`] for a malformed reference.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        Self::parse_for("secret reference", value)
    }

    /// As [`SecretRef::parse`], naming the configuration key in errors.
    ///
    /// # Errors
    /// As [`SecretRef::parse`].
    pub fn parse_for(key: &str, value: &str) -> Result<Self, ConfigError> {
        if let Some(rest) = value.strip_prefix("keyring:") {
            if let Some((service, user)) = rest.split_once('/') {
                let reference = Self::Keyring {
                    service: service.to_owned(),
                    user: user.to_owned(),
                };
                reference
                    .location()
                    .map_err(|_| invalid(key, "expected bounded keyring:service/user"))?;
                return Ok(reference);
            }
            return Err(invalid(key, "expected keyring:service/user"));
        }
        if let Some(name) = value.strip_prefix("env:") {
            let reference = Self::Env(name.to_owned());
            reference
                .location()
                .map_err(|_| invalid(key, "expected env:NAME with NAME in [A-Z0-9_]{1,64}"))?;
            return Ok(reference);
        }
        Err(ConfigError::SecretLiteral(key.to_owned()))
    }

    /// The canonical text form (`keyring:service/user` or `env:NAME`).
    #[must_use]
    pub fn to_reference_string(&self) -> String {
        match self {
            Self::Keyring { service, user } => format!("keyring:{service}/{user}"),
            Self::Env(name) => format!("env:{name}"),
        }
    }
}

/// `true` for keys that hold credentials (their values must be references).
pub(crate) fn is_secret_key(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
    ["api_key", "token", "secret", "password"]
        .iter()
        .any(|marker| last.contains(marker))
}

/// Rejects literal values under credential-like keys.
pub(crate) fn check_secret_value(path: &str, value: &toml::Value) -> Result<(), ConfigError> {
    if !is_secret_key(path) {
        return Ok(());
    }
    match value {
        toml::Value::String(text) => SecretRef::parse_for(path, text).map(|_| ()),
        _ => Err(ConfigError::SecretLiteral(path.to_owned())),
    }
}
