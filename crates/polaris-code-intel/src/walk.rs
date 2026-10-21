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
mod tests {
    use super::{Descend, walk_preorder};
    use crate::{Lang, ParserPool};

    #[test]
    fn tracked_depth_matches_native_cursor_on_mixed_trees() -> Result<(), Box<dyn std::error::Error>>
    {
        let cases = [
            (
                Lang::Rust,
                "mod m { pub fn f() { if true { let x = (1 + 2); } } }",
            ),
            (
                Lang::Python,
                "class C:\n    def f(self):\n        if True:\n            return [1, 2]\n",
            ),
            (
                Lang::Cpp,
                "namespace n { class C { int f() { if (true) { return 1; } } }; }",
            ),
            (
                Lang::Cuda,
                "__global__ void k(int* p) { if (p) { p[0] = 1; } }",
            ),
            (
                Lang::TypeScript,
                "export class C { f() { if (true) { return [1, 2]; } } }",
            ),
        ];
        let pool = ParserPool::new();
        for (lang, source) in cases {
            let parsed = pool.parse(lang, source.as_bytes(), None)?;
            for skip_at in [None, Some(2)] {
                let mut reference = Vec::new();
                let mut cursor = parsed.tree().walk();
                loop {
                    let depth = cursor.depth();
                    reference.push((cursor.node().id(), depth));
                    if skip_at != Some(depth) && cursor.goto_first_child() {
                        continue;
                    }
                    loop {
                        if cursor.goto_next_sibling() {
                            break;
                        }
                        if !cursor.goto_parent() {
                            break;
                        }
                    }
                    if cursor.depth() == 0 {
                        break;
                    }
                }
                let mut observed = Vec::new();
                walk_preorder(parsed.tree().root_node(), |node, depth| {
                    observed.push((node.id(), depth));
                    if skip_at == Some(depth) {
                        Descend::Skip
                    } else {
                        Descend::Into
                    }
                });
                assert_eq!(observed, reference, "{lang:?}, skip_at={skip_at:?}");
            }
        }
        Ok(())
    }
}

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
