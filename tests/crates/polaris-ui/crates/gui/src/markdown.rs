// Unit tests of crates/polaris-ui/crates/gui/src/markdown.rs (`tests`); compiled into that module via #[path].
use super::{Block, blocks, diff_line_kind};

fn block(kind: &'static str, text: &str) -> Block {
    Block {
        kind,
        text: text.to_owned(),
    }
}

#[test]
fn replies_split_into_headings_items_paragraphs_and_code() {
    let reply = "## Plan\nThe **parser** is `LL(1)`.\nIt has two passes.\n\n1. Read tokens\n- Build the tree\n\n```rust\nfn main() {}\n```\nDone.";
    assert_eq!(
        blocks(reply),
        vec![
            block("h", "Plan"),
            block("p", "The parser is LL(1).\nIt has two passes."),
            block("li", "Read tokens"),
            block("li", "Build the tree"),
            block("code", "fn main() {}"),
            block("p", "Done."),
        ]
    );
}

#[test]
fn an_open_fence_while_streaming_is_still_code() {
    assert_eq!(
        blocks("Look:\n```\nlet x = 1;"),
        vec![block("p", "Look:"), block("code", "let x = 1;")]
    );
}

#[test]
fn diff_fences_become_diff_blocks_with_line_kinds() {
    let reply = "Change it?\n\n```diff\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same\n```";
    assert_eq!(
        blocks(reply),
        vec![
            block("p", "Change it?"),
            block("diff", "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same"),
        ]
    );
    let kinds: Vec<&str> = ["--- a/x", "+++ b/x", "@@ -1 +1 @@", "-old", "+new", " same"]
        .into_iter()
        .map(diff_line_kind)
        .collect();
    assert_eq!(kinds, ["head", "head", "hunk", "del", "add", "ctx"]);
}
