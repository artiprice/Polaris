//! Descriptor: specs/descriptors/codeintel_parser_pool.json

use polaris_code_intel::{CodeIntelError, Lang, ParseLimits, ParserPool, skeleton};
use proptest::prelude::*;
use std::sync::Arc;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn sample(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        Lang::Python => "def add(a, b):\n    return a + b\n",
        Lang::Cpp => "int add(int a, int b) { return a + b; }\n",
        Lang::Cuda => "__global__ void k(float* p) { p[0] = 1.0f; }\n",
        Lang::TypeScript => "export function add(a: number, b: number): number { return a + b; }\n",
    }
}

#[test]
fn every_language_parses_clean() -> TestResult {
    let pool = ParserPool::new();
    for lang in Lang::ALL {
        let parsed = pool.parse(lang, sample(lang).as_bytes(), None)?;
        assert_eq!(parsed.lang(), lang);
        assert!(!parsed.has_syntax_error(), "{lang:?}");
        assert_eq!(parsed.source(), sample(lang).as_bytes());
        assert!(parsed.node_count() > 1, "{lang:?}");
    }
    Ok(())
}

#[test]
fn every_grammar_is_abi_compatible_with_the_runtime() -> TestResult {
    let pool = ParserPool::new();
    for lang in Lang::ALL {
        assert!((13..=15).contains(&lang.abi_version()), "{lang:?}");
        // Loading a grammar into a parser fails with GrammarVersionMismatch otherwise.
        pool.parse(lang, b"", None)?;
    }
    Ok(())
}

#[test]
fn oversized_source_is_rejected_at_the_limit() -> TestResult {
    let limits = ParseLimits {
        max_source_bytes: 16,
        ..ParseLimits::default()
    };
    let pool = ParserPool::with_limits(limits);
    assert!(pool.parse(Lang::Rust, &[b' '; 16], None).is_ok());
    let err = pool.parse(Lang::Rust, &[b' '; 17], None).err();
    assert_eq!(err, Some(CodeIntelError::TooLarge { limit: 16 }));
    Ok(())
}

#[test]
fn zero_timeout_cancels_and_the_pool_recovers() -> TestResult {
    let limits = ParseLimits {
        timeout: Duration::ZERO,
        ..ParseLimits::default()
    };
    let pool = ParserPool::with_limits(limits);
    let big = "fn f(a: i32) -> i32 { a + 1 }\n".repeat(20_000);
    assert_eq!(
        pool.parse(Lang::Rust, big.as_bytes(), None).err(),
        Some(CodeIntelError::ParseTimeout)
    );
    // A cancelled parser must not resume the abandoned parse on the next call.
    let small = pool.parse(Lang::Rust, sample(Lang::Rust).as_bytes(), None)?;
    let fresh = ParserPool::new().parse(Lang::Rust, sample(Lang::Rust).as_bytes(), None)?;
    assert_eq!(small.tree_fingerprint(), fresh.tree_fingerprint());
    Ok(())
}

#[test]
fn utf8_bom_keeps_original_offsets() -> TestResult {
    let src = b"\xEF\xBB\xBFpub fn main() {}\n";
    let parsed = ParserPool::new().parse(Lang::Rust, src, None)?;
    assert!(!parsed.has_syntax_error());
    assert_eq!(parsed.source(), src);
    let items = skeleton(&parsed)?.items;
    let first = items.first().ok_or("no item")?;
    assert_eq!(
        first.span.start, 3,
        "span is relative to the original bytes"
    );
    assert_eq!(first.signature, "pub fn main()");
    Ok(())
}

#[test]
fn incremental_reparse_matches_a_full_parse() -> TestResult {
    let pool = ParserPool::new();
    let v1 = "fn a() -> i32 { 1 }\nfn b() -> i32 { 2 }\nfn c() -> i32 { 3 }\n";
    let edits = [
        "fn a() -> i32 { 10 }\nfn b() -> i32 { 2 }\nfn c() -> i32 { 3 }\n",
        "fn b() -> i32 { 2 }\nfn c() -> i32 { 3 }\n",
        "fn a() -> i32 { 1 }\nfn b() -> i32 { 2 }\nfn d() -> i32 { 4 }\nfn c() -> i32 { 3 }\n",
        "",
    ];
    let prev = pool.parse(Lang::Rust, v1.as_bytes(), None)?;
    for v2 in edits {
        let inc = pool.parse(Lang::Rust, v2.as_bytes(), Some(&prev))?;
        let full = pool.parse(Lang::Rust, v2.as_bytes(), None)?;
        assert_eq!(inc.tree_fingerprint(), full.tree_fingerprint(), "{v2:?}");
        assert_eq!(inc.source(), v2.as_bytes());
    }
    Ok(())
}

