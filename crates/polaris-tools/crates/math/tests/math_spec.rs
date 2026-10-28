use polaris_tools_math::{CalcLimits, MathError, calc, calc_f64};
use rust_decimal::Decimal;
use std::str::FromStr;

#[test]
fn point_one_plus_point_two_is_exact() {
    assert_eq!(
        calc("0.1 + 0.2", &CalcLimits::default()),
        Ok(Decimal::from_str("0.3").unwrap_or(Decimal::ZERO))
    );
}

#[test]
fn divide_by_zero_is_error() {
    assert!(calc("1 / 0", &CalcLimits::default()).is_err());
}

#[test]
fn overflow_is_error_not_wrap() {
    assert!(calc("79228162514264337593543950335 * 2", &CalcLimits::default()).is_err());
}

#[test]
fn deep_nesting_rejected() {
    let expression = "((((1))))";
    let limits = CalcLimits {
        max_depth: 2,
        ..CalcLimits::default()
    };
    assert!(calc(expression, &limits).is_err());
}

#[test]
fn unknown_identifier_rejected() {
    assert!(calc("secret(1)", &CalcLimits::default()).is_err());
}

#[test]
fn huge_exponent_rejected() {
    assert!(calc("2^1001", &CalcLimits::default()).is_err());
}

#[test]
fn negative_power_and_precedence_are_correct() {
    assert_eq!(
        calc("-(2^2)", &CalcLimits::default()),
        Ok(Decimal::from(-4))
    );
    assert_eq!(
        calc("2^3^2", &CalcLimits::default()),
        Ok(Decimal::from(512))
    );
}

#[test]
fn float_path_supports_whitelisted_transcendentals_and_rejects_unknown_names() {
    let limits = CalcLimits::default();
    assert!(calc_f64("sin(PI / 2)", &limits).is_ok_and(|value| (value - 1.0).abs() < 1e-12));
    assert!(calc_f64("environment(1)", &limits).is_err());
    assert_eq!(calc_f64("2^1001", &limits), Err(MathError::ExponentLimit));
}

#[test]
fn float_power_limit_cannot_be_bypassed_with_parenthesized_exponent() {
    assert_eq!(
        calc_f64("2^(1001)", &CalcLimits::default()),
        Err(MathError::ExponentLimit)
    );
    assert_eq!(
        calc_f64("2^(1000 + 1)", &CalcLimits::default()),
        Err(MathError::ExponentLimit)
    );
    assert_eq!(
        calc_f64("2^2^10", &CalcLimits::default()),
        Err(MathError::ExponentLimit)
    );
    assert_eq!(calc_f64("2^(10 + 1)", &CalcLimits::default()), Ok(2048.0));
}

#[test]
fn float_power_guard_preserves_unparenthesized_unary_function_operands() {
    assert!(calc_f64("2^sin PI", &CalcLimits::default()).is_ok());
    assert!(calc_f64("2^π", &CalcLimits::default()).is_ok());
}

#[test]
fn float_max_depth_limits_unparenthesized_prefix_functions() {
    let limits = CalcLimits {
        max_depth: 2,
        ..CalcLimits::default()
    };
    assert_eq!(calc_f64("2^sin sin PI", &limits), Err(MathError::TooDeep));
}

#[test]
fn float_max_depth_limits_unparenthesized_power_chains() {
    let limits = CalcLimits {
        max_depth: 2,
        ..CalcLimits::default()
    };
    assert_eq!(calc_f64("2^2^3^4", &limits), Err(MathError::TooDeep));
}

#[test]
fn float_zero_max_depth_is_rejected() {
    let limits = CalcLimits {
        max_depth: 0,
        ..CalcLimits::default()
    };
    assert_eq!(calc_f64("1", &limits), Err(MathError::TooDeep));
}

#[test]
fn negative_integer_power_is_checked_decimal() {
    assert_eq!(calc("2^-1", &CalcLimits::default()), Ok(Decimal::new(5, 1)));
    assert_eq!(
        calc("0^-1", &CalcLimits::default()),
        Err(MathError::DivideByZero)
    );
}
