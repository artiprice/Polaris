//! Workspace member consistency (`cargo xtask check-members`).
//!
//! The root manifest's member globs are `apps/*`, `crates/*`, `crates/*/crates/*`,
//! `tests`, `tests/spike/*` and `xtask` (one nesting level only). Cargo fails the whole
//! workspace when a glob matches a directory without `Cargo.toml`, so this check runs
//! before any build.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::error::XtaskError;
use crate::table::CrateTable;

/// A disagreement between the member globs, the filesystem and the crate table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberViolation {
    /// A directory matched by a member glob has no `Cargo.toml` (relative path).
    MissingManifest(String),
    /// A regular file sits directly at a glob level (relative path).
    StrayFile(String),
    /// A table package's directory does not exist (package name).
    NotOnDisk(String),
    /// A matched member directory is not in the table (relative path).
    NotInTable(String),
}

/// Fixed (non-glob) members.
const FIXED_MEMBERS: &[&str] = &["tests", "xtask"];

/// Directories whose immediate subdirectories are members (`<dir>/*`).
fn glob_parents(root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut parents = vec![
        "apps".to_owned(),
        "crates".to_owned(),
        "tests/spike".to_owned(),
    ];
    for top in subdirs(root, "crates")? {
        let nested = format!("{top}/crates");
        if root.join(&nested).is_dir() {
            parents.push(nested);
        }
    }
    Ok(parents)
}

/// Relative paths (`/`-separated) of the immediate subdirectories of `rel`.
fn subdirs(root: &Path, rel: &str) -> Result<Vec<String>, XtaskError> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            out.push(format!("{rel}/{}", entry.file_name().to_string_lossy()));
        }
    }
    out.sort();
    Ok(out)
}

/// Relative paths of regular files directly inside `rel`.
fn files_in(root: &Path, rel: &str) -> Result<Vec<String>, XtaskError> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            out.push(format!("{rel}/{}", entry.file_name().to_string_lossy()));
        }
    }
    out.sort();
    Ok(out)
}

/// Checks the member globs against the filesystem and the crate table.
///
/// # Errors
/// Returns [`XtaskError::Io`] when a directory cannot be read.
pub fn check_members(root: &Path, table: &CrateTable) -> Result<Vec<MemberViolation>, XtaskError> {
    let mut violations = Vec::new();
    let mut matched = BTreeSet::new();

    let parents = glob_parents(root)?;
    let mut candidates: Vec<String> = FIXED_MEMBERS.iter().map(|m| (*m).to_owned()).collect();
    for parent in &parents {
        candidates.extend(subdirs(root, parent)?);
        for file in files_in(root, parent)? {
            violations.push(MemberViolation::StrayFile(file));
        }
    }
    for rel in candidates {
        let dir = root.join(&rel);
        if !dir.is_dir() {
            continue;
        }
        if dir.join("Cargo.toml").is_file() {
            matched.insert(rel);
        } else {
            violations.push(MemberViolation::MissingManifest(rel));
        }
    }

    let table_paths: BTreeSet<String> = table
        .crates
        .iter()
        .map(|c| c.path.replace('\\', "/"))
        .collect();
    for entry in &table.crates {
        if !root.join(&entry.path).is_dir() {
            violations.push(MemberViolation::NotOnDisk(entry.name.clone()));
        }
    }
    for rel in &matched {
        if !table_paths.contains(rel) {
            violations.push(MemberViolation::NotInTable(rel.clone()));
        }
    }
    Ok(violations)
}
