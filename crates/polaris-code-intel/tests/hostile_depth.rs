use polaris_code_intel::{CodeIntelError, Lang, ParseLimits, ParserPool, skeleton};
use std::time::Duration;

#[test]
fn ten_thousand_nested_blocks_never_recurse_in_our_walkers()
-> Result<(), Box<dyn std::error::Error>> {
    let mut source = String::from("fn nested() {");
    source.push_str(&"{".repeat(10_000));
    source.push_str("let value = 1;");
    source.push_str(&"}".repeat(10_000));
    source.push('}');
    let pool = ParserPool::with_limits(ParseLimits {
        max_source_bytes: 64 * 1024,
        timeout: Duration::from_secs(2),
    });
    match pool.parse(Lang::Rust, source.as_bytes(), None) {
        Ok(parsed) => {
            assert!(!parsed.has_syntax_error());
            assert!(parsed.node_count() > 10_000);
            let view = skeleton(&parsed)?;
            assert!(view.items.iter().any(|item| item.name == "nested"));
            let _fingerprint = parsed.tree_fingerprint();
        }
        Err(CodeIntelError::ParseTimeout) => {
            // The parser's enforced time bound is an allowed response to hostile input.
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
