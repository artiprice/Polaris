//! Lexical validation for resolver-supplied workspace-relative paths.

use std::fmt;

use super::manager::LeaseError;

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_COMPONENTS: usize = 512;

/// A canonical, case-folded workspace-relative key supplied by a trusted resolver.
///
/// Construction rejects ambiguous Windows path aliases but does not perform filesystem lookup.
/// The caller remains responsible for proving containment in the intended workspace.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseKey(Box<str>);

impl LeaseKey {
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
        Ok(Self(parts.join("/").into_boxed_str()))
    }

    /// Returns the canonical workspace-relative key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LeaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LeaseKey").field(&self.0).finish()
    }
}

impl fmt::Display for LeaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
