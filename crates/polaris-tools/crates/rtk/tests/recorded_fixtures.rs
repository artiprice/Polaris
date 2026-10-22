//! Contract checks for the recorded real Cargo fixture under `tests/fixtures/rtk`.

use polaris_tools_rtk::{StreamFormat, filter_text, parse_diagnostics};

#[test]
fn recorded_cargo_e0308_has_code_location_and_all_primary_spans_are_safe()
-> Result<(), Box<dyn std::error::Error>> {
    let input = include_bytes!("fixtures/rtk/cargo-e0308.jsonl");
    let diagnostics = parse_diagnostics(input, StreamFormat::CargoJson)?;
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics.first().ok_or("missing recorded diagnostic")?;
    assert_eq!(diagnostic.code.as_deref(), Some("E0308"));
    assert!(diagnostic.message.contains("mismatched types"));
    let location = diagnostic
        .primary
        .as_ref()
        .ok_or("missing primary location")?;
    assert_eq!(location.file, "src\\main.rs");
    assert_eq!(location.line, 1);
    assert_eq!(location.column, 30);
    Ok(())
}

#[test]
fn recorded_cargo_human_stream_retains_real_diagnostic() -> Result<(), Box<dyn std::error::Error>> {
    let input = include_bytes!("fixtures/rtk/cargo-e0308.txt");
    let filtered = filter_text(input)?;
    assert!(filtered.contains("error[E0308]: mismatched types"));
    assert!(filtered.contains("--> src\\main.rs:1:30"));
    assert!(!filtered.contains("Compiling"));
    Ok(())
}
