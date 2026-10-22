//! Workspace-relative path validation and containment checks.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// Failures from workspace path validation or resolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// The supplied path is not a supported workspace-relative path.
    #[error("invalid workspace path")]
    Invalid(&'static str),
    /// The requested target does not exist.
    #[error("workspace path does not exist")]
    NotFound,
    /// The target is outside the workspace or crosses a reparse point.
    #[error("workspace path is outside the trusted workspace")]
    Escape,
    /// The target is on the default secret-file deny list.
    #[error("secret-bearing paths are denied")]
    SecretDenied,
    /// Filesystem metadata or canonicalization failed.
    #[error("workspace path could not be resolved")]
    Io,
}

/// A validated, normalized relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePath(String);

impl WorkspacePath {
    /// Validate a user path using Windows-compatible separators and names.
    ///
    /// Paths are normalized to `/`. Parent traversal, absolute/UNC/device paths, alternate data
    /// streams, reserved Windows device names, empty components, trailing dots/spaces, and NULs
    /// are rejected on every platform.
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value.is_empty() || value.chars().any(|character| character == '\0') {
            return Err(PathError::Invalid("empty path or NUL"));
        }
        if value.starts_with('/') || value.starts_with('\\') {
            return Err(PathError::Invalid("absolute or UNC path"));
        }
        let normalized = value.replace('\\', "/");
        if normalized.starts_with("//")
            || normalized.starts_with("?/")
            || normalized.starts_with("./")
        {
            return Err(PathError::Invalid("device or rooted path"));
        }
        let mut components = Vec::new();
        for component in normalized.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(PathError::Invalid("empty, dot, or parent component"));
            }
            if component.contains(':')
                || component.ends_with('.')
                || component.ends_with(' ')
                || component
                    .chars()
                    .any(|character| matches!(character, '<' | '>' | '"' | '|' | '?' | '*'))
            {
                return Err(PathError::Invalid(
                    "alternate stream or ambiguous Windows name",
                ));
            }
            if is_reserved_device_name(component) {
                return Err(PathError::Invalid("reserved Windows device name"));
            }
            components.push(component);
        }
        Ok(Self(components.join("/")))
    }

    /// Normalized `/`-separated path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Resolve an existing file or directory under a workspace root.
///
/// Resolution rejects lexical escapes, secret-bearing paths, symlinks and platform reparse
/// points. It canonicalizes the root and target and verifies component containment. The current
/// SDK provides only `&Path`, not a handle-relative workspace resolver: an attacker able to swap
/// filesystem components concurrently can race the metadata/canonicalize/open sequence. Callers
/// must supply a stable workspace; this function does not claim race-free junction handling.
pub fn resolve_in_workspace(root: &Path, relative: &str) -> Result<PathBuf, PathError> {
    let relative = WorkspacePath::parse(relative)?;
    if is_secret_path(&relative) {
        return Err(PathError::SecretDenied);
    }
    let canonical_root = fs::canonicalize(root).map_err(|_| PathError::Io)?;
    let mut candidate = canonical_root.clone();
    for component in relative.as_str().split('/') {
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PathError::NotFound
            } else {
                PathError::Io
            }
        })?;
        if is_reparse_point(&metadata) {
            return Err(PathError::Escape);
        }
    }
    let canonical_target = fs::canonicalize(&candidate).map_err(|_| PathError::Io)?;
    if !path_is_within(&canonical_root, &canonical_target) {
        return Err(PathError::Escape);
    }
    Ok(canonical_target)
}

fn path_is_within(root: &Path, target: &Path) -> bool {
    let mut root_components = root.components();
    let mut target_components = target.components();
    loop {
        match (root_components.next(), target_components.next()) {
            (None, _) => return true,
            (Some(root_component), Some(target_component))
                if component_eq(root_component, target_component) => {}
            _ => return false,
        }
    }
}

#[cfg(windows)]
fn component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    left.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
}

#[cfg(not(windows))]
fn component_eq(left: Component<'_>, right: Component<'_>) -> bool {
    left == right
}

fn is_secret_path(path: &WorkspacePath) -> bool {
    let components: Vec<String> = path
        .as_str()
        .split('/')
        .map(|component| component.to_ascii_lowercase())
        .collect();
    components.iter().any(|component| {
        component == ".env"
            || component.starts_with(".env.")
            || component.ends_with(".pem")
            || component.ends_with(".key")
    }) || components.windows(2).any(|pair| {
        pair.first().is_some_and(|first| first == ".git")
            && pair.get(1).is_some_and(|second| second == "config")
    })
}

fn is_reserved_device_name(component: &str) -> bool {
    let stem = match component.split('.').next() {
        Some(value) => value.to_ascii_uppercase(),
        None => return false,
    };
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM¹"
            | "COM²"
            | "COM³"
            | "LPT¹"
            | "LPT²"
            | "LPT³"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            suffix.len() == 1
                && suffix
                    .chars()
                    .all(|character| matches!(character, '1'..='9'))
        })
    })
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
