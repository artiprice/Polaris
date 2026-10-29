//! Descriptor: specs/descriptors/codeintel_mutation_metrics.json

use polaris_code_intel::{
    CodeIntelError, Lang, MetricsConfig, MutationMetrics, Parsed, ParserPool, SymbolKind,
    SymbolRef, cyclomatic_complexity, metrics, metrics_with,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn parse(lang: Lang, src: &str) -> Result<Parsed, CodeIntelError> {
    ParserPool::new().parse(lang, src.as_bytes(), None)
}

fn measure(lang: Lang, old: &str, new: &str) -> Result<MutationMetrics, CodeIntelError> {
    metrics(&parse(lang, old)?, &parse(lang, new)?)
}

fn sym(name: &str, kind: SymbolKind) -> SymbolRef {
    SymbolRef {
        name: name.to_owned(),
        kind,
    }
}

#[test]
fn identical_trees_zero_metrics() -> TestResult {
    let src = "fn f(x: i32) -> i32 { if x > 0 && x < 9 { x } else { 0 } }\nstruct S;\n";
    let m = measure(Lang::Rust, src, src)?;
    assert_eq!(m.churn_lines, 0);
    assert_eq!(m.cyclomatic_delta, 0);
    assert_eq!(m.tree_edit_distance, 0);
    assert!(m.tree_edit_distance_exact);
    assert!(m.changed_symbols.is_empty());
    Ok(())
}

#[test]
fn added_branch_raises_cyclomatic() -> TestResult {
    let cases = [
        (
            Lang::Rust,
            "fn f(x: i32) -> i32 { x }\n",
            "fn f(x: i32) -> i32 { if x > 0 { 1 } else { x } }\n",
        ),
        (
            Lang::Python,
            "def f(x):\n    return x\n",
            "def f(x):\n    if x > 0:\n        return 1\n    return x\n",
        ),
        (
            Lang::Cpp,
            "int f(int x) { return x; }\n",
            "int f(int x) { if (x > 0) return 1; return x; }\n",
        ),
        (
            Lang::Cuda,
            "__global__ void k(int* p) { p[0] = 1; }\n",
            "__global__ void k(int* p) { if (p[1]) p[0] = 1; }\n",
        ),
        (
            Lang::TypeScript,
            "function f(x: number) { return x; }\n",
            "function f(x: number) { if (x > 0) { return 1; } return x; }\n",
        ),
    ];
    for (lang, old, new) in cases {
        let m = measure(lang, old, new)?;
        assert_eq!(m.cyclomatic_delta, 1, "{lang:?}");
        let back = measure(lang, new, old)?;
        assert_eq!(back.cyclomatic_delta, -1, "{lang:?} reversed");
    }
    Ok(())
}

#[test]
fn cyclomatic_counts_each_decision_point() -> TestResult {
    let rust = "\
fn f(a: bool, b: bool, x: i32) -> i32 {
    if a && b || a { 1 } else if b { 2 } else { 3 };
    while a { break; }
    for _ in 0..3 {}
    loop { break; }
    match x { 1 => 1, 2 => 2, _ => 3 }
}
";
    // 1 base + if(1) + &&(1) + ||(1) + else-if(1) + while(1) + for(1) + loop(1) + 3 arms.
    assert_eq!(cyclomatic_complexity(&parse(Lang::Rust, rust)?), 11);

    let python = "\
def f(a, b, xs):
    if a and b or a:
        pass
    elif b:
        pass
    while a:
        pass
    for x in xs:
        pass
    try:
        pass
    except ValueError:
        pass
    return [i for i in xs if i] if a else 0
";
    // 1 + if + and + or + elif + while + for + except + comprehension(for + if) + ternary.
    assert_eq!(cyclomatic_complexity(&parse(Lang::Python, python)?), 11);

    let cpp = "\
int f(int a, int b) {
    if (a && b) return 1;
    for (int i = 0; i < 3; i++) {}
    switch (a) { case 1: break; case 2: break; default: break; }
    return a ? 1 : 2;
}
";
    // 1 + if + && + for + 2 non-default cases + ternary.
    assert_eq!(cyclomatic_complexity(&parse(Lang::Cpp, cpp)?), 7);

    let ts = "\
function f(a: boolean, b?: number) {
    if (a && b) { return 1; }
    for (const x of [1]) {}
    switch (b) { case 1: break; default: break; }
    try { } catch (e) { }
    return a ? (b ?? 1) : 2;
}
";
    // 1 + if + && + for + case + catch + ternary + ??.
    assert_eq!(cyclomatic_complexity(&parse(Lang::TypeScript, ts)?), 8);
    Ok(())
}