#[test]
fn previous_tree_of_another_language_is_ignored() -> TestResult {
    let pool = ParserPool::new();
    let py = pool.parse(Lang::Python, sample(Lang::Python).as_bytes(), None)?;
    let rs = pool.parse(Lang::Rust, sample(Lang::Rust).as_bytes(), Some(&py))?;
    let fresh = pool.parse(Lang::Rust, sample(Lang::Rust).as_bytes(), None)?;
    assert_eq!(rs.tree_fingerprint(), fresh.tree_fingerprint());
    Ok(())
}

#[test]
fn hostile_nesting_does_not_overflow_the_stack() -> TestResult {
    let pool = ParserPool::new();
    let depth = 50_000;
    let rust = format!("fn f() {{ {}1{} }}", "(".repeat(depth), ")".repeat(depth));
    let python = format!("x = {}1{}\n", "[".repeat(depth), "]".repeat(depth));
    let cpp = format!(
        "int f() {{ return {}1{}; }}",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    let cuda = format!(
        "__global__ void k() {{{}{}}}",
        "{".repeat(depth),
        "}".repeat(depth)
    );
    let ts = format!("const x = {}1{};", "[".repeat(depth), "]".repeat(depth));
    for (lang, src) in [
        (Lang::Rust, rust),
        (Lang::Python, python),
        (Lang::Cpp, cpp),
        (Lang::Cuda, cuda),
        (Lang::TypeScript, ts),
    ] {
        match pool.parse(lang, src.as_bytes(), None) {
            Ok(parsed) => {
                // Every walk in the crate must also survive the deep tree.
                skeleton(&parsed)?;
                let _ = parsed.tree_fingerprint();
                let _ = parsed.node_count();
            }
            Err(e) => assert!(
                matches!(
                    e,
                    CodeIntelError::ParseTimeout | CodeIntelError::ParseFailed
                ),
                "{lang:?}: {e}"
            ),
        }
    }
    Ok(())
}

#[test]
fn deep_tree_fingerprint_has_a_bounded_walk_cost() -> TestResult {
    let depth = 20_000;
    let source = format!("fn f() {{ {}1{} }}", "(".repeat(depth), ")".repeat(depth));
    let parsed = ParserPool::new().parse(Lang::Rust, source.as_bytes(), None)?;
    assert!(parsed.node_count() > depth);
    let started = std::time::Instant::now();
    let fingerprint = parsed.tree_fingerprint();
    let elapsed = started.elapsed();
    // Captured from the original native-depth walk: optimizing traversal must keep the digest.
    assert_eq!(fingerprint.get(), 6_567_796_617_403_192_368);
    assert!(
        elapsed < Duration::from_secs(3),
        "fingerprint took {elapsed:?}: {fingerprint:?}"
    );
    Ok(())
}

#[test]
fn shared_pool_parses_from_many_threads() -> TestResult {
    let pool = Arc::new(ParserPool::new());
    let reference = pool.parse(Lang::Python, sample(Lang::Python).as_bytes(), None)?;
    let want = reference.tree_fingerprint();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let pool = Arc::clone(&pool);
                scope.spawn(move || -> Result<(), CodeIntelError> {
                    for _ in 0..25 {
                        let p = pool.parse(Lang::Python, sample(Lang::Python).as_bytes(), None)?;
                        assert_eq!(p.tree_fingerprint(), want);
                    }
                    Ok(())
                })
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(r) => r?,
                Err(_) => return Err("worker thread panicked".into()),
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn arbitrary_bytes_never_panic_in_any_language(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let pool = ParserPool::new();
        for lang in Lang::ALL {
            match pool.parse(lang, &bytes, None) {
                Ok(parsed) => {
                    prop_assert_eq!(parsed.source(), bytes.as_slice());
                    prop_assert!(skeleton(&parsed).is_ok());
                }
                Err(e) => prop_assert!(matches!(e, CodeIntelError::ParseFailed | CodeIntelError::ParseTimeout)),
            }
        }
    }

    #[test]
    fn incremental_equals_full_for_valid_edits(a in 0u32..100_000, b in 0u32..100_000, pad in "[ \n]{0,6}") {
        let pool = ParserPool::new();
        let v1 = format!("fn a() -> u32 {{ {a} }}\nfn b() -> u32 {{ {b} }}\n");
        let v2 = format!("fn a() -> u32 {{ {b} }}{pad}\nfn b() -> u32 {{ {a}{pad} }}\n");
        let prev = pool.parse(Lang::Rust, v1.as_bytes(), None).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let inc = pool.parse(Lang::Rust, v2.as_bytes(), Some(&prev)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let full = pool.parse(Lang::Rust, v2.as_bytes(), None).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(inc.tree_fingerprint(), full.tree_fingerprint());
    }
}
