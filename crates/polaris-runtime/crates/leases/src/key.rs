//! Lexical validation for resolver-supplied workspace-relative paths.

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use super::manager::LeaseError;

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 512;

/// A canonical, case-folded workspace-relative key supplied by a trusted resolver.
///
/// Construction rejects ambiguous Windows path aliases but does not perform filesystem lookup.
/// The caller remains responsible for proving containment in the intended workspace.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeaseKey {
    path: Box<str>,
    hash: u64,
}

impl Hash for LeaseKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Canonicalization computes this once. The enclosing map still applies its
        // independently seeded hasher; path equality resolves any fingerprint collision.
        state.write_u64(self.hash);
    }
}

impl LeaseKey {
    /// Resolve a filesystem path under an existing workspace root before creating a key.
    /// A missing final filename is allowed only when its canonical parent already exists.
    /// Ambiguous 8.3 aliases are rejected rather than creating a second ownership identity.
    /// # Errors
    /// [`LeaseError::InvalidPath`] for traversal, escape, unsupported encoding or resolution
    /// failure. Resolution does not authorize a later filesystem mutation: callers must retain
    /// their containment checks at write time against concurrent symlink changes.
    pub fn from_path(root: &std::path::Path, path: &std::path::Path) -> Result<Self, LeaseError> {
        use std::path::Component;
        let text = path.to_str().ok_or(LeaseError::InvalidPath)?;
        if text.as_bytes().get(1) == Some(&b':') && !path.is_absolute() {
            return Err(LeaseError::InvalidPath);
        }
        if text.starts_with(r"\\")
            || text.contains('~')
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(LeaseError::InvalidPath);
        }
        let root = std::fs::canonicalize(root).map_err(|_| LeaseError::InvalidPath)?;
        if !root.is_dir() {
            return Err(LeaseError::InvalidPath);
        }
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let canonical = match std::fs::canonicalize(&target) {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A dangling symlink is an existing alias, not an absent new filename.
                if std::fs::symlink_metadata(&target).is_ok() {
                    return Err(LeaseError::InvalidPath);
                }
                let parent = target.parent().ok_or(LeaseError::InvalidPath)?;
                let name = target.file_name().ok_or(LeaseError::InvalidPath)?;
                std::fs::canonicalize(parent)
                    .map_err(|_| LeaseError::InvalidPath)?
                    .join(name)
            }
            Err(_) => return Err(LeaseError::InvalidPath),
        };
        let relative = canonical
            .strip_prefix(&root)
            .map_err(|_| LeaseError::InvalidPath)?;
        Self::from_resolved_relative(relative.to_str().ok_or(LeaseError::InvalidPath)?)
    }

    /// Canonicalizes a resolver-supplied relative path using conservative Windows rules.
    ///
    /// Separators become `/`, Unicode lowercase is used for conservative case aliasing, and `.` components are removed. Absolute,
    /// UNC, device, alternate-stream, parent-traversal, trailing-dot/space, and control-character
    /// forms are rejected. Symlinks and short-name aliases must be resolved or rejected upstream.
    ///
    /// # Errors
    /// Returns [`LeaseError::InvalidPath`] if the string is not an unambiguous relative key.
    pub fn from_resolved_relative(path: &str) -> Result<Self, LeaseError> {
        if path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path.starts_with('/')
            || path.starts_with('\\')
        {
            return Err(LeaseError::InvalidPath);
        }
        if path.as_bytes().get(1) == Some(&b':') {
            return Err(LeaseError::InvalidPath);
        }

        let mut parts: Vec<String> = Vec::new();
        for raw in path.split(['/', '\\']) {
            if raw.is_empty() || raw == "." {
                continue;
            }
            if raw == ".." || (raw.ends_with('.') || raw.ends_with(' ')) || raw.contains(':') {
                return Err(LeaseError::InvalidPath);
            }
            if raw
                .chars()
                .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
            {
                return Err(LeaseError::InvalidPath);
            }
            if parts.len() >= MAX_PATH_COMPONENTS {
                return Err(LeaseError::LimitExceeded);
            }
            let folded = raw.to_lowercase();
            if is_reserved_device(&folded) {
                return Err(LeaseError::InvalidPath);
            }
            parts.push(folded);
        }
        if parts.is_empty() {
            return Err(LeaseError::InvalidPath);
        }
        let path = parts.join("/").into_boxed_str();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        Ok(Self {
            path,
            hash: hasher.finish(),
        })
    }

    /// Returns the canonical workspace-relative key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.path
    }
}

impl fmt::Debug for LeaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LeaseKey").field(&self.path).finish()
    }
}

impl fmt::Display for LeaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)
    }
}

fn is_reserved_device(component: &str) -> bool {
    let base = match component.split('.').next() {
        Some(value) => value.trim_end_matches(' ').to_ascii_uppercase(),
        None => String::new(),
    };
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix)
            .and_then(|suffix| suffix.parse::<u8>().ok())
            .is_some_and(|number| (1..=9).contains(&number))
    })
}
