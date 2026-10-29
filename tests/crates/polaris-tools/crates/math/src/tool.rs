// Unit tests of crates/polaris-tools/crates/math/src/tool.rs (`tests`); compiled into that module via #[path].
use super::*;
use polaris_sdk::registry::Tool;

#[test]
fn tool_spec_is_pure_and_has_a_real_operation_schema() {
    let spec = MathTool.spec();
    let schema: Result<serde_json::Value, _> = serde_json::from_str(&spec.input_schema_json);
    assert_eq!(spec.name.as_ref(), "math");
    assert_eq!(spec.effects, Effects::PURE);
    assert!(
        schema
            .as_ref()
            .is_ok_and(|value| value.get("oneOf").is_some() || value.get("$defs").is_some())
    );
    let snapshot_value = serde_json::json!({
        "name": spec.name,
        "description": spec.description,
        "effects": spec.effects,
        "input_schema": schema.unwrap_or(serde_json::Value::Null),
    });
    insta::assert_json_snapshot!("math_tool_spec", snapshot_value);
}

#[test]
fn typed_dispatch_preserves_decimal_output() {
    let result = dispatch(MathInput::Calc {
        expression: "0.1+0.2".to_owned(),
    });
    assert_eq!(result, Ok(MathOutput::Decimal("0.3".to_owned())));
}

#[test]
fn invalid_decimal_string_returns_typed_error() {
    let result = dispatch(MathInput::EffectiveBandwidth {
        bytes: 10,
        elapsed_seconds: "zero".to_owned(),
    });
    assert!(result.is_err());
}
