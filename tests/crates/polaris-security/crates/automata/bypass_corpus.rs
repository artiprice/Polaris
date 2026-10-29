//! Synthetic secret-detection bypass corpus. Each negative case is explicitly out of scope for
//! byte-oriented rules; this does not imply a broader absence guarantee.

use std::error::Error;

use polaris_security_automata::DfaScanner;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn documents_known_secret_representation_bypasses() -> TestResult {
    let scanner = DfaScanner::default_pack()?;
    let fake_key = "AKIAABCDEFGHIJKLMNOP";
    assert!(scanner.any_match(fake_key.as_bytes())?);

    let corpus = [
        ("homoglyph", "АKIAABCDEFGHIJKLMNOP"),
        ("zero_width", "AKIA\u{200B}ABCDEFGHIJKLMNOP"),
        ("base64_wrapped", "QUtJQUFCQ0RFRkdISUpLTE1OT1A="),
        ("line_folded", "AKIA\nABCDEFGHIJKLMNOP"),
    ];
    for (case, text) in corpus {
        let detected = scanner.any_match(text.as_bytes())?;
        eprintln!("bypass_case={case};detected={detected};classification=out_of_scope");
        assert!(!detected, "{case} is documented as out of scope");
    }
    Ok(())
}
