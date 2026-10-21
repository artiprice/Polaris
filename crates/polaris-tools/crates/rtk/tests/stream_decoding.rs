//! Stream decoding (descriptor: specs/descriptors/rtk_stream_decoding.json).

use std::error::Error;

use polaris_tools_rtk::decode_stream;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
    let mut bytes = if little_endian {
        vec![0xFF, 0xFE]
    } else {
        vec![0xFE, 0xFF]
    };
    for unit in text.encode_utf16() {
        let pair = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        bytes.extend_from_slice(&pair);
    }
    bytes
}

#[test]
fn utf8_bom_is_stripped() -> TestResult {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"error: boom\n");
    assert_eq!(decode_stream(&bytes), "error: boom\n");
    Ok(())
}

#[test]
fn utf16_little_endian_bom_is_transcoded() -> TestResult {
    let bytes = utf16("error: größe\r\n", true);
    assert_eq!(decode_stream(&bytes), "error: größe\n");
    Ok(())
}

#[test]
fn utf16_big_endian_bom_is_transcoded() -> TestResult {
    let bytes = utf16("error: größe\n", false);
    assert_eq!(decode_stream(&bytes), "error: größe\n");
    Ok(())
}

#[test]
fn crlf_and_lone_cr_become_lf() -> TestResult {
    assert_eq!(decode_stream(b"a\r\nb\rc\n\r\nd"), "a\nb\nc\n\nd");
    Ok(())
}

#[test]
fn ansi_escape_sequences_are_stripped() -> TestResult {
    let input = "\u{1b}[1m\u{1b}[38;5;9merror\u{1b}[0m: x\n\u{1b}]0;title\u{7}ok\n\u{1b}]8;;http://h\u{1b}\\link\n";
    assert_eq!(decode_stream(input.as_bytes()), "error: x\nok\nlink\n");
    Ok(())
}

#[test]
fn unterminated_escape_does_not_swallow_following_lines() -> TestResult {
    let input = "a\u{1b}]0;never closed\nerror: still visible\nb\u{1b}[31\nerror: also visible\n";
    let out = decode_stream(input.as_bytes());
    assert!(out.contains("error: still visible"), "got {out:?}");
    assert!(out.contains("error: also visible"), "got {out:?}");
    Ok(())
}

#[test]
fn control_characters_are_dropped_and_tabs_kept() -> TestResult {
    assert_eq!(decode_stream(b"a\x00b\x07c\x08d\te\x7ff\n"), "abcd\tef\n");
    Ok(())
}

#[test]
fn invalid_utf8_is_replaced_not_fatal() -> TestResult {
    let out = decode_stream(b"ok \xff\xfe end\n");
    assert!(out.starts_with("ok "), "got {out:?}");
    assert!(out.contains('\u{fffd}'), "got {out:?}");
    assert!(out.ends_with(" end\n"), "got {out:?}");
    Ok(())
}

proptest! {
    #[test]
    fn prop_decoded_text_has_no_escape_or_carriage_return(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048)
    ) {
        let out = decode_stream(&bytes);
        let has_escape = out.contains('\u{1b}');
        let has_cr = out.contains('\r');
        prop_assert!(!has_escape);
        prop_assert!(!has_cr);
    }
}
