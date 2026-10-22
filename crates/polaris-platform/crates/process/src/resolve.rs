//! Safe executable lookup for commands that will cross the native process boundary.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// A resolved executable and whether Windows would route it through a shell script handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramResolution {
    /// Canonical absolute executable path.
    pub path: PathBuf,
    /// True for `.bat` or `.cmd` targets, which require a caller-specific argument policy.
    pub is_batch_script: bool,
}

/// Failures while resolving an executable without consulting the current directory implicitly.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// The supplied name was empty or included a relative path component.
    #[error("program must be an absolute path or a bare executable name")]
    InvalidName,
    /// A same-name file in the current directory could shadow the PATH result.
    #[error("program is shadowed by a file in the current directory")]
    CurrentDirectoryShadow,
    /// PATH resolution did not find an executable.
    #[error("program was not found on PATH: {0}")]
    NotFound(String),
    /// The resolved path could not be canonicalized.
    #[error("program path could not be canonicalized: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolves an absolute executable path or a bare name using PATH while rejecting cwd shadowing.
pub fn resolve_program(program: &OsStr, cwd: &Path) -> Result<ProgramResolution, ResolveError> {
    if program.is_empty() {
        return Err(ResolveError::InvalidName);
    }
    let supplied = Path::new(program);
    let resolved = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        if supplied.components().count() != 1 {
            return Err(ResolveError::InvalidName);
        }
        let candidate = cwd.join(supplied);
        if candidate.exists() {
            return Err(ResolveError::CurrentDirectoryShadow);
        }
        which::which_in(program, std::env::var_os("PATH"), cwd)
            .map_err(|_| ResolveError::NotFound(program.to_string_lossy().into_owned()))?
    };
    let path = resolved.canonicalize()?;
    if !path.is_file() {
        return Err(ResolveError::NotFound(path.display().to_string()));
    }
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);
    Ok(ProgramResolution {
        path,
        is_batch_script: matches!(extension.as_deref(), Some("bat" | "cmd")),
    })
}
