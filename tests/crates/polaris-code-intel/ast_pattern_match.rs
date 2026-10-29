//! AST pattern match contract (descriptor: specs/descriptors/codeintel_ast_pattern_match.json).
use std::error::Error;

use polaris_code_intel::{Lang, ParserPool};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn matches_structural_pattern() -> TestResult {
    let parsed = ParserPool::new().parse(Lang::Python, b"os.system(user_input)\n", None)?;
    let matches = parsed.find_ast_pattern("os.system($ARG)", 8, 100)?;
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches.first().map(|span| (span.start, span.end)),
        Some((0, 21))
    );
    Ok(())
}

#[test]
fn ignores_text_inside_literals() -> TestResult {
    let parsed =
        ParserPool::new().parse(Lang::Python, b"message = 'os.system(user_input)'\n", None)?;
    assert!(
        parsed
            .find_ast_pattern("os.system($ARG)", 8, 100)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn match_limit_fails_closed() -> TestResult {
    let parsed = ParserPool::new().parse(Lang::Python, b"os.system(a)\nos.system(b)\n", None)?;
    assert!(parsed.find_ast_pattern("os.system($ARG)", 1, 100).is_err());
    Ok(())
}

#[test]
fn invalid_utf8_is_rejected() -> TestResult {
    let parsed = ParserPool::new().parse(Lang::Python, b"value = '\xff'\n", None)?;
    assert!(parsed.find_ast_pattern("value = $VALUE", 8, 100).is_err());
    Ok(())
}
