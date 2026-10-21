//! The symbol graph: per-file definitions, references and outlines.

use crate::decl::{SymbolKind, declarations};
use crate::error::CodeIntelError;
use crate::lang::Lang;
use crate::parse::Parsed;
use crate::skeleton::{MAX_SKELETON_ITEMS, skeleton};
use crate::walk::{Descend, node_text, walk_preorder};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What the repo map needs to know about one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileSymbols {
    /// Names the file declares.
    pub defs: BTreeSet<String>,
    /// Identifier occurrences: name to number of uses.
    pub refs: BTreeMap<String, u32>,
    /// Skeleton lines (signatures, indented by nesting).
    pub outline: Vec<String>,
}

/// Definitions and references of a set of files, in a deterministic order.
///
/// Files are keyed by their path with `\` normalised to `/`, so the same repository produces the
/// same graph on every platform and in every insertion order.
#[derive(Debug, Clone, Default)]
pub struct SymbolGraph {
    files: BTreeMap<String, FileSymbols>,
}

impl SymbolGraph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of files in the graph.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Adds (or replaces) a file: its declarations, identifier uses and skeleton lines.
    ///
    /// # Errors
    /// [`CodeIntelError::TooLarge`] if the file's skeleton exceeds [`MAX_SKELETON_ITEMS`].
    pub fn add_file(
        &mut self,
        path: impl AsRef<Path>,
        parsed: &Parsed,
    ) -> Result<(), CodeIntelError> {
        let sk = skeleton(parsed)?;
        let defs = declarations(parsed, MAX_SKELETON_ITEMS)?
            .into_iter()
            .filter(|d| d.kind != SymbolKind::Impl)
            .map(|d| d.name)
            .collect();
        let entry = FileSymbols {
            defs,
            refs: identifier_uses(parsed),
            outline: sk.render().lines().map(str::to_owned).collect(),
        };
        self.files.insert(normalize_path(path.as_ref()), entry);
        Ok(())
    }

    pub(crate) fn files(&self) -> &BTreeMap<String, FileSymbols> {
        &self.files
    }
}

/// The graph key of a path: lossy UTF-8 with `\` replaced by `/`.
pub(crate) fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn identifier_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &["identifier", "type_identifier", "field_identifier"],
        Lang::Python => &["identifier"],
        Lang::Cpp | Lang::Cuda => &[
            "identifier",
            "type_identifier",
            "field_identifier",
            "namespace_identifier",
        ],
        Lang::TypeScript => &["identifier", "type_identifier", "property_identifier"],
    }
}

fn identifier_uses(p: &Parsed) -> BTreeMap<String, u32> {
    let kinds = identifier_kinds(p.lang());
    let src = p.source();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    walk_preorder(p.tree().root_node(), |node, _| {
        if node.child_count() == 0 && kinds.contains(&node.kind()) {
            let text = node_text(node, src);
            match counts.get_mut(text.as_ref()) {
                Some(n) => *n = n.saturating_add(1),
                None => {
                    counts.insert(text.into_owned(), 1);
                }
            }
        }
        Descend::Into
    });
    counts
}
