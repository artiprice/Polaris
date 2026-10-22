use polaris_code_intel::{Lang, MAX_SAFETY_NODES, ParseLimits, ParserPool, analyze_safety};
use std::time::{Duration, Instant};

const DEFAULT_MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;

#[test]
fn safety_inventory_scales_to_ten_thousand_rust_tests() -> Result<(), Box<dyn std::error::Error>> {
    let mut source = String::new();
    for index in 0..10_000usize {
        source.push_str("#[test] fn test_");
        source.push_str(&index.to_string());
        source.push_str("() { assert!(true); }\n");
    }
    let pool = ParserPool::with_limits(ParseLimits {
        max_source_bytes: 2 * 1024 * 1024,
        timeout: Duration::from_secs(5),
    });
    let parsed = pool.parse(Lang::Rust, source.as_bytes(), None)?;
    assert!(!parsed.has_syntax_error());
    assert!(parsed.node_count() <= MAX_SAFETY_NODES);

    let started = Instant::now();
    let analysis = analyze_safety(&parsed)?;
    let elapsed = started.elapsed();
    assert_eq!(analysis.counts.tests, 10_000);
    assert_eq!(analysis.counts.assertions, 10_000);
    assert_eq!(analysis.counts.tautologies, 10_000);
    eprintln!(
        "safety inventory: tests=10000 bytes={} nodes={} elapsed_ms={}",
        source.len(),
        parsed.node_count(),
        elapsed.as_millis()
    );
    Ok(())
}

#[test]
fn fifty_megabyte_source_is_rejected_before_tree_sitter() {
    let source = vec![b' '; 50 * 1024 * 1024];
    let result = ParserPool::new().parse(Lang::Rust, &source, None);
    assert!(matches!(
        result,
        Err(polaris_code_intel::CodeIntelError::TooLarge {
            limit: DEFAULT_MAX_SOURCE_BYTES
        })
    ));
}