#[test]
fn churn_counts_added_plus_deleted() -> TestResult {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nLINE2\nline3\nline4\n";
    // `line2` deleted, `LINE2` added, `line4` added.
    let m = measure(Lang::Python, old, new)?;
    assert_eq!(m.churn_lines, 3);
    let back = measure(Lang::Python, new, old)?;
    assert_eq!(back.churn_lines, 3);
    Ok(())
}

#[test]
fn churn_sees_line_ending_changes() -> TestResult {
    let m = measure(
        Lang::Rust,
        "fn a() {}\nfn b() {}\n",
        "fn a() {}\r\nfn b() {}\r\n",
    )?;
    assert_eq!(m.churn_lines, 4);
    Ok(())
}

#[test]
fn ted_rename_costs_one_and_insert_costs_the_subtree_size() -> TestResult {
    let base = "fn f() { let a = 1; }\n";
    let renamed = measure(Lang::Rust, base, "fn f() { let b = 1; }\n")?;
    assert_eq!(renamed.tree_edit_distance, 1);
    // `let c = 2;` adds let_declaration + identifier + integer_literal.
    let grown = measure(Lang::Rust, base, "fn f() { let a = 1; let c = 2; }\n")?;
    assert_eq!(grown.tree_edit_distance, 3);
    assert!(renamed.tree_edit_distance_exact && grown.tree_edit_distance_exact);
    Ok(())
}

#[test]
fn ted_ignores_comments_and_layout() -> TestResult {
    let m = measure(
        Lang::Rust,
        "fn f() { let a = 1; }\n",
        "// note\nfn f() {\n    let a = 1; // why\n}\n",
    )?;
    assert_eq!(m.tree_edit_distance, 0);
    assert!(m.churn_lines > 0);
    Ok(())
}

#[test]
fn ted_fallback_above_node_cap() -> TestResult {
    let old = parse(Lang::Rust, "fn f() { let a = 1; let b = 2; }\n")?;
    let new = parse(Lang::Rust, "fn f() { let a = 1; let c = 3; let d = 4; }\n")?;
    let exact = metrics(&old, &new)?;
    assert!(exact.tree_edit_distance_exact);
    let capped = metrics_with(&old, &new, &MetricsConfig { ted_node_cap: 4 })?;
    assert!(!capped.tree_edit_distance_exact);
    assert!(capped.tree_edit_distance > 0);
    assert!(
        capped.tree_edit_distance <= exact.tree_edit_distance,
        "lower bound"
    );
    // The fallback only affects the tree-edit distance.
    assert_eq!(capped.churn_lines, exact.churn_lines);
    assert_eq!(capped.cyclomatic_delta, exact.cyclomatic_delta);
    assert_eq!(capped.changed_symbols, exact.changed_symbols);
    Ok(())
}

#[test]
fn changed_symbols_lists_edited_added_and_removed_declarations() -> TestResult {
    let old = "\
fn keep() -> i32 { 1 }
fn edit() -> i32 { 2 }
fn gone() {}
struct Foo;
impl Foo {
    fn steady(&self) {}
    fn tweak(&self) -> u8 { 1 }
}
";
    let new = "\
fn keep() -> i32 { 1 }
fn edit() -> i32 { 22 }
fn fresh() {}
struct Foo;
impl Foo {
    fn steady(&self) {}
    fn tweak(&self) -> u8 { 2 }
}
";
    let m = measure(Lang::Rust, old, new)?;
    assert_eq!(
        m.changed_symbols,
        [
            sym("Foo::tweak", SymbolKind::Method),
            sym("edit", SymbolKind::Function),
            sym("fresh", SymbolKind::Function),
            sym("gone", SymbolKind::Function),
        ]
    );
    Ok(())
}

