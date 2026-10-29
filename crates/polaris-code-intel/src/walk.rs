//! Iterative tree traversal and small text helpers shared by the analyses.
//!
//! Every walk in this crate goes through here so that no analysis recurses on the (untrusted)
//! depth of a syntax tree.

use std::borrow::Cow;
use tree_sitter::Node;

/// What a visitor wants the walk to do with the children of the visited node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Descend {
    /// Visit the children as well.
    Into,
    /// Skip the whole subtree below this node.
    Skip,
}

/// Pre-order walk over `root` and everything below it; `visit` gets each node and its depth
/// relative to `root`.
pub(crate) fn walk_preorder<'t>(root: Node<'t>, mut visit: impl FnMut(Node<'t>, u32) -> Descend) {
    let mut cursor = root.walk();
    // TreeCursor::depth scans its ancestor stack. Track visible cursor moves instead so deep
    // inputs do not turn an otherwise linear walk into repeated ancestor scans.
    let mut depth = 0u32;
    loop {
        let descend = visit(cursor.node(), depth);
        if descend == Descend::Into && cursor.goto_first_child() {
            depth = depth.saturating_add(1);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
            depth = depth.saturating_sub(1);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-code-intel/src/walk.rs"]
mod tests;

/// The text of a node, lossily decoded; empty when the node range lies outside `src`.
pub(crate) fn node_text<'a>(node: Node<'_>, src: &'a [u8]) -> Cow<'a, str> {
    src.get(node.start_byte()..node.end_byte())
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
}

/// The text of the child stored under field `field`, if any.
pub(crate) fn field_text(node: Node<'_>, field: &str, src: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(n, src).into_owned())
}

/// The named children of a node, collected so they can be pushed onto an explicit stack.
pub(crate) fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
