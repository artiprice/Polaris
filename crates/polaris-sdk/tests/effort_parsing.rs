//! Effort Parsing (descriptor: specs/descriptors/sdk_effort_parsing.json).

use std::error::Error;

use polaris_sdk::effort::Effort;
use polaris_sdk::error::ParseError;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn parses_names_case_insensitively() -> TestResult {
    assert_eq!("low".parse::<Effort>()?, Effort::Low);
    assert_eq!("Medium".parse::<Effort>()?, Effort::Medium);
    assert_eq!("HIGH".parse::<Effort>()?, Effort::High);
    assert_eq!("xHigh".parse::<Effort>()?, Effort::XHigh);
    assert_eq!("  max ".parse::<Effort>()?, Effort::Max);
    Ok(())
}

#[test]
fn parses_numeric_1_to_5() -> TestResult {
    let all = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::XHigh,
        Effort::Max,
    ];
    for (n, want) in (1u8..=5).zip(all) {
        assert_eq!(n.to_string().parse::<Effort>()?, want);
        assert_eq!(want.as_u8(), n);
    }
    Ok(())
}

#[test]
fn rejects_zero_and_six() {
    for bad in ["0", "6", "", "extreme", "1.5", "-1"] {
        assert!(
            matches!(bad.parse::<Effort>(), Err(ParseError::InvalidEffort(_))),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn display_roundtrips_name() -> TestResult {
    for level in [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::XHigh,
        Effort::Max,
    ] {
        let name = level.to_string();
        assert_eq!(name, name.to_lowercase());
        assert_eq!(name.parse::<Effort>()?, level);
    }
    assert_eq!(Effort::XHigh.to_string(), "xhigh");
    Ok(())
}

#[test]
fn try_from_u8_is_total() {
    for n in 0..=u8::MAX {
        let parsed = Effort::try_from(n);
        assert_eq!(parsed.is_ok(), (1..=5).contains(&n), "u8 {n}");
    }
}

proptest! {
    #[test]
    fn prop_order_matches_numeric_form(a in 1u8..=5, b in 1u8..=5) {
        let (ea, eb) = (Effort::try_from(a), Effort::try_from(b));
        prop_assert!(ea.is_ok() && eb.is_ok());
        if let (Ok(ea), Ok(eb)) = (ea, eb) {
            prop_assert_eq!(ea.cmp(&eb), a.cmp(&b));
        }
    }
}
