//! DFA Redactor (descriptor: specs/descriptors/automata_dfa_redactor.json).

use std::error::Error;

use polaris_sdk::registry::Redactor;
use polaris_security_automata::DfaRedactor;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn aws_key() -> String {
    format!("{}{}", "AK", "IA") + "Q7XH2PZL4MNB8KVD"
}

fn redact(r: &DfaRedactor, input: &str) -> (String, u32) {
    let mut out = Vec::new();
    let stats = r.redact_into(input.as_bytes(), &mut out);
    (
        String::from_utf8_lossy(&out).into_owned(),
        stats.replacements,
    )
}

#[test]
fn redacts_matched_spans() -> TestResult {
    let r = DfaRedactor::default_redactor()?;
    let entropy = "kQ9vT2xLp8Zr4WmN7bYc1HdF6sJ3gUaE0oRiVt5X";
    let (out, n) = redact(
        &r,
        &format!("aws={} mail=ops@example.org blob={entropy} ok", aws_key()),
    );
    assert_eq!(
        out,
        "aws=[REDACTED:secret] mail=[REDACTED:pii] blob=[REDACTED:secret] ok"
    );
    assert_eq!(n, 3);
    Ok(())
}

#[test]
fn leaves_clean_text_identical() -> TestResult {
    let r = DfaRedactor::default_redactor()?;
    let clean = "cargo test -p polaris-config && rm -rf ./target/tmp";
    let mut out = Vec::new();
    let stats = r.redact_into(clean.as_bytes(), &mut out);
    assert_eq!(out, clean.as_bytes());
    assert_eq!((stats.bytes_in, stats.replacements), (clean.len(), 0));
    Ok(())
}

#[test]
fn reuses_output_buffer() -> TestResult {
    let r = DfaRedactor::default_redactor()?;
    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(b"prefix|");
    let ptr = out.as_ptr();
    r.redact_into(format!("k={}", aws_key()).as_bytes(), &mut out);
    assert_eq!(out, b"prefix|k=[REDACTED:secret]");
    assert_eq!(
        out.as_ptr(),
        ptr,
        "appending within capacity must not reallocate"
    );
    Ok(())
}

#[test]
fn idempotent_on_redacted_text() -> TestResult {
    let r = DfaRedactor::default_redactor()?;
    let (once, _) = redact(&r, &format!("{} ops@example.org", aws_key()));
    let (twice, n) = redact(&r, &once);
    assert_eq!(once, twice);
    assert_eq!(n, 0);
    Ok(())
}

proptest! {
    #[test]
    fn prop_redaction_is_idempotent(prefix in "[ -~]{0,40}", suffix in "[ -~]{0,40}") {
        let r = DfaRedactor::default_redactor().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let key = aws_key();
        let input = format!("{prefix} {key} {suffix}");
        let (once, _) = redact(&r, &input);
        let (twice, _) = redact(&r, &once);
        prop_assert!(!once.contains(&key), "secret survived: {}", once);
        prop_assert_eq!(once, twice);
    }
}
