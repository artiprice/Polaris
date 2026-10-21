//! Internal dependency edges from `cargo metadata`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::error::XtaskError;

/// Returns, for every workspace package, the set of workspace packages it depends on
/// (normal dependencies only).
///
/// # Errors
/// Returns [`XtaskError::Io`] when cargo cannot be spawned, [`XtaskError::Cargo`] when it
/// fails, or [`XtaskError::Json`] when its output cannot be parsed.
pub fn internal_edges(root: &Path) -> Result<BTreeMap<String, BTreeSet<String>>, XtaskError> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(XtaskError::Cargo {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    edges_from_metadata(&serde_json::from_slice(&output.stdout)?)
}

/// Extracts internal normal-dependency edges from parsed `cargo metadata` JSON.
///
/// # Errors
/// Returns [`XtaskError::Usage`] when the JSON lacks the `packages` array.
pub fn edges_from_metadata(meta: &Value) -> Result<BTreeMap<String, BTreeSet<String>>, XtaskError> {
    let packages = meta
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XtaskError::Usage("cargo metadata JSON has no `packages` array".to_owned())
        })?;
    let names: BTreeSet<String> = packages
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    let mut edges = BTreeMap::new();
    for package in packages {
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        let deps: BTreeSet<String> = match package.get("dependencies").and_then(Value::as_array) {
            Some(deps) => deps
                .iter()
                .filter(|d| d.get("kind").is_none_or(Value::is_null))
                .filter_map(|d| d.get("name").and_then(Value::as_str))
                .filter(|d| names.contains(*d))
                .map(str::to_owned)
                .collect(),
            None => BTreeSet::new(),
        };
        edges.insert(name.to_owned(), deps);
    }
    Ok(edges)
}
