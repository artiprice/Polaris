//! Secret references. Configuration never holds secret values, only where to find them.

use crate::error::{ConfigError, invalid};

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

fn is_keyring_part(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_env_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

impl SecretRef {
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
            let mut parts = rest.splitn(2, '/');
            let (service, user) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            if is_keyring_part(service) && is_keyring_part(user) {
                return Ok(Self::Keyring {
                    service: service.to_owned(),
                    user: user.to_owned(),
                });
            }
            return Err(invalid(key, "expected keyring:service/user"));
        }
        if let Some(name) = value.strip_prefix("env:") {
            if is_env_name(name) {
                return Ok(Self::Env(name.to_owned()));
            }
            return Err(invalid(key, "expected env:NAME with NAME in [A-Z0-9_]"));
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
