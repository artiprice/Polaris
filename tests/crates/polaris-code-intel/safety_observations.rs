use polaris_code_intel::{
    AnalysisCoverage, Lang, MarkerKind, ParserPool, TautologyKind, analyze_safety,
};

#[test]
fn safety_observations_cover_the_five_supported_grammars() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = ParserPool::new();
    let cases = [
        (
            Lang::Rust,
            "mod unit { #[test] #[ignore] fn test_case() { assert_eq!(value, value); unsafe { core::hint::black_box(0); } } }",
        ),
        (
            Lang::Python,
            "def test_case():\n    assert True\n    eval('value')\n",
        ),
        (
            Lang::Cpp,
            "void f(int *p) { int value = (int)p; assert(true); }",
        ),
        (
            Lang::Cuda,
            "__global__ void f(int *p) { int value = (int)p; assert(true); }",
        ),
        (
            Lang::TypeScript,
            "test.skip('case', () => { expect(true).toBe(true); });",
        ),
    ];

    for (lang, source) in cases {
        let parsed = pool.parse(lang, source.as_bytes(), None)?;
        let analysis = analyze_safety(&parsed)?;
        assert!(!analysis.syntax_error, "{lang:?}");
        assert!(analysis.counts.calls > 0, "{lang:?}");
        assert_ne!(
            analysis.call_coverage,
            AnalysisCoverage::Unsupported,
            "{lang:?}"
        );
    }

    let rust = analyze_safety(&pool.parse(Lang::Rust, cases[0].1.as_bytes(), None)?)?;
    assert_eq!(rust.counts.tests, 1);
    assert!(
        rust.tests
            .first()
            .is_some_and(|test| test.identity == "unit::test_case")
    );
    assert!(rust.tests.first().is_some_and(|test| {
        test.markers
            .iter()
            .any(|marker| marker.kind == MarkerKind::IgnoreOrSkip)
    }));
    assert!(
        rust.tautologies
            .iter()
            .any(|value| value.kind == TautologyKind::SelfEqualityAssertion)
    );
    assert!(rust.counts.unsafe_constructs > 0);

    let python = analyze_safety(&pool.parse(Lang::Python, cases[1].1.as_bytes(), None)?)?;
    assert_eq!(python.counts.tests, 1);
    assert!(
        python
            .tautologies
            .iter()
            .any(|value| value.kind == TautologyKind::LiteralTrueAssertion)
    );
    assert!(python.counts.unsafe_constructs > 0);

    for (lang, source) in [cases[2], cases[3]] {
        let analysis = analyze_safety(&pool.parse(lang, source.as_bytes(), None)?)?;
        assert_eq!(analysis.test_coverage, AnalysisCoverage::Unsupported);
        assert!(analysis.counts.unsafe_constructs > 0, "{lang:?}");
    }

    let typescript = analyze_safety(&pool.parse(Lang::TypeScript, cases[4].1.as_bytes(), None)?)?;
    assert!(typescript.counts.tests > 0);
    assert!(typescript.tests.iter().any(|test| {
        test.markers
            .iter()
            .any(|marker| marker.kind == MarkerKind::IgnoreOrSkip)
    }));
    assert!(
        typescript
            .tautologies
            .iter()
            .any(|value| value.kind == TautologyKind::LiteralTrueAssertion)
    );
    Ok(())
}

#[test]
fn unrelated_rust_attribute_text_does_not_create_a_test() -> Result<(), Box<dyn std::error::Error>>
{
    let pool = ParserPool::new();
    let parsed = pool.parse(Lang::Rust, b"#[contest] fn ordinary() {}", None)?;
    let analysis = analyze_safety(&parsed)?;
    assert_eq!(analysis.counts.tests, 0);
    Ok(())
}
