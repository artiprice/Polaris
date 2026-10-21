//! Workspace-relative paths and sync errors.

use std::fmt;

use polaris_sdk::error::{ErrorKind, HasKind};

/// Sync failures. Per-file transfer failures are reported in [`crate::SyncReport`] instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    /// A path is not a valid workspace-relative path.
    #[error("invalid path {path:?}: {reason}")]
    InvalidPath {
        /// The rejected text.
        path: String,
        /// Why.
        reason: &'static str,
    },
    /// A path or link target leaves the workspace.
    #[error("path escapes the workspace: {0}")]
    RootEscape(String),
    /// Walking the workspace failed.
    #[error("walk failed: {0}")]
    Walk(String),
    /// Reading a file failed.
    #[error("cannot read {path}: {reason}")]
    Io {
        /// Relative path.
        path: String,
        /// OS error text.
        reason: String,
    },
    /// Invalid ignore pattern.
    #[error("invalid ignore rule: {0}")]
    Rule(String),
}

impl HasKind for SyncError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { .. } | Self::Walk(_) => ErrorKind::Transient,
            Self::RootEscape(_) => ErrorKind::Policy,
            Self::InvalidPath { .. } | Self::Rule(_) => ErrorKind::Permanent,
        }
    }
}

/// A normalised workspace-relative path: `/`-separated, no empty, `.` or `..` components, no
/// leading `/`, no backslash, no NUL, and no drive prefix (`C:`) in the first component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

fn invalid(path: &str, reason: &'static str) -> SyncError {
    SyncError::InvalidPath {
        path: path.chars().take(256).collect(),
        reason,
    }
}

impl RelPath {
    /// Validates `text`.
    ///
    /// # Errors
    /// [`SyncError::RootEscape`] for `..` or absolute paths, [`SyncError::InvalidPath`] otherwise.
    pub fn new(text: &str) -> Result<Self, SyncError> {
        if text.is_empty() {
            return Err(invalid(text, "empty"));
        }
        if text.contains('\0') {
            return Err(invalid(text, "NUL byte"));
        }
        if text.contains('\\') {
            return Err(invalid(text, "backslash (use '/')"));
        }
        if text.starts_with('/') {
            return Err(SyncError::RootEscape(text.chars().take(256).collect()));
        }
        let mut first = true;
        for component in text.split('/') {
            match component {
                "" => return Err(invalid(text, "empty component")),
                "." => return Err(invalid(text, "'.' component")),
                ".." => return Err(SyncError::RootEscape(text.chars().take(256).collect())),
                _ => {}
            }
            let mut chars = component.chars();
            if first
                && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && chars.next() == Some(':')
            {
                return Err(SyncError::RootEscape(text.chars().take(256).collect()));
            }
            first = false;
        }
        Ok(Self(text.to_owned()))
    }

    /// The path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `/`-separated components.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The parent directory (`None` at the top level).
    #[must_use]
    pub fn parent(&self) -> Option<&str> {
        self.0.rsplit_once('/').map(|(parent, _)| parent)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
