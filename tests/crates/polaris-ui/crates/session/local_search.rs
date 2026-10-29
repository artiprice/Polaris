//! P5 keyless local search: `/search-local` ranks workspace windows with BM25 and reports each hit
//! as `path:first-last` with its best lines; `$rag` carries the best passages into a turn.

use std::path::{Path, PathBuf};

use polaris_ui_session::commands::LocalCommand;
use polaris_ui_session::local_search::{rag_context, search, search_lines};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tests/fixtures")
}

/// Copy a sample repository into a temporary workspace, dropping the `.fixture` suffix.
fn materialize(name: &str) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    let root = fixtures().join(name);
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
            let relative = relative.strip_suffix(".fixture").unwrap_or(&relative);
            let destination = target.path().join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, destination)?;
        }
    }
    Ok(target)
}

#[test]
fn the_best_window_comes_first_with_its_lines() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    let hits = search(workspace.path(), "withdraw balance lock", 5)?;
    let first = hits.first().ok_or("a hit")?;
    assert_eq!(first.path, "src/account.rs");
    // The window spans withdraw (line 17) through the write under the second lock (line 22); the
    // snippet prefers lines that match more query words.
    assert!(first.first_line <= 17 && first.last_line >= 22, "{first:?}");
    assert!(
        first
            .snippet
            .iter()
            .any(|(line, text)| *line == 22 && text.contains("balance - amount")),
        "{first:?}"
    );
    // snake_case queries match identifiers and their parts.
    let hits = search(workspace.path(), "next id", 3)?;
    assert_eq!(
        hits.first().map(|hit| hit.path.as_str()),
        Some("src/ids.rs")
    );
    let lines = search_lines("next id", &hits);
    assert!(
        lines
            .first()
            .is_some_and(|line| line.contains("for \"next id\""))
    );
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with("1. src/ids.rs:1-")),
        "{lines:#?}"
    );
    Ok(())
}

#[test]
fn camel_case_parts_overlap_and_misses_are_empty() -> TestResult {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir_all(workspace.path().join("src"))?;
    std::fs::write(
        workspace.path().join("src/profile.ts"),
        "export function fetchUserProfile(id: string) {\n  return http.get(`/users/${id}`);\n}\n",
    )?;
    std::fs::write(
        workspace.path().join("notes.md"),
        "Nothing about accounts here.\n",
    )?;
    let hits = search(workspace.path(), "user profile", 5)?;
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits.first().map(|hit| hit.path.as_str()),
        Some("src/profile.ts")
    );
    assert!(search(workspace.path(), "kubernetes ingress", 5)?.is_empty());
    assert!(search(workspace.path(), "  ", 5)?.is_empty());
    let none = search_lines("kubernetes", &[]);
    assert!(none.iter().any(|line| line.starts_with("Nothing matched")));
    // Overlapping windows of one file are reported once.
    let long: String = (0..120)
        .map(|n| format!("let value_{n} = compute_total(input);\n"))
        .collect();
    std::fs::write(workspace.path().join("src/long.rs"), long)?;
    let hits = search(workspace.path(), "compute total", 10)?;
    let long_hits: Vec<_> = hits
        .iter()
        .filter(|hit| hit.path == "src/long.rs")
        .collect();
    for (a, b) in long_hits.iter().zip(long_hits.iter().skip(1)) {
        assert!(
            a.last_line < b.first_line || b.last_line < a.first_line,
            "{a:?} {b:?}"
        );
    }
    Ok(())
}

#[test]
fn rag_passages_name_their_source_lines() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    let context =
        rag_context(workspace.path(), "How does withdraw check the balance?")?.ok_or("passages")?;
    assert!(context.starts_with("Workspace passages for this question ($rag, lexical search)."));
    assert!(context.contains("--- src/account.rs:"), "{context}");
    assert!(context.contains("untrusted workspace data"));
    assert_eq!(rag_context(workspace.path(), "zzz qqq")?, None);
    Ok(())
}

#[test]
fn search_local_needs_a_query() {
    assert_eq!(
        LocalCommand::parse("/search-local \"withdraw balance\""),
        Some(LocalCommand::SearchLocal("withdraw balance".to_owned()))
    );
    assert_eq!(
        LocalCommand::parse("/search-local next_id"),
        Some(LocalCommand::SearchLocal("next_id".to_owned()))
    );
    assert_eq!(LocalCommand::parse("/search-local"), None);
    assert_eq!(LocalCommand::parse("/search-local   "), None);
    assert_eq!(
        LocalCommand::SearchLocal(String::new()).title(),
        "Local search"
    );
}

#[test]
fn a_changed_file_is_indexed_again() -> TestResult {
    let workspace = tempfile::tempdir()?;
    let file = workspace.path().join("notes.md");
    std::fs::write(&file, "alpha beta\n")?;
    assert!(search(workspace.path(), "gamma", 5)?.is_empty());
    assert_eq!(search(workspace.path(), "alpha", 5)?.len(), 1);
    // Different size: the cached index of the old content is not reused.
    std::fs::write(&file, "gamma delta epsilon\n")?;
    assert_eq!(search(workspace.path(), "gamma", 5)?.len(), 1);
    assert!(search(workspace.path(), "alpha", 5)?.is_empty());
    Ok(())
}
