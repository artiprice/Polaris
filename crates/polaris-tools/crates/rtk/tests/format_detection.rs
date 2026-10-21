//! Format detection (descriptor: specs/descriptors/rtk_format_detection.json).

use std::error::Error;

use polaris_tools_rtk::{StreamFormat, detect_format};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const JSON_LINE: &str = r#"{"reason":"compiler-message","message":{"level":"error"}}"#;

#[test]
fn leading_brace_is_json() -> TestResult {
    let stream = format!("{JSON_LINE}\n{JSON_LINE}\n");
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::CargoJson);
    Ok(())
}

#[test]
fn leading_whitespace_then_brace_is_json() -> TestResult {
    let stream = format!("  \n\t\n   {JSON_LINE}\n");
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::CargoJson);
    Ok(())
}

#[test]
fn stderr_noise_then_json_is_mixed() -> TestResult {
    let stream = format!("   Compiling demo v0.1.0 (C:\\w\\demo)\n{JSON_LINE}\n");
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::Mixed);
    Ok(())
}

#[test]
fn trailing_summary_after_json_is_mixed() -> TestResult {
    let stream = format!("{JSON_LINE}\nerror: could not compile `demo` (bin \"demo\")\n");
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::Mixed);
    Ok(())
}

#[test]
fn plain_text_is_text() -> TestResult {
    let stream = "error[E0425]: cannot find value `x` in this scope\n --> src/main.rs:2:5\n";
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::Text);
    Ok(())
}

#[test]
fn empty_input_is_text() -> TestResult {
    assert_eq!(detect_format(b""), StreamFormat::Text);
    assert_eq!(detect_format(b" \n\t\r\n"), StreamFormat::Text);
    Ok(())
}

#[test]
fn brace_inside_text_line_is_not_json() -> TestResult {
    let stream = "error: expected `{`, found `}`\nstruct A {\n{ not json at all\n";
    assert_eq!(detect_format(stream.as_bytes()), StreamFormat::Text);
    Ok(())
}

#[test]
fn utf16_json_is_detected_after_transcoding() -> TestResult {
    let text = format!("{JSON_LINE}\r\n");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(detect_format(&bytes), StreamFormat::CargoJson);
    Ok(())
}

proptest! {
    #[test]
    fn prop_detect_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = detect_format(&bytes);
    }
}