#[test]
fn changed_symbols_ignores_whitespace_outside_declarations() -> TestResult {
    let m = measure(
        Lang::Rust,
        "fn a() {}\nfn b() {}\n",
        "fn a() {}\n\n\n// spacer\nfn b() {}\n",
    )?;
    assert!(m.changed_symbols.is_empty());
    Ok(())
}

#[test]
fn comparing_different_languages_is_an_error() -> TestResult {
    let rs = parse(Lang::Rust, "fn f() {}")?;
    let py = parse(Lang::Python, "def f(): pass")?;
    assert_eq!(
        metrics(&rs, &py).err(),
        Some(CodeIntelError::LanguageMismatch {
            old: Lang::Rust,
            new: Lang::Python
        })
    );
    Ok(())
}

#[test]
fn hostile_nesting_is_measured_without_overflow() -> TestResult {
    let depth = 20_000;
    let make = |inner: &str| {
        format!(
            "fn f() {{ {}{inner}{} }}",
            "(".repeat(depth),
            ")".repeat(depth)
        )
    };
    let old = parse(Lang::Rust, &make("1"))?;
    let new = parse(Lang::Rust, &make("2"))?;
    let m = metrics(&old, &new)?;
    assert!(
        !m.tree_edit_distance_exact,
        "deep trees use the capped path"
    );
    assert_eq!(m.churn_lines, 2);
    Ok(())
}

fn rust_from(lines: &[(u8, u8)]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, (cond, val))| {
            if cond % 2 == 0 {
                format!("fn f{i}(x: u8) -> u8 {{ if x > {val} {{ 1 }} else {{ 0 }} }}\n")
            } else {
                format!("fn f{i}(x: u8) -> u8 {{ x + {val} }}\n")
            }
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn metrics_of_identical_inputs_are_zero(spec in proptest::collection::vec((any::<u8>(), any::<u8>()), 0..8)) {
        let src = rust_from(&spec);
        let m = measure(Lang::Rust, &src, &src).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(m.churn_lines, 0);
        prop_assert_eq!(m.cyclomatic_delta, 0);
        prop_assert_eq!(m.tree_edit_distance, 0);
        prop_assert!(m.changed_symbols.is_empty());
    }

    #[test]
    fn churn_and_tree_edit_distance_are_symmetric(
        a in proptest::collection::vec((any::<u8>(), any::<u8>()), 0..6),
        b in proptest::collection::vec((any::<u8>(), any::<u8>()), 0..6),
    ) {
        let (sa, sb) = (rust_from(&a), rust_from(&b));
        let ab = measure(Lang::Rust, &sa, &sb).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let ba = measure(Lang::Rust, &sb, &sa).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(ab.churn_lines, ba.churn_lines);
        prop_assert_eq!(ab.tree_edit_distance, ba.tree_edit_distance);
        prop_assert_eq!(ab.cyclomatic_delta, -ba.cyclomatic_delta);
        prop_assert_eq!(ab.changed_symbols.len(), ba.changed_symbols.len());
    }

    #[test]
    fn capped_distance_is_a_lower_bound_of_the_exact_one(
        a in proptest::collection::vec((any::<u8>(), any::<u8>()), 0..5),
        b in proptest::collection::vec((any::<u8>(), any::<u8>()), 0..5),
    ) {
        let pa = parse(Lang::Rust, &rust_from(&a)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let pb = parse(Lang::Rust, &rust_from(&b)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let exact = metrics(&pa, &pb).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let capped = metrics_with(&pa, &pb, &MetricsConfig { ted_node_cap: 0 }).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(exact.tree_edit_distance_exact);
        prop_assert!(capped.tree_edit_distance <= exact.tree_edit_distance);
    }
}
