// Unit tests of crates/polaris-code-intel/src/walk.rs (`tests`); compiled into that module via #[path].
use super::{Descend, walk_preorder};
use crate::{Lang, ParserPool};

#[test]
fn tracked_depth_matches_native_cursor_on_mixed_trees() -> Result<(), Box<dyn std::error::Error>> {
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
