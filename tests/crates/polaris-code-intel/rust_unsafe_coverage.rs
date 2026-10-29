use polaris_code_intel::{AnalysisCoverage, Lang, ParserPool, analyze_safety};
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
fn rust_without_expansion_has_complete_unsafe_inventory() -> TestResult {
    let pool = ParserPool::new();
    for (source, minimum) in [
        ("", 0),
        ("fn ordinary() {}", 0),
        (r#"fn text() { let value = "unsafe extern"; }"#, 0),
        ("unsafe fn operation() {}", 1),
        ("fn operation() { unsafe {} }", 1),
        ("unsafe trait Marker {}", 1),
        ("struct Value; unsafe impl Send for Value {}", 1),
        (r#"unsafe extern "C" { fn operation(); }"#, 1),
    ] {
        let parsed = pool.parse(Lang::Rust, source.as_bytes(), None)?;
        let analysis = analyze_safety(&parsed)?;
        assert!(!analysis.syntax_error, "invalid fixture: {source}");
        assert_eq!(
            analysis.unsafe_coverage,
            AnalysisCoverage::Complete,
            "{source}"
        );
        assert!(
            analysis.unsafe_constructs.len() >= minimum,
            "missing unsafe syntax: {source}"
        );
        if minimum == 0 {
            assert!(
                analysis.unsafe_constructs.is_empty(),
                "false unsafe observation: {source}"
            );
        }
    }
    Ok(())
}
#[test]
fn rust_expansion_and_invalid_syntax_keep_incomplete_coverage() -> TestResult {
    let pool = ParserPool::new();
    for source in [
        "fn example() { expand!(); }",
        "macro_rules! expand { () => {} }",
        "#[derive(Clone)] struct Value;",
        "#[test] fn example() {}",
        "fn broken(",
    ] {
        let parsed = pool.parse(Lang::Rust, source.as_bytes(), None)?;
        assert_eq!(
            analyze_safety(&parsed)?.unsafe_coverage,
            AnalysisCoverage::Incomplete,
            "{source}"
        );
    }
    Ok(())
}
