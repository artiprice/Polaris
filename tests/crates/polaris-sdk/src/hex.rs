// Unit tests of crates/polaris-sdk/src/hex.rs (`tests`); compiled into that module via #[path].
use super::nibble;

#[test]
fn every_byte_matches_ascii_hex_contract() {
    for byte in u8::MIN..=u8::MAX {
        let expected = char::from(byte)
            .to_digit(16)
            .and_then(|value| u8::try_from(value).ok());
        match expected {
            Some(value) => assert_eq!(nibble(byte, 2, 1), Ok(value), "ASCII hex mapping"),
            None => assert!(nibble(byte, 2, 1).is_err(), "non-hex byte is rejected"),
        }
    }
}
