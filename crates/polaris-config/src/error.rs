//! Configuration errors. Messages name keys and files, never secret values.

use std::path::PathBuf;

use polaris_sdk::error::{ErrorKind, HasKind};

/// Configuration failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A config file could not be read.
    #[error("cannot read {path}: {reason}", path = path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// OS error text.
        reason: String,
    },
    /// A configuration file exceeded the bounded parser input size.
    #[error("configuration file {path} exceeds the {limit}-byte limit", path = path.display())]
    FileTooLarge {
        /// The file.
        path: PathBuf,
        /// Maximum accepted file size.
        limit: usize,
    },
    /// TOML syntax error.
    #[error("{file}:{line}: {reason}", file = file.display())]
    Parse {
        /// The file (or `<Layer>` for in-memory layers).
        file: PathBuf,
        /// 1-based line.
        line: u32,
        /// Parser message.
        reason: String,
    },
    /// A key that is not part of the configuration schema.
    #[error("unknown configuration key `{0}`")]
    UnknownKey(String),
    /// A credential-like key holds a literal instead of a `keyring:`/`env:` reference.
    #[error(
        "`{0}` must be a secret reference (keyring:service/user or env:NAME), not a literal value"
    )]
    SecretLiteral(String),
    /// A repository layer tried to set a key it is not trusted to set.
    #[error(
        "repository config may not set `{0}` (set it in ~/.polaris/config.toml or grant trust)"
    )]
    UntrustedRepoKey(String),
    /// A known key has an invalid value.
    #[error("invalid value for `{key}`: {reason}")]
    Invalid {
        /// The key.
        key: String,
        /// What is wrong.
        reason: String,
    },
    /// The user's home directory could not be determined.
    #[error("cannot determine the home directory")]
    NoHome,
    /// Session overrides could not be rendered as TOML.
    #[error("cannot encode session overrides: {0}")]
    Encode(String),
}

impl HasKind for ConfigError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { .. } => ErrorKind::Transient,
            Self::SecretLiteral(_) | Self::UntrustedRepoKey(_) => ErrorKind::Policy,
            Self::Parse { .. }
            | Self::UnknownKey(_)
            | Self::Invalid { .. }
            | Self::NoHome
            | Self::Encode(_)
            | Self::FileTooLarge { .. } => ErrorKind::Permanent,
        }
    }
}

pub(crate) fn invalid(key: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
        reason: reason.into(),
    }
}
