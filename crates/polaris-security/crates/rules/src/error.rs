//! Typed failures for pack parsing and bounded audits.

/// Failure to load or evaluate a security rule pack.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RulesError {
    /// Pack YAML is malformed, oversized or violates its schema.
    #[error("invalid rule pack YAML: {0}")]
    Yaml(String),
    /// Pattern compilation failed for the named rule.
    #[error("invalid pattern for rule {rule}: {reason}")]
    BadPattern { rule: String, reason: String },
    /// CWE is absent from the explicit mapping table.
    #[error("CWE-{0} is not mapped by this rule library")]
    UnknownCwe(u16),
    /// Pack, rule, path, or request violates a hard bound.
    #[error("security rule input exceeded or violated a hard bound: {0}")]
    Bound(String),
    /// Duplicate pack or rule identifiers.
    #[error("duplicate security rule identifier: {0}")]
    DuplicateId(String),
    /// Invalid rule identifier.
    #[error("invalid security rule identifier")]
    InvalidRuleId,
    /// Rule schema has no valid exactly-one pattern definition.
    #[error("rule must define exactly one supported pattern kind")]
    InvalidRuleKind,
    /// A configured or requested language is unsupported.
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
    /// Source parsing or AST pattern evaluation failed closed.
    #[error("AST evaluation failed: {0}")]
    Ast(String),
    /// A supported manifest is malformed or contains an invalid package name.
    #[error("invalid package manifest: {0}")]
    Manifest(String),
    /// DFA scanner failed to compile or scan.
    #[error("DFA evaluation failed: {0}")]
    Dfa(String),
    /// S1 result could not establish a calibrated verdict; findings remain intact.
    #[error("S1 arbitration unavailable or untrusted: {0}")]
    Arbitration(String),
}
