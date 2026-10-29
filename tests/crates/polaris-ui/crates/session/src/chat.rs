// Unit tests of crates/polaris-ui/crates/session/src/chat.rs (`tests`); compiled into that module via #[path].
use super::{Exchange, MAX_GOAL_BYTES, compose_goal, cut, mention_hint};
use std::collections::VecDeque;

#[test]
fn mentions_name_workspace_paths_and_flag_escapes() -> Result<(), Box<dyn std::error::Error>> {
    let outer = tempfile::tempdir()?;
    let workspace = outer.path().join("ws");
    std::fs::create_dir_all(workspace.join("src"))?;
    std::fs::write(workspace.join("src/lib.rs"), "pub fn f() {}")?;
    std::fs::write(outer.path().join("secret.txt"), "x")?;
    let hint = mention_hint(
        &workspace,
        "compare @src/lib.rs with @src and @missing.rs, not @../secret.txt",
    )
    .ok_or("expected a hint")?;
    assert!(hint.contains("`src/lib.rs` (file)"), "{hint}");
    assert!(hint.contains("`src` (directory)"), "{hint}");
    assert!(hint.contains("`missing.rs` (not found)"), "{hint}");
    assert!(
        hint.contains("`../secret.txt` (outside the workspace"),
        "{hint}"
    );
    assert!(mention_hint(&workspace, "mail dev@example.com").is_none());
    Ok(())
}

#[test]
fn goal_without_history_is_the_prompt() {
    assert_eq!(compose_goal(&VecDeque::new(), "hello"), "hello");
}

#[test]
fn goal_keeps_newest_exchanges_within_budget() {
    let history: VecDeque<Exchange> = (0..8)
        .map(|index| Exchange {
            prompt: format!("question {index} {}", "q".repeat(1400)),
            answer: format!("answer {index} {}", "a".repeat(1400)),
        })
        .collect();
    let goal = compose_goal(&history, "now");
    assert!(goal.len() <= MAX_GOAL_BYTES);
    assert!(goal.ends_with("Current request:\nnow"));
    assert!(goal.contains("question 7"), "newest exchange is kept");
    assert!(!goal.contains("question 0"), "oldest exchange is dropped");
    let first = goal.find("question 6").unwrap_or(usize::MAX);
    let second = goal.find("question 7").unwrap_or(0);
    assert!(first < second, "kept exchanges stay in order");
}

#[test]
fn cut_respects_character_boundaries() {
    assert_eq!(cut("привет", 3), "п");
    assert_eq!(cut("abc", 10), "abc");
}
