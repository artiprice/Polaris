//! Descriptor: specs/descriptors/codeintel_repo_map_ranking.json

use polaris_code_intel::{
    CodeIntelError, Lang, ParserPool, RepoMapBudget, SymbolGraph, rank_files, repo_map,
};
use polaris_sdk::ids::TokenCount;
use proptest::prelude::*;
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FILES: [(&str, &str); 4] = [
    ("src/core.rs", "pub fn core_util() {}\npub struct Hub;\n"),
    ("src/b.rs", "fn b() { core_util(); let _ = Hub; }\n"),
    ("src/c.rs", "fn c() { core_util(); }\n"),
    ("src/lonely.rs", "fn lonely() {}\n"),
];

fn graph(files: &[(&str, &str)]) -> Result<SymbolGraph, CodeIntelError> {
    let pool = ParserPool::new();
    let mut g = SymbolGraph::new();
    for (path, src) in files {
        g.add_file(path, &pool.parse(Lang::Rust, src.as_bytes(), None)?)?;
    }
    Ok(g)
}

fn budget(tokens: u32, focus: &[&str]) -> RepoMapBudget {
    RepoMapBudget {
        max_tokens: TokenCount::new(tokens),
        focus_files: focus.iter().map(PathBuf::from).collect(),
    }
}

fn rank_of(ranks: &[(PathBuf, f64)], path: &str) -> Result<f64, String> {
    ranks
        .iter()
        .find(|(p, _)| p == &PathBuf::from(path))
        .map(|(_, r)| *r)
        .ok_or_else(|| format!("{path} not ranked"))
}

#[test]
fn output_is_byte_identical_across_runs() -> TestResult {
    let forward = graph(&FILES)?;
    let mut reversed_files = FILES;
    reversed_files.reverse();
    let reversed = graph(&reversed_files)?;
    let b = budget(10_000, &[]);
    let first = repo_map(&forward, &b)?;
    for _ in 0..5 {
        assert_eq!(repo_map(&forward, &b)?.render(), first.render());
    }
    // Insertion order must not matter either.
    let other = repo_map(&reversed, &b)?;
    assert_eq!(other.render(), first.render());
    assert_eq!(other, first);
    Ok(())
}

#[test]
fn most_referenced_file_ranks_first() -> TestResult {
    let g = graph(&FILES)?;
    let map = repo_map(&g, &budget(10_000, &[]))?;
    let first = map.entries.first().ok_or("empty map")?;
    assert_eq!(first.path, PathBuf::from("src/core.rs"));
    let last = map.entries.last().ok_or("empty map")?;
    assert_eq!(last.path, PathBuf::from("src/lonely.rs"));
    assert!(first.outline.iter().any(|l| l == "pub fn core_util()"));
    Ok(())
}

#[test]
fn focus_file_boosts_rank() -> TestResult {
    let g = graph(&FILES)?;
    let plain = rank_files(&g, &[]);
    let focused = rank_files(&g, &[PathBuf::from("src/lonely.rs")]);
    assert!(rank_of(&focused, "src/lonely.rs")? > rank_of(&plain, "src/lonely.rs")?);
    let map = repo_map(&g, &budget(10_000, &["src/lonely.rs"]))?;
    let top = map.entries.first().ok_or("empty map")?;
    assert_eq!(top.path, PathBuf::from("src/lonely.rs"));
    Ok(())
}

#[test]
fn focus_boosts_the_files_it_depends_on() -> TestResult {
    let g = graph(&FILES)?;
    let plain = rank_files(&g, &[]);
    let focused = rank_files(&g, &[PathBuf::from("src/b.rs")]);
    let ratio = |ranks: &[(PathBuf, f64)]| -> Result<f64, String> {
        Ok(rank_of(ranks, "src/core.rs")? / rank_of(ranks, "src/lonely.rs")?)
    };
    // Relative to an unrelated file, the dependency of the focus file gains rank.
    assert!(ratio(&focused)? > ratio(&plain)?);
    Ok(())
}

#[test]
fn unknown_focus_files_are_ignored() -> TestResult {
    let g = graph(&FILES)?;
    assert_eq!(
        rank_files(&g, &[PathBuf::from("nope.rs")]),
        rank_files(&g, &[])
    );
    Ok(())
}

#[test]
fn budget_is_respected() -> TestResult {
    let g = graph(&FILES)?;
    let full = repo_map(&g, &budget(10_000, &[]))?;
    assert_eq!(full.entries.len(), FILES.len());
    for tokens in [0u32, 1, 8, 20, 30, 45] {
        let map = repo_map(&g, &budget(tokens, &[]))?;
        let rendered = map.render();
        assert!(map.token_estimate.get() <= tokens, "{tokens}");
        assert_eq!(map.token_estimate.get(), rendered.len().div_ceil(4) as u32);
        assert!(rendered.len().div_ceil(4) as u32 <= tokens);
    }
    let tiny = repo_map(&g, &budget(0, &[]))?;
    assert!(tiny.entries.is_empty());
    let some = repo_map(&g, &budget(20, &[]))?;
    assert!(!some.entries.is_empty() && some.entries.len() < FILES.len());
    let first = some.entries.first().ok_or("empty map")?;
    assert_eq!(
        first.path,
        PathBuf::from("src/core.rs"),
        "highest rank kept first"
    );
    Ok(())
}

