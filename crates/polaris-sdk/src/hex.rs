//! Lowercase hex encoding and strict decoding without indexing or panics.

use crate::error::ParseError;

/// Appends the lowercase hex form of `bytes` to `out`.
pub(crate) fn encode_into(bytes: &[u8], out: &mut String) {
    out.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            if let Some(c) = char::from_digit(u32::from(nibble), 16) {
                out.push(c);
            }
        }
    }
}

/// Lowercase hex form of `bytes`.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    encode_into(bytes, &mut out);
    out
}

fn nibble(c: u8, expected: usize, found: usize) -> Result<u8, ParseError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(ParseError::InvalidHex { expected, found }),
    }
}

fn decode_pairs(text: &str, out: &mut [u8], expected: usize) -> Result<(), ParseError> {
    let found = text.len();
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let (hi, lo) = match pair {
            [hi, lo] => (nibble(*hi, expected, found)?, nibble(*lo, expected, found)?),
            _ => return Err(ParseError::InvalidHex { expected, found }),
        };
        *slot = (hi << 4) | lo;
    }
    Ok(())
}

/// Decodes exactly `N` bytes from `2 * N` hex characters (either case).
pub(crate) fn decode_array<const N: usize>(text: &str) -> Result<[u8; N], ParseError> {
    let expected = N.saturating_mul(2);
    if text.len() != expected {
        return Err(ParseError::InvalidHex {
            expected,
            found: text.len(),
        });
    }
    let mut out = [0u8; N];
    decode_pairs(text, &mut out, expected)?;
    Ok(out)
}

/// Decodes an even-length hex string of any size.
pub(crate) fn decode_vec(text: &str) -> Result<Vec<u8>, ParseError> {
    let found = text.len();
    if found % 2 != 0 {
        return Err(ParseError::InvalidHex {
            expected: found.saturating_add(1),
            found,
        });
    }
    let mut out = vec![0u8; found / 2];
    decode_pairs(text, &mut out, found)?;
    Ok(out)
}
