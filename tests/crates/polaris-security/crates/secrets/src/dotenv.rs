// Unit tests of crates/polaris-security/crates/secrets/src/dotenv.rs (`tests`); compiled into that module via #[path].
use super::{DotenvError, parse};
use secrecy::ExposeSecret;

#[test]
fn parses_names_values_comments_quotes_and_export() -> Result<(), DotenvError> {
    let values = parse(
        "# comment\n\nOPENROUTER_API_KEY=sk-or-1\nexport NEBIUS_API_KEY=\"nb 2\"\nTAVILY_API_KEY='tv3'\nEMPTY=\n",
    )?;
    let get = |name: &str| {
        values
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value.expose_secret().to_owned())
    };
    assert_eq!(get("OPENROUTER_API_KEY").as_deref(), Some("sk-or-1"));
    assert_eq!(get("NEBIUS_API_KEY").as_deref(), Some("nb 2"));
    assert_eq!(get("TAVILY_API_KEY").as_deref(), Some("tv3"));
    assert_eq!(get("EMPTY"), None, "empty values are treated as unset");
    Ok(())
}

#[test]
fn rejects_malformed_lines_without_echoing_content() {
    let error = parse("GOOD=1\nnot a pair\n").err();
    assert_eq!(error, Some(DotenvError::Syntax { line: 2 }));
    assert_eq!(
        parse("lower=1").err(),
        Some(DotenvError::Syntax { line: 1 })
    );
    let message = DotenvError::Syntax { line: 2 }.to_string();
    assert!(!message.contains("not a pair"));
}
