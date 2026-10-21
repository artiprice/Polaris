//! `.polarisignore` (gitignore syntax) plus a non-overridable secret deny-list.

use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::path::{RelPath, SyncError};

/// Name of the per-workspace ignore file.
pub const POLARISIGNORE: &str = ".polarisignore";

/// Paths that never leave the machine, whatever `.polarisignore` says.
pub const DEFAULT_DENY: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/*.pfx",
    "**/*.p12",
    "**/.git/config",
    "**/.ssh",
    "**/.ssh/**",
    "**/id_rsa",
    "**/id_rsa.*",
    "**/id_ed25519",
    "**/id_ed25519.*",
    "**/id_ecdsa",
    "**/id_ecdsa.*",
    "**/.netrc",
    "**/.npmrc",
    "**/.pypirc",
];

/// Outcome of classifying one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// Synced.
    Included,
    /// Excluded by `.polarisignore`.
    Ignored,
    /// Excluded by the deny-list (reported, never overridable).
    Denied,
}

/// `.polarisignore` rules and the deny-list, evaluated deny-first.
#[derive(Debug, Clone)]
pub struct IgnoreRules {
    gitignore: Gitignore,
    deny: GlobSet,
}

impl IgnoreRules {
    /// Rules from `polarisignore` text (gitignore syntax) plus [`DEFAULT_DENY`].
    ///
    /// # Errors
    /// [`SyncError::Rule`] for an invalid pattern.
    pub fn new(root: &Path, polarisignore: Option<&str>) -> Result<Self, SyncError> {
        let mut builder = GitignoreBuilder::new(root);
        for line in polarisignore.unwrap_or_default().lines() {
            builder
                .add_line(None, line)
                .map_err(|e| SyncError::Rule(e.to_string()))?;
        }
        let gitignore = builder
            .build()
            .map_err(|e| SyncError::Rule(e.to_string()))?;
        let mut deny = GlobSetBuilder::new();
        for pattern in DEFAULT_DENY {
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| SyncError::Rule(e.to_string()))?;
            deny.add(glob);
        }
        let deny = deny.build().map_err(|e| SyncError::Rule(e.to_string()))?;
        Ok(Self { gitignore, deny })
    }

    /// Rules from `<root>/.polarisignore` (absent file = no ignore rules).
    ///
    /// # Errors
    /// [`SyncError::Io`] when the file exists but cannot be read; [`SyncError::Rule`].
    pub fn from_workspace(root: &Path) -> Result<Self, SyncError> {
        match std::fs::read_to_string(root.join(POLARISIGNORE)) {
            Ok(text) => Self::new(root, Some(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::new(root, None),
            Err(e) => Err(SyncError::Io {
                path: POLARISIGNORE.into(),
                reason: e.to_string(),
            }),
        }
    }

    /// Classifies a path; the deny-list wins over any `.polarisignore` negation.
    #[must_use]
    pub fn classify(&self, path: &RelPath, is_dir: bool) -> Exclusion {
        if self.deny.is_match(path.as_str()) {
            return Exclusion::Denied;
        }
        // `RelPath` is relative and drive-free, so the matcher's "under the root" assertion holds.
        if self
            .gitignore
            .matched_path_or_any_parents(path.as_str(), is_dir)
            .is_ignore()
        {
            return Exclusion::Ignored;
        }
        Exclusion::Included
    }
}
