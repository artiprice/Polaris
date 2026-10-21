//! Layer and edge rules (`cargo xtask check-layers`).
//!
//! Rule (PLAN.md §8): an edge must point strictly downward (`dep.layer < crate.layer`),
//! except facade -> own child and sibling -> sibling (same parent), which may stay on the
//! same layer (`<=`). A child never depends on its own parent. Manifest edges must equal
//! the table's declared edges.

use std::collections::{BTreeMap, BTreeSet};

use crate::table::{CrateEntry, CrateTable};

/// A violation of the layer model or a mismatch between manifests and the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerViolation {
    /// A child crate depends on its own parent facade.
    ChildToParent {
        /// The child package.
        child: String,
        /// Its parent facade.
        parent: String,
    },
    /// An edge does not point to a lower layer as the rule requires.
    NotDownward {
        /// Depending package.
        from: String,
        /// Dependency.
        to: String,
        /// Layer of `from`.
        from_layer: u8,
        /// Layer of `to`.
        to_layer: u8,
    },
    /// Manifest edges differ from the table's declared edges.
    EdgeMismatch {
        /// The package whose edges differ.
        package: String,
        /// Declared in the table but missing from the manifest.
        missing: BTreeSet<String>,
        /// Present in the manifest but not declared in the table.
        extra: BTreeSet<String>,
    },
    /// A package or dependency name is unknown to the table or to the manifests.
    Unknown(String),
}

/// Evaluates one declared edge `src -> dst` against the layer rule.
fn edge_violation(src: &CrateEntry, dst: &CrateEntry) -> Option<LayerViolation> {
    let not_downward = || LayerViolation::NotDownward {
        from: src.name.clone(),
        to: dst.name.clone(),
        from_layer: src.layer,
        to_layer: dst.layer,
    };
    if dst.parent.as_deref() == Some(src.name.as_str()) {
        return (dst.layer > src.layer).then(not_downward);
    }
    if src.parent.as_deref() == Some(dst.name.as_str()) {
        return Some(LayerViolation::ChildToParent {
            child: src.name.clone(),
            parent: dst.name.clone(),
        });
    }
    if src.parent.is_some() && src.parent == dst.parent {
        return (dst.layer > src.layer).then(not_downward);
    }
    (dst.layer >= src.layer).then(not_downward)
}

/// Checks `actual` (internal edges per package, e.g. from `cargo metadata`) against the table.
#[must_use]
pub fn check_layers(
    table: &CrateTable,
    actual: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<LayerViolation> {
    let mut violations = Vec::new();
    for src in &table.crates {
        let declared: BTreeSet<String> = src.deps.iter().cloned().collect();
        match actual.get(&src.name) {
            Some(found) if found != &declared => violations.push(LayerViolation::EdgeMismatch {
                package: src.name.clone(),
                missing: declared.difference(found).cloned().collect(),
                extra: found.difference(&declared).cloned().collect(),
            }),
            Some(_) => {}
            None => violations.push(LayerViolation::Unknown(src.name.clone())),
        }
        for dep in &declared {
            match table.get(dep) {
                Some(dst) => violations.extend(edge_violation(src, dst)),
                None => violations.push(LayerViolation::Unknown(dep.clone())),
            }
        }
    }
    violations
}
