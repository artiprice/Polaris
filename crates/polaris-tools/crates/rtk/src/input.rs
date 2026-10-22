//! Stream decoding: BOM and UTF-16 handling, line-ending normalisation, terminal-escape stripping.

use std::borrow::Cow;
use std::iter::Peekable;
use std::str::Chars;

use crate::error::RtkError;

/// Largest stream the engine accepts (256 MiB, the CAS blob limit, so the raw stream can always be
/// stored before it is filtered).
pub const MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;

/// Refuses streams over [`MAX_STREAM_BYTES`].
pub(crate) fn check_size(stream: &[u8]) -> Result<(), RtkError> {
    if stream.len() > MAX_STREAM_BYTES {
        return Err(RtkError::StreamTooLarge {
            size: stream.len(),
            limit: MAX_STREAM_BYTES,
        });
    }
    Ok(())
}

/// Decodes UTF-16 code units (without BOM) from byte pairs; an odd trailing byte is dropped and
/// unpaired surrogates become U+FFFD.
fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units = bytes.chunks_exact(2).filter_map(|pair| match *pair {
        [a, b] => Some(unit([a, b])),
        _ => None,
    });
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Interprets `bytes` as text: a UTF-8 BOM is stripped, a UTF-16 BOM (PowerShell redirection)
/// selects UTF-16 LE or BE, anything else is UTF-8 with invalid sequences replaced.
fn decode_encoding(bytes: &[u8]) -> Cow<'_, str> {
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest),
        [0xFF, 0xFE, rest @ ..] => Cow::Owned(decode_utf16(rest, u16::from_le_bytes)),
        [0xFE, 0xFF, rest @ ..] => Cow::Owned(decode_utf16(rest, u16::from_be_bytes)),
        _ => String::from_utf8_lossy(bytes),
    }
}

/// Consumes the rest of an escape sequence after `ESC`. A sequence never consumes a newline, so a
/// hostile stream cannot use an unterminated sequence to hide the lines that follow it.
fn skip_escape(chars: &mut Peekable<Chars<'_>>) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            while let Some(&next) = chars.peek() {
                if !(' '..='~').contains(&next) {
                    break;
                }
                chars.next();
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
        Some(']' | 'P' | 'X' | '^' | '_') => {
            chars.next();
            while let Some(&next) = chars.peek() {
                match next {
                    '\n' | '\r' => break,
                    '\u{7}' => {
                        chars.next();
                        break;
                    }
                    '\u{1b}' => {
                        chars.next();
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    _ => {
                        chars.next();
                    }
                }
            }
        }
        Some('\n' | '\r') | None => {}
        Some(_) => {
            chars.next();
        }
    }
}

/// Decodes a raw compiler or linter stream into clean text.
///
/// - UTF-8 BOM stripped; UTF-16 (LE or BE, BOM-marked) transcoded; invalid UTF-8 replaced.
/// - `CRLF` and lone `CR` become `LF`.
/// - Terminal escape sequences (CSI colours, OSC titles, ...) and control characters other than
///   `LF` and `TAB` are removed, so nothing a compiler prints can drive the terminal or the ledger.
#[must_use]
pub fn decode_stream(bytes: &[u8]) -> String {
    let utf8_body = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(body) => body,
        None => bytes,
    };
    if let Ok(text) = std::str::from_utf8(utf8_body) {
        if !has_controls_or_escape(text.as_bytes()) {
            return text.to_owned();
        }
    }
    let text = decode_encoding(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\t' => out.push(c),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{1b}' => skip_escape(&mut chars),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn has_controls_or_escape(bytes: &[u8]) -> bool {
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'\r'
            || byte == 0x1B
            || byte == 0x7F
            || (byte < 0x20 && byte != b'\n' && byte != b'\t')
        {
            return true;
        }
        if byte == 0xC2
            && bytes
                .get(index.saturating_add(1))
                .is_some_and(|next| (0x80..=0x9F).contains(next))
        {
            return true;
        }
    }
    false
}
