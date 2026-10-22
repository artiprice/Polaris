//! Validated names and paths used at git process boundaries.

use std::fmt;

use crate::VcsError;

/// A repository-relative path that cannot escape through separators or dot components.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(String);

impl RepoPath {
    /// Validates a repository-relative `/` separated path.
    ///
    /// # Errors
    /// Returns [`VcsError::InvalidRef`] for absolute paths, traversal, empty components, NUL, or
    /// Windows drive syntax.
    pub fn parse(value: &str) -> Result<Self, VcsError> {
        if value.is_empty()
            || value.len() > 4096
            || value.starts_with('/')
            || value.contains(['\\', '\0'])
        {
            return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
        }
        let mut first = true;
        for component in value.split('/') {
            if component.is_empty()
                || component.len() > 255
                || matches!(component, "." | "..")
                || component.eq_ignore_ascii_case(".git")
                || component.eq_ignore_ascii_case(".polaris")
                || component.ends_with('.')
                || component.ends_with(' ')
                || component
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
            {
                return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
            }
            let stem = match component.split('.').next() {
                Some(stem) => stem.to_ascii_uppercase(),
                None => String::new(),
            };
            if [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
            {
                return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
            }
            let mut chars = component.chars();
            if first
                && chars
                    .next()
                    .is_some_and(|character| character.is_ascii_alphabetic())
                && chars.next() == Some(':')
            {
                return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
            }
            first = false;
        }
        Ok(Self(value.to_owned()))
    }

    /// The normalized repository-relative path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Components split on `/`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated branch name accepted by git's ref-name rules and Windows path rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Validates a branch name. It can never begin with `-` or contain `..`.
    ///
    /// # Errors
    /// Returns [`VcsError::InvalidRef`] if any component violates git or Windows restrictions.
    pub fn parse(value: &str) -> Result<Self, VcsError> {
        if value.is_empty()
            || value.len() > 256
            || value.starts_with('-')
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains("..")
            || value.contains("@{")
            || value.contains([
                '\\', '\0', ' ', '~', '^', ':', '?', '*', '[', '<', '>', '"', '|',
            ])
            || value == "@"
        {
            return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
        }
        for component in value.split('/') {
            let upper = component.to_ascii_uppercase();
            let stem = match component.split('.').next() {
                Some(stem) => stem.to_ascii_uppercase(),
                None => upper.clone(),
            };
            if component.is_empty()
                || component.starts_with('.')
                || component.ends_with('.')
                || component.to_ascii_lowercase().ends_with(".lock")
                || component.len() > 255
                || [
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ]
                .contains(&stem.as_str())
            {
                return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
            }
        }
        Ok(Self(value.to_owned()))
    }

    /// The validated branch name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A single validated revision expression (commit id, branch, tag, or `HEAD~N`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevSpec(String);

impl RevSpec {
    /// Validates a bounded revision expression without range or option syntax.
    ///
    /// # Errors
    /// Returns [`VcsError::InvalidRef`] when the expression contains option or revision-range
    /// syntax, control characters, or exceeds the length limit.
    pub fn parse(value: &str) -> Result<Self, VcsError> {
        let valid_hex = value.len() == 40 || value.len() == 64;
        let is_hex = value.chars().all(|character| character.is_ascii_hexdigit());
        let simple = BranchName::parse(value).is_ok();
        let head_parent = value
            .strip_prefix("HEAD~")
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
        let branch_like = simple
            && !value.starts_with('-')
            && !value.contains([' ', '\0', '^', ':', '?', '*', '[', '\\']);
        if value.is_empty()
            || value.len() > 256
            || value.starts_with('-')
            || value.chars().any(char::is_control)
            || !(head_parent || (valid_hex && is_hex) || branch_like)
        {
            return Err(VcsError::InvalidRef(value.chars().take(256).collect()));
        }
        Ok(Self(value.to_owned()))
    }

    /// The validated revision expression.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
