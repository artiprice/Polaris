//! Descriptor: specs/descriptors/codeintel_subtree_fingerprints.json

use polaris_code_intel::{Lang, ParserPool, subtree_fingerprints};
use polaris_sdk::ids::FastFingerprint;
use proptest::prelude::*;
use xxhash_rust::xxh3::xxh3_64;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn prints(
    lang: Lang,
    src: &str,
) -> Result<Vec<(usize, usize, FastFingerprint)>, Box<dyn std::error::Error>> {
    let parsed = ParserPool::new().parse(lang, src.as_bytes(), None)?;
    Ok(subtree_fingerprints(&parsed)
        .into_iter()
        .map(|(span, fp)| (span.start, span.end, fp))
        .collect())
}

const BASE: &str = "fn a() -> i32 { 1 }\nfn b() -> i32 { 2 }\nstruct S { x: i32 }\n";

#[test]
fn identical_sources_have_identical_fingerprints() -> TestResult {
    assert_eq!(prints(Lang::Rust, BASE)?, prints(Lang::Rust, BASE)?);
    assert_eq!(prints(Lang::Rust, BASE)?.len(), 3);
    Ok(())
}

#[test]
fn fingerprint_is_xxh3_of_the_declaration_bytes() -> TestResult {
    for (start, end, fp) in prints(Lang::Rust, BASE)? {
        let bytes = BASE.as_bytes().get(start..end).ok_or("span out of range")?;
        assert_eq!(fp, FastFingerprint::new(xxh3_64(bytes)));
    }
    Ok(())
}

#[test]
fn editing_one_function_changes_only_its_fingerprint() -> TestResult {
    let edited = BASE.replace("{ 2 }", "{ 3 }");
    let before = prints(Lang::Rust, BASE)?;
    let after = prints(Lang::Rust, &edited)?;
    assert_eq!(before.len(), after.len());
    let differing: Vec<_> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (b, a))| b.2 != a.2)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(differing, [1]);
    Ok(())
}

#[test]
fn moving_a_function_keeps_its_fingerprint() -> TestResult {
    let shifted = format!("\n\n// header comment\n{BASE}");
    let before = prints(Lang::Rust, BASE)?;
    let after = prints(Lang::Rust, &shifted)?;
    let fps = |v: &[(usize, usize, FastFingerprint)]| v.iter().map(|t| t.2).collect::<Vec<_>>();
    assert_eq!(fps(&before), fps(&after));
    let first_before = before.first().ok_or("no prints")?;
    let first_after = after.first().ok_or("no prints")?;
    assert!(first_after.0 > first_before.0, "spans move with the text");
    Ok(())
}

#[test]
fn equal_functions_share_a_fingerprint_but_not_a_span() -> TestResult {
    let src = "fn same() { 1 }\nfn same() { 1 }\n";
    let p = prints(Lang::Rust, src)?;
    assert_eq!(p.len(), 2);
    let (a, b) = (p.first().ok_or("none")?, p.get(1).ok_or("none")?);
    assert_eq!(a.2, b.2);
    assert_ne!((a.0, a.1), (b.0, b.1));
    Ok(())
}

#[test]
fn nested_declarations_get_their_own_fingerprints() -> TestResult {
    let src = "impl Foo { fn m(&self) {} fn n(&self) {} }\n";
    let p = prints(Lang::Rust, src)?;
    assert_eq!(p.len(), 3, "impl block plus two methods");
    Ok(())
}

#[test]
fn works_for_every_language() -> TestResult {
    let cases = [
        (
            Lang::Python,
            "def f(): pass\nclass C:\n    def m(self): pass\n",
            3,
        ),
        (
            Lang::Cpp,
            "int f() { return 1; }\nstruct S { int x; };\n",
            2,
        ),
        (Lang::Cuda, "__global__ void k() {}\n", 1),
        (Lang::TypeScript, "function f() {}\nclass C { m() {} }\n", 3),
    ];
    for (lang, src, want) in cases {
        assert_eq!(prints(lang, src)?.len(), want, "{lang:?}");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn unchanged_functions_keep_their_fingerprint(vals in proptest::collection::vec(0u32..1000, 1..8), idx in 0usize..8, new_val in 1000u32..2000) {
        let render = |v: &[u32]| -> String {
            v.iter().enumerate().map(|(i, x)| format!("fn f{i}() -> u32 {{ {x} }}\n")).collect()
        };
        let mut edited = vals.clone();
        let at = idx % vals.len();
        if let Some(slot) = edited.get_mut(at) {
            *slot = new_val;
        }
        let a = prints(Lang::Rust, &render(&vals)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let b = prints(Lang::Rust, &render(&edited)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            prop_assert_eq!(x.2 != y.2, i == at);
        }
    }
}
