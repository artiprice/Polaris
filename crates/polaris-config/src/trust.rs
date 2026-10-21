//! The repository-layer trust filter: a cloned repository's `.polaris/config.toml` is untrusted.

use std::collections::BTreeSet;

use crate::error::ConfigError;
use crate::layer::leaves;

/// Keys a repository layer may set without a grant: presentation and self-throttling only, never
/// endpoints, credentials, storage or the sandbox tier.
pub const REPO_ALLOWED: &[&str] = &["ui.*", "log.level", "concurrency.*", "ratelimit.*"];

fn matches_pattern(pattern: &str, key: &str) -> bool {
    pattern.strip_suffix(".*").map_or(pattern == key, |prefix| {
        key.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Repository keys the user has explicitly trusted (e.g. after a `/settings` prompt).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustGrant {
    keys: BTreeSet<String>,
}

impl TrustGrant {
    /// No extra trust: only [`REPO_ALLOWED`] keys pass.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Trusts these exact keys (or `prefix.*` patterns) in addition to [`REPO_ALLOWED`].
    #[must_use]
    pub fn keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            keys: keys.into_iter().map(Into::into).collect(),
        }
    }

    fn allows(&self, key: &str) -> bool {
        REPO_ALLOWED
            .iter()
            .copied()
            .chain(self.keys.iter().map(String::as_str))
            .any(|p| matches_pattern(p, key))
    }
}

/// Rejects repository-layer keys that are neither allow-listed nor granted.
///
/// # Errors
/// [`ConfigError::UntrustedRepoKey`] naming the first offending key.
pub fn filter_repo_layer(table: &toml::Table, grant: &TrustGrant) -> Result<(), ConfigError> {
    match leaves(table)
        .into_iter()
        .find(|(key, _)| !grant.allows(key))
    {
        Some((key, _)) => Err(ConfigError::UntrustedRepoKey(key)),
        None => Ok(()),
    }
}
