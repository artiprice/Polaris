//! Source-policy checks (`cargo xtask check-witness`).
//!
//! `Command<Verified>` can only be built through `Command::verify(ApprovalWitness)`, but
//! `ApprovalWitness::new` is public (Rust cannot restrict a constructor to one other crate). This
//! check enforces the rule instead: only `polaris-sdk` (definition, docs, tests) and
//! `polaris-kernel-verifier` may construct witnesses.

use std::fs;
use std::path::Path;

use crate::error::XtaskError;

const NEEDLE: &str = "ApprovalWitness::new(";
const ALLOWED: &[&str] = &[
    "crates/polaris-sdk/",
    "crates/polaris-kernel/crates/verifier/",
];
const SCANNED: &[&str] = &["crates", "apps"];

fn scan(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), XtaskError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{rel}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if name != "target" {
                scan(&entry.path(), &child, out)?;
            }
        } else if kind.is_file()
            && name.ends_with(".rs")
            && !ALLOWED.iter().any(|a| child.starts_with(a))
        {
            let text = fs::read_to_string(entry.path())?;
            for (i, line) in text.lines().enumerate() {
                if line.contains(NEEDLE) {
                    out.push(format!("{child}:{}", i.saturating_add(1)));
                }
            }
        }
    }
    Ok(())
}

/// Scans `crates/` and `apps/` for `ApprovalWitness::new(` call sites outside the crates allowed
/// to construct approval witnesses (`target/` directories are skipped).
///
/// # Errors
/// Returns [`XtaskError::Io`] when a directory or file cannot be read.
pub fn check_witness(root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut violations = Vec::new();
    for top in SCANNED {
        let dir = root.join(top);
        if dir.is_dir() {
            scan(&dir, top, &mut violations)?;
        }
    }
    violations.sort();
    Ok(violations)
}
