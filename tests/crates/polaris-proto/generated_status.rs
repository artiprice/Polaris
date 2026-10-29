#[path = "../../../crates/polaris-proto/build_support.rs"]
mod build_support;

#[test]
fn generated_status_uses_typed_conversion() {
    let source = include_str!(concat!(env!("OUT_DIR"), "/polaris.v1.rs"));
    assert_eq!(
        source
            .matches("i32::from(tonic::Code::Unimplemented)")
            .count(),
        3,
        "all three generated service fallbacks use the typed conversion"
    );
    assert!(
        !source.contains("tonic::Code::Unimplemented as i32"),
        "unchecked generated casts are absent"
    );
}

#[test]
fn rewrites_exact_expected_status_casts() -> Result<(), Box<dyn std::error::Error>> {
    let source = "(tonic::Code::Unimplemented as i32);\n".repeat(3);
    let checked = build_support::rewrite_generated_status_casts(&source)?;
    assert_eq!(
        checked,
        "i32::from(tonic::Code::Unimplemented);\n".repeat(3),
        "only known fallbacks change"
    );
    Ok(())
}

#[test]
fn unexpected_generator_shape_fails_closed() {
    for count in [0, 1, 2, 4] {
        let source = "(tonic::Code::Unimplemented as i32);\n".repeat(count);
        assert!(
            build_support::rewrite_generated_status_casts(&source).is_err(),
            "service fallback drift requires review"
        );
    }
    let known = "(tonic::Code::Unimplemented as i32);\n".repeat(3);
    for cast in [
        "value as u64",
        "value as usize;",
        "value as f32)",
        "value as _;",
    ] {
        assert!(
            build_support::rewrite_generated_status_casts(&format!("{known}\n{cast}")).is_err(),
            "new primitive casts require review"
        );
    }
}
