use polaris_tools_math::{CalcLimits, MathError, calc};

#[test]
fn hostile_expression_shapes_are_rejected_at_configured_bounds() {
    let limits = CalcLimits::default();
    let overlong = "1".repeat(limits.max_len.saturating_add(1));
    assert_eq!(calc(&overlong, &limits), Err(MathError::TooLong));
    let excessive_length = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
    assert_eq!(calc(&excessive_length, &limits), Err(MathError::TooLong));
    let deep = format!("{}1{}", "(".repeat(65), ")".repeat(65));
    assert_eq!(calc(&deep, &limits), Err(MathError::TooDeep));
    assert_eq!(
        calc("9^9^9^9^9^9^9^9", &limits),
        Err(MathError::ExponentLimit)
    );
}
