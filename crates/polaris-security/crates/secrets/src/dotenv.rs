//! Explicit `.env` file overlay for environment secret references.
//!
//! The overlay is read from one path the host names (for example `polaris --env-file .env`); it is
//! never discovered from a workspace, so repository content cannot supply credentials on its own.
//! Values never enter the process environment: `std::env::set_var` is not used.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use polaris_sdk::secrets::{EnvName, SecretLocation, SecretSource, SecretSourceError};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

/// Largest accepted `.env` file.
pub const MAX_DOTENV_BYTES: u64 = 64 * 1024;

/// Why an `.env` file was rejected. Messages never contain file content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DotenvError {
    /// The file could not be opened or read.
    #[error("environment file could not be read")]
    Io,
    /// The file exceeds [`MAX_DOTENV_BYTES`] or a value exceeds the secret size limit.
    #[error("environment file is too large")]
    TooLarge,
    /// The file is not UTF-8.
    #[error("environment file is not valid UTF-8")]
    Encoding,
    /// A non-comment line is not `NAME=value` with `NAME` in `[A-Z0-9_]{1,64}`.
    #[error("environment file line {line} is not NAME=value")]
    Syntax {
        /// One-based line number.
        line: usize,
    },
}

/// Environment values loaded from a `.env` file, overlaid on another source.
pub struct DotenvSecretSource<S> {
    values: BTreeMap<EnvName, SecretString>,
    inner: S,
}

impl<S> std::fmt::Debug for DotenvSecretSource<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DotenvSecretSource")
            .field("names", &self.values.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl<S: SecretSource> DotenvSecretSource<S> {
    /// Read `path` and overlay its values on `inner` for environment locations.
    /// # Errors
    /// Returns a typed error for unreadable, oversized, non-UTF-8 or malformed files.
    pub fn open(path: &Path, inner: S) -> Result<Self, DotenvError> {
        let file = File::open(path).map_err(|_| DotenvError::Io)?;
        let mut text = Zeroizing::new(String::new());
        let read = file
            .take(MAX_DOTENV_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::InvalidData {
                    DotenvError::Encoding
                } else {
                    DotenvError::Io
                }
            })?;
        if u64::try_from(read).map_or(true, |bytes| bytes > MAX_DOTENV_BYTES) {
            return Err(DotenvError::TooLarge);
        }
        Ok(Self {
            values: parse(&text)?,
            inner,
        })
    }

    /// Names defined by the file, without values.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(EnvName::as_str)
    }
}

fn parse(text: &str) -> Result<BTreeMap<EnvName, SecretString>, DotenvError> {
    let mut values = BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let syntax = DotenvError::Syntax { line: index + 1 };
        let (name, value) = line.split_once('=').ok_or(syntax)?;
        let name = EnvName::new(name.trim()).map_err(|_| syntax)?;
        let value = unquote(value.trim());
        if value.len() > crate::MAX_SECRET_BYTES {
            return Err(DotenvError::TooLarge);
        }
        if value.is_empty() {
            continue;
        }
        drop(values.insert(name, SecretString::from(value)));
    }
    Ok(values)
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

impl<S: SecretSource> SecretSource for DotenvSecretSource<S> {
    fn load(&self, location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        if let SecretLocation::Environment(name) = location
            && let Some(value) = self.values.get(name)
        {
            return Ok(Some(SecretString::from(value.expose_secret())));
        }
        self.inner.load(location)
    }

    fn store(
        &self,
        location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        self.inner.store(location, value)
    }

    fn delete(&self, location: &SecretLocation) -> Result<bool, SecretSourceError> {
        self.inner.delete(location)
    }
}

#[cfg(test)]
mod tests {
    use super::{DotenvError, parse};
    use secrecy::ExposeSecret;

    #[test]
    fn parses_names_values_comments_quotes_and_export() -> Result<(), DotenvError> {
        let values = parse(
            "# comment\n\nOPENROUTER_API_KEY=sk-or-1\nexport NEBIUS_API_KEY=\"nb 2\"\nTAVILY_API_KEY='tv3'\nEMPTY=\n",
        )?;
        let get = |name: &str| {
            values
                .iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, value)| value.expose_secret().to_owned())
        };
        assert_eq!(get("OPENROUTER_API_KEY").as_deref(), Some("sk-or-1"));
        assert_eq!(get("NEBIUS_API_KEY").as_deref(), Some("nb 2"));
        assert_eq!(get("TAVILY_API_KEY").as_deref(), Some("tv3"));
        assert_eq!(get("EMPTY"), None, "empty values are treated as unset");
        Ok(())
    }

    #[test]
    fn rejects_malformed_lines_without_echoing_content() {
        let error = parse("GOOD=1\nnot a pair\n").err();
        assert_eq!(error, Some(DotenvError::Syntax { line: 2 }));
        assert_eq!(
            parse("lower=1").err(),
            Some(DotenvError::Syntax { line: 1 })
        );
        let message = DotenvError::Syntax { line: 2 }.to_string();
        assert!(!message.contains("not a pair"));
    }
}
