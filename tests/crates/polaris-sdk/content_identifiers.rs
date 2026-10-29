//! Content Identifiers (descriptor: specs/descriptors/sdk_content_identifiers.json).

use std::error::Error;

use polaris_sdk::error::ParseError;
use polaris_sdk::ids::{Cid, ShortRef};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn sample_bytes() -> [u8; 32] {
    core::array::from_fn(|i| u8::try_from(i).map_or(0, |b| b.wrapping_mul(37).wrapping_add(11)))
}

#[test]
fn cid_hex_roundtrip() -> TestResult {
    let bytes = sample_bytes();
    let cid = Cid::from_bytes(bytes);
    let hex = cid.to_hex();
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{hex}"
    );
    assert_eq!(Cid::from_hex(&hex)?, cid);
    assert_eq!(Cid::from_hex(&hex.to_uppercase())?, cid);
    assert_eq!(cid.as_bytes(), &bytes);
    Ok(())
}

#[test]
fn short_ref_is_first_48_bits() {
    let bytes = sample_bytes();
    let short = ShortRef::of(&Cid::from_bytes(bytes));
    assert_eq!(Some(&short.as_bytes()[..]), bytes.get(..6));
}

#[test]
fn short_ref_renders_12_lowercase_hex() {
    let short = ShortRef::of(&Cid::from_bytes(sample_bytes()));
    let text = short.to_string();
    assert_eq!(text, "0b30557a9fc4");
    assert_eq!(short.to_marker(), "§ref:0b30557a9fc4");
}

#[test]
fn cid_rejects_wrong_length_hex() {
    for bad in ["a".repeat(63), "a".repeat(65), String::new()] {
        assert!(
            matches!(
                Cid::from_hex(&bad),
                Err(ParseError::InvalidHex { expected: 64, .. })
            ),
            "length {} must be rejected",
            bad.len()
        );
    }
    let mut not_hex = "0".repeat(63);
    not_hex.push('g');
    assert!(matches!(
        Cid::from_hex(&not_hex),
        Err(ParseError::InvalidHex { .. })
    ));
    assert!(
        Cid::from_hex("é".repeat(32).as_str()).is_err(),
        "multi-byte input must not panic"
    );
}

#[test]
fn short_ref_parse_accepts_marker_and_bare_hex() -> TestResult {
    let marked: ShortRef = "§ref:0123456789ab".parse()?;
    let bare: ShortRef = "0123456789AB".parse()?;
    assert_eq!(marked, bare);
    assert_eq!(bare.to_string(), "0123456789ab");
    for bad in [
        "0123456789a",
        "0123456789abc",
        "§ref:0123456789abc",
        "§ref:",
        "ref:0123456789ab",
    ] {
        assert!(bad.parse::<ShortRef>().is_err(), "{bad:?} must be rejected");
    }
    Ok(())
}

proptest! {
    #[test]
    fn prop_cid_shortref_hex_roundtrip(bytes in any::<[u8; 32]>()) {
        let cid = Cid::from_bytes(bytes);
        prop_assert_eq!(Cid::from_hex(&cid.to_hex()).ok(), Some(cid));
        let short = ShortRef::of(&cid);
        prop_assert_eq!(short.to_string().parse::<ShortRef>().ok(), Some(short));
        prop_assert_eq!(short.to_marker().parse::<ShortRef>().ok(), Some(short));
    }
}
