//! The crate table (`@prime/tools/workspace.toml`): the single source of truth for packages.

use std::path::Path;

use serde::Deserialize;

use crate::error::XtaskError;

/// One package entry (`[[crate]]`) of the table. Unknown keys are ignored.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CrateEntry {
    /// Cargo package name.
    pub name: String,
    /// Path relative to the repository root, `/`-separated.
    pub path: String,
    /// Architectural layer (0 = sdk … 8 = tests/tooling).
    pub layer: u8,
    /// Package kind (`lib`, `facade`, `app`, `test`, `spike`, `xtask`).
    pub kind: String,
    /// Parent facade package, for nested crates.
    #[serde(default)]
    pub parent: Option<String>,
    /// Declared internal dependencies (package names).
    #[serde(default)]
    pub deps: Vec<String>,
    /// `true` when the package is hand-written and must never be regenerated.
    #[serde(default)]
    pub implemented: bool,
}

/// The parsed crate table.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CrateTable {
    /// All packages, in table order.
    #[serde(rename = "crate", default)]
    pub crates: Vec<CrateEntry>,
}

impl CrateTable {
    /// Parses the table from TOML text.
    ///
    /// # Errors
    /// Returns [`XtaskError::Table`] when the text is not valid TOML or an entry lacks a
    /// required key.
    pub fn from_toml_str(text: &str) -> Result<Self, XtaskError> {
        Ok(toml::from_str(text)?)
    }

    /// Reads and parses the table file.
    ///
    /// # Errors
    /// Returns [`XtaskError::Io`] when the file cannot be read, or [`XtaskError::Table`]
    /// when it cannot be parsed.
    pub fn load(path: &Path) -> Result<Self, XtaskError> {
        Self::from_toml_str(&std::fs::read_to_string(path)?)
    }

    /// Looks up a package by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CrateEntry> {
        self.crates.iter().find(|c| c.name == name)
    }
}