#[test]
fn empty_graph_yields_empty_map() -> TestResult {
    let g = SymbolGraph::new();
    let map = repo_map(&g, &budget(1_000, &[]))?;
    assert!(map.entries.is_empty());
    assert_eq!(map.token_estimate, TokenCount::new(0));
    assert_eq!(map.render(), "");
    assert!(rank_files(&g, &[]).is_empty());
    Ok(())
}

#[test]
fn ties_break_by_path() -> TestResult {
    let g = graph(&[
        ("z.rs", "fn z() {}\n"),
        ("a.rs", "fn a() {}\n"),
        ("m.rs", "fn m() {}\n"),
    ])?;
    let order: Vec<_> = rank_files(&g, &[]).into_iter().map(|(p, _)| p).collect();
    assert_eq!(
        order,
        [
            PathBuf::from("a.rs"),
            PathBuf::from("m.rs"),
            PathBuf::from("z.rs")
        ]
    );
    Ok(())
}

#[test]
fn path_separators_are_normalised() -> TestResult {
    let a = graph(&[
        ("src\\core.rs", "pub fn core_util() {}\n"),
        ("src/b.rs", "fn b() { core_util(); }\n"),
    ])?;
    let b = graph(&[
        ("src/core.rs", "pub fn core_util() {}\n"),
        ("src/b.rs", "fn b() { core_util(); }\n"),
    ])?;
    let bud = budget(1_000, &[]);
    assert_eq!(repo_map(&a, &bud)?.render(), repo_map(&b, &bud)?.render());
    let focused = repo_map(&a, &budget(1_000, &["src/core.rs"]))?;
    let top = focused.entries.first().ok_or("empty map")?;
    assert_eq!(top.path, PathBuf::from("src/core.rs"));
    Ok(())
}

#[test]
fn re_adding_a_file_replaces_it() -> TestResult {
    let pool = ParserPool::new();
    let mut g = SymbolGraph::new();
    g.add_file(
        "a.rs",
        &pool.parse(Lang::Rust, b"fn old_name() {}\n", None)?,
    )?;
    g.add_file(
        "a.rs",
        &pool.parse(Lang::Rust, b"fn new_name() {}\n", None)?,
    )?;
    assert_eq!(g.file_count(), 1);
    let map = repo_map(&g, &budget(1_000, &[]))?;
    assert!(map.render().contains("new_name"));
    assert!(!map.render().contains("old_name"));
    Ok(())
}

#[test]
fn version_changes_with_content() -> TestResult {
    let a = repo_map(&graph(&FILES)?, &budget(10_000, &[]))?;
    let b = repo_map(&graph(&FILES)?, &budget(10_000, &[]))?;
    let c = repo_map(&graph(&FILES[..2])?, &budget(10_000, &[]))?;
    assert_eq!(a.version, b.version);
    assert_ne!(a.version, c.version);
    Ok(())
}

fn synthetic(n: usize, edges: &[(usize, usize)]) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            let calls: String = edges
                .iter()
                .filter(|(from, _)| from % n == i)
                .map(|(_, to)| format!("f{}();", to % n))
                .collect();
            (
                format!("p/m{i}.rs"),
                format!("pub fn f{i}() {{ {calls} }}\n"),
            )
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn ranks_are_finite_non_negative_and_sum_to_one(
        n in 1usize..12,
        edges in proptest::collection::vec((0usize..64, 0usize..64), 0..40),
        focus in proptest::collection::vec(0usize..16, 0..3),
    ) {
        let files = synthetic(n, &edges);
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
        let g = graph(&refs).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let focus: Vec<PathBuf> = focus.iter().map(|i| PathBuf::from(format!("p/m{i}.rs"))).collect();
        let ranks = rank_files(&g, &focus);
        prop_assert_eq!(ranks.len(), n);
        let mut sum = 0.0f64;
        for (_, r) in &ranks {
            prop_assert!(r.is_finite() && *r >= 0.0);
            sum += r;
        }
        prop_assert!((sum - 1.0).abs() < 1e-6, "sum = {}", sum);
        // Repeating the computation is bit-identical.
        prop_assert_eq!(&ranks, &rank_files(&g, &focus));
    }

    #[test]
    fn rendering_is_independent_of_insertion_order(
        n in 1usize..10,
        edges in proptest::collection::vec((0usize..64, 0usize..64), 0..30),
        rot in 0usize..10,
    ) {
        let mut files = synthetic(n, &edges);
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
        let a = graph(&refs).map_err(|e| TestCaseError::fail(e.to_string()))?;
        files.rotate_left(rot % n);
        let refs: Vec<(&str, &str)> = files.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
        let b = graph(&refs).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let bud = budget(100_000, &[]);
        let ma = repo_map(&a, &bud).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mb = repo_map(&b, &bud).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(ma.render(), mb.render());
    }
}
