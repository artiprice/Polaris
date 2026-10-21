//! Descriptor: specs/descriptors/codeintel_symbol_merge.json

use polaris_code_intel::{
    CodeIntelError, Lang, MergeOutcome, Parsed, ParserPool, SymbolKind, SymbolRef, merge3,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn parse(lang: Lang, src: &str) -> Result<Parsed, CodeIntelError> {
    ParserPool::new().parse(lang, src.as_bytes(), None)
}

fn merge(lang: Lang, base: &str, ours: &str, theirs: &str) -> Result<MergeOutcome, CodeIntelError> {
    merge3(
        &parse(lang, base)?,
        &parse(lang, ours)?,
        &parse(lang, theirs)?,
    )
}

fn merged_text(outcome: MergeOutcome) -> Result<String, Box<dyn std::error::Error>> {
    match outcome {
        MergeOutcome::Merged(bytes) => Ok(String::from_utf8(bytes)?),
        MergeOutcome::Conflict { symbols } => {
            Err(format!("unexpected conflict: {symbols:?}").into())
        }
    }
}

fn conflict_symbols(outcome: MergeOutcome) -> Result<Vec<SymbolRef>, Box<dyn std::error::Error>> {
    match outcome {
        MergeOutcome::Conflict { symbols } => Ok(symbols),
        MergeOutcome::Merged(bytes) => Err(format!(
            "expected a conflict, got merged bytes: {}",
            String::from_utf8_lossy(&bytes)
        )
        .into()),
    }
}

fn func(name: &str) -> SymbolRef {
    SymbolRef {
        name: name.to_owned(),
        kind: SymbolKind::Function,
    }
}

const BASE: &str = "fn a() -> i32 {\n    1\n}\n\nfn b() -> i32 {\n    2\n}\n";

#[test]
fn orthogonal_functions_merge_cleanly() -> TestResult {
    let ours = BASE.replace("    1\n", "    10\n");
    let theirs = BASE.replace("    2\n", "    20\n");
    let out = merged_text(merge(Lang::Rust, BASE, &ours, &theirs)?)?;
    assert_eq!(
        out,
        "fn a() -> i32 {\n    10\n}\n\nfn b() -> i32 {\n    20\n}\n"
    );
    Ok(())
}

#[test]
fn same_function_edited_conflicts() -> TestResult {
    let ours = BASE.replace("    1\n", "    10\n");
    let theirs = BASE.replace("    1\n", "    11\n");
    let symbols = conflict_symbols(merge(Lang::Rust, BASE, &ours, &theirs)?)?;
    assert_eq!(symbols, [func("a")]);
    Ok(())
}

#[test]
fn conflict_never_returns_merged_bytes() -> TestResult {
    // `a` conflicts while `b` merges cleanly: the whole outcome is still a conflict.
    let ours = BASE
        .replace("    1\n", "    10\n")
        .replace("    2\n", "    20\n");
    let theirs = BASE.replace("    1\n", "    11\n");
    let outcome = merge(Lang::Rust, BASE, &ours, &theirs)?;
    assert!(matches!(outcome, MergeOutcome::Conflict { .. }));
    assert_eq!(conflict_symbols(outcome)?, [func("a")]);
    Ok(())
}

#[test]
fn adjacent_one_sided_changes_are_not_reported_as_conflicts() -> TestResult {
    let base = format!("{BASE}\nfn c() -> i32 {{\n    3\n}}\n");
    let wide = base
        .replace("    1\n", "    10\n")
        .replace("    2\n", "    20\n")
        .replace("    3\n", "    30\n");
    for (needle, replacement, name) in [
        ("    1\n", "    11\n", "a"),
        ("    2\n", "    21\n", "b"),
        ("    3\n", "    31\n", "c"),
    ] {
        let narrow = base.replace(needle, replacement);
        for (ours, theirs) in [(&wide, &narrow), (&narrow, &wide)] {
            assert_eq!(
                conflict_symbols(merge(Lang::Rust, &base, ours, theirs)?)?,
                [func(name)]
            );
        }
    }
    Ok(())
}

#[test]
fn eol_style_preserved() -> TestResult {
    let crlf = |s: &str| s.replace('\n', "\r\n");
    let base = crlf(BASE);
    let ours = crlf(&BASE.replace("    1\n", "    10\n"));
    let theirs = crlf(&BASE.replace("    2\n", "    20\n"));
    let out = merged_text(merge(Lang::Rust, &base, &ours, &theirs)?)?;
    assert_eq!(
        out,
        crlf("fn a() -> i32 {\n    10\n}\n\nfn b() -> i32 {\n    20\n}\n")
    );
    assert!(
        !out.replace("\r\n", "").contains('\n'),
        "no bare LF introduced"
    );
    // An added function must borrow the CRLF separator, not invent LF.
    let theirs_add = format!("{base}\r\nfn c() {{}}\r\n");
    let out = merged_text(merge(Lang::Rust, &base, &ours, &theirs_add)?)?;
    assert!(!out.replace("\r\n", "").contains('\n'));
    assert!(out.contains("10") && out.contains("fn c()"));
    Ok(())
}

#[test]
fn utf8_bom_is_preserved() -> TestResult {
    let with_bom = |s: &str| format!("\u{feff}{s}");
    let out = merged_text(merge(
        Lang::Rust,
        &with_bom(BASE),
        &with_bom(&BASE.replace("    1\n", "    10\n")),
        &with_bom(&BASE.replace("    2\n", "    20\n")),
    )?)?;
    assert!(out.starts_with('\u{feff}'));
    assert!(out.contains("10") && out.contains("20"));
    Ok(())
}

#[test]
fn both_sides_adding_different_functions_keeps_both() -> TestResult {
    let ours = format!("{BASE}\nfn c() {{}}\n");
    let theirs = format!("{BASE}\nfn d() {{}}\n");
    let out = merged_text(merge(Lang::Rust, BASE, &ours, &theirs)?)?;
    assert_eq!(out, format!("{BASE}\nfn c() {{}}\n\nfn d() {{}}\n"));
    Ok(())
}

#[test]
fn both_sides_adding_the_same_name_differently_conflicts() -> TestResult {
    let ours = format!("{BASE}\nfn c() {{ 1 }}\n");
    let theirs = format!("{BASE}\nfn c() {{ 2 }}\n");
    let symbols = conflict_symbols(merge(Lang::Rust, BASE, &ours, &theirs)?)?;
    assert_eq!(symbols, [func("c")]);
    Ok(())
}

#[test]
fn identical_edits_on_both_sides_merge() -> TestResult {
    let both = BASE.replace("    1\n", "    10\n");
    let out = merged_text(merge(Lang::Rust, BASE, &both, &both)?)?;
    assert_eq!(out, both);
    Ok(())
}

#[test]
fn deleting_an_untouched_function_succeeds_but_deleting_an_edited_one_conflicts() -> TestResult {
    let without_b = "fn a() -> i32 {\n    1\n}\n";
    let edit_b = BASE.replace("    2\n", "    20\n");
    // Ours deletes b, theirs edits b: conflict.
    let symbols = conflict_symbols(merge(Lang::Rust, BASE, without_b, &edit_b)?)?;
    assert_eq!(symbols, [func("b")]);
    // Ours deletes b, theirs edits a: clean.
    let edit_a = BASE.replace("    1\n", "    10\n");
    let out = merged_text(merge(Lang::Rust, BASE, without_b, &edit_a)?)?;
    assert_eq!(out, "fn a() -> i32 {\n    10\n}\n");
    Ok(())
}

#[test]
fn rust_impl_methods_merge_independently() -> TestResult {
    let base = "impl Foo {\n    fn x(&self) -> u8 {\n        1\n    }\n\n    fn y(&self) -> u8 {\n        2\n    }\n}\n";
    let ours = base.replace("        1\n", "        10\n");
    let theirs = base.replace("        2\n", "        20\n");
    let out = merged_text(merge(Lang::Rust, base, &ours, &theirs)?)?;
    assert_eq!(
        out,
        "impl Foo {\n    fn x(&self) -> u8 {\n        10\n    }\n\n    fn y(&self) -> u8 {\n        20\n    }\n}\n"
    );
    let same_method = base.replace("        1\n", "        11\n");
    let symbols = conflict_symbols(merge(Lang::Rust, base, &ours, &same_method)?)?;
    assert_eq!(
        symbols,
        [SymbolRef {
            name: "Foo::x".to_owned(),
            kind: SymbolKind::Method
        }]
    );
    Ok(())
}

#[test]
fn both_sides_adding_methods_to_one_impl_keeps_both() -> TestResult {
    let base = "impl Foo {\n    fn x(&self) {}\n}\n";
    let ours = "impl Foo {\n    fn x(&self) {}\n\n    fn p(&self) {}\n}\n";
    let theirs = "impl Foo {\n    fn x(&self) {}\n\n    fn q(&self) {}\n}\n";
    let out = merged_text(merge(Lang::Rust, base, ours, theirs)?)?;
    assert_eq!(
        out,
        "impl Foo {\n    fn x(&self) {}\n\n    fn p(&self) {}\n\n    fn q(&self) {}\n}\n"
    );
    Ok(())
}

#[test]
fn python_class_methods_merge_independently() -> TestResult {
    let base =
        "class A:\n    def x(self):\n        return 1\n\n    def y(self):\n        return 2\n";
    let ours = base.replace("return 1", "return 10");
    let theirs = base.replace("return 2", "return 20");
    let out = merged_text(merge(Lang::Python, base, &ours, &theirs)?)?;
    assert_eq!(
        out,
        "class A:\n    def x(self):\n        return 10\n\n    def y(self):\n        return 20\n"
    );
    Ok(())
}

#[test]
fn typescript_functions_merge_independently() -> TestResult {
    let base =
        "export function a(): number { return 1; }\nexport function b(): number { return 2; }\n";
    let ours = base.replace("return 1", "return 10");
    let theirs = base.replace("return 2", "return 20");
    let out = merged_text(merge(Lang::TypeScript, base, &ours, &theirs)?)?;
    assert_eq!(
        out,
        "export function a(): number { return 10; }\nexport function b(): number { return 20; }\n"
    );
    Ok(())
}

#[test]
fn cpp_and_cuda_functions_merge_independently() -> TestResult {
    let cases = [
        (
            Lang::Cpp,
            "int a() { return 1; }
int b() { return 2; }
",
        ),
        (
            Lang::Cuda,
            "__global__ void a() { return 1; }
__device__ int b() { return 2; }
",
        ),
    ];
    for (lang, base) in cases {
        let ours = base.replace("return 1", "return 10");
        let theirs = base.replace("return 2", "return 20");
        let out = merged_text(merge(lang, base, &ours, &theirs)?)?;
        assert_eq!(
            out,
            base.replace("return 1", "return 10")
                .replace("return 2", "return 20"),
            "{lang:?}"
        );
    }
    Ok(())
}

#[test]
fn import_lines_merge_by_content() -> TestResult {
    let base = "use a::A;\n\nfn f() {}\n";
    let ours = "use a::A;\nuse b::B;\n\nfn f() {}\n";
    let theirs = "use a::A;\n\nfn f() { 1; }\n";
    let out = merged_text(merge(Lang::Rust, base, ours, theirs)?)?;
    assert_eq!(out, "use a::A;\nuse b::B;\n\nfn f() { 1; }\n");
    Ok(())
}

#[test]
fn merging_different_languages_is_an_error() -> TestResult {
    let rs = parse(Lang::Rust, "fn f() {}")?;
    let py = parse(Lang::Python, "def f(): pass")?;
    assert!(matches!(
        merge3(&rs, &rs, &py),
        Err(CodeIntelError::LanguageMismatch { .. })
    ));
    Ok(())
}

#[test]
fn unchanged_inputs_merge_to_themselves() -> TestResult {
    let cases = [
        (
            Lang::Rust,
            "fn f() {}
",
        ),
        (
            Lang::Python,
            "def f():
    pass
",
        ),
        (
            Lang::Cpp,
            "int f() { return 1; }
",
        ),
        (
            Lang::Cuda,
            "__global__ void f() {}
",
        ),
        (
            Lang::TypeScript,
            "function f() {}
",
        ),
    ];
    for (lang, src) in cases {
        assert_eq!(merged_text(merge(lang, src, src, src)?)?, src, "{lang:?}");
    }
    Ok(())
}

fn doc(vals: &[u32], extra: Option<(usize, &str)>) -> String {
    let mut parts: Vec<String> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| format!("fn f{i}() -> u32 {{\n    {v}\n}}\n"))
        .collect();
    if let Some((at, name)) = extra {
        let at = at.min(parts.len());
        parts.insert(at, format!("fn {name}() {{}}\n"));
    }
    parts.join("\n")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn one_sided_changes_reproduce_the_changed_side(
        vals in proptest::collection::vec(0u32..100, 1..6),
        idx in 0usize..6,
        new_val in 100u32..200,
        add_at in 0usize..7,
        add in any::<bool>(),
    ) {
        let base = doc(&vals, None);
        let mut edited = vals.clone();
        if let Some(slot) = edited.get_mut(idx % vals.len()) {
            *slot = new_val;
        }
        let changed = if add { doc(&edited, Some((add_at, "added"))) } else { doc(&edited, None) };
        let a = merge(Lang::Rust, &base, &base, &changed).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let b = merge(Lang::Rust, &base, &changed, &base).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let a = merged_text(a).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let b = merged_text(b).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&a, &changed);
        prop_assert_eq!(&b, &changed);
    }

    #[test]
    fn disjoint_edits_commute(
        vals in proptest::collection::vec(0u32..100, 2..7),
        i in 0usize..7,
        j in 0usize..7,
    ) {
        let n = vals.len();
        let (i, j) = (i % n, j % n);
        prop_assume!(i != j);
        let base = doc(&vals, None);
        let mut o = vals.clone();
        let mut t = vals.clone();
        if let Some(s) = o.get_mut(i) { *s += 1000; }
        if let Some(s) = t.get_mut(j) { *s += 2000; }
        let (os, ts) = (doc(&o, None), doc(&t, None));
        let ab = merged_text(merge(Lang::Rust, &base, &os, &ts).map_err(|e| TestCaseError::fail(e.to_string()))?).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let ba = merged_text(merge(Lang::Rust, &base, &ts, &os).map_err(|e| TestCaseError::fail(e.to_string()))?).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&ab, &ba);
        let mut both = vals.clone();
        if let Some(s) = both.get_mut(i) { *s += 1000; }
        if let Some(s) = both.get_mut(j) { *s += 2000; }
        prop_assert_eq!(ab, doc(&both, None));
    }
}
