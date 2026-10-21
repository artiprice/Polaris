//! Subagent Model Ranges (descriptor: specs/descriptors/directives_subagent_model_ranges.json).

use std::error::Error;

use polaris_kernel_directives::directive::{ModelEffort, SessionDirective};
use polaris_kernel_directives::effort::parse_model_efforts;
use polaris_kernel_directives::{DirectiveError, ParseLimits, parse};
use polaris_sdk::effort::Effort;

type TestResult = Result<(), Box<dyn Error>>;

fn entry(model: &str, lo: Effort, hi: Effort) -> ModelEffort {
    ModelEffort {
        model: model.into(),
        lo,
        hi,
    }
}

#[test]
fn named_and_numeric_efforts_are_equal() -> TestResult {
    assert_eq!(
        parse_model_efforts("nano(1-3)")?,
        parse_model_efforts("nano(low-high)")?
    );
    assert_eq!(
        parse_model_efforts("nano(1-3)")?,
        vec![entry("nano", Effort::Low, Effort::High)]
    );
    Ok(())
}

#[test]
fn single_effort_means_lo_equals_hi() -> TestResult {
    assert_eq!(
        parse_model_efforts("nano(medium)")?,
        vec![entry("nano", Effort::Medium, Effort::Medium)]
    );
    assert_eq!(
        parse_model_efforts("nano(5)")?,
        vec![entry("nano", Effort::Max, Effort::Max)]
    );
    Ok(())
}

#[test]
fn mixed_forms_parse() -> TestResult {
    assert_eq!(
        parse_model_efforts("nano(1-medium)")?,
        vec![entry("nano", Effort::Low, Effort::Medium)]
    );
    assert_eq!(
        parse_model_efforts("nano( 1 - 3 )")?,
        vec![entry("nano", Effort::Low, Effort::High)]
    );
    assert_eq!(
        parse_model_efforts("nano(XHIGH-max)")?,
        vec![entry("nano", Effort::XHigh, Effort::Max)]
    );
    Ok(())
}

#[test]
fn reversed_range_is_rejected() {
    assert!(matches!(
        parse_model_efforts("nano(high-low)"),
        Err(DirectiveError::BadEffortRange { .. })
    ));
    assert!(matches!(
        parse_model_efforts("nano(0-2)"),
        Err(DirectiveError::BadEffortRange { .. })
    ));
    assert!(matches!(
        parse_model_efforts("nano(1-2-3)"),
        Err(DirectiveError::BadEffortRange { .. })
    ));
    assert!(matches!(
        parse_model_efforts("nano"),
        Err(DirectiveError::BadArg { .. })
    ));
}

#[test]
fn nested_parens_do_not_leak_between_entries() -> TestResult {
    assert_eq!(
        parse_model_efforts("a(1-2), b(4-5)")?,
        vec![
            entry("a", Effort::Low, Effort::Medium),
            entry("b", Effort::XHigh, Effort::Max)
        ]
    );
    Ok(())
}

#[test]
fn quoted_model_id_with_slash_parses() -> TestResult {
    assert_eq!(
        parse_model_efforts("\"nvidia/nemotron-3-super-120b-a12b\"(high-max)")?,
        vec![entry(
            "nvidia/nemotron-3-super-120b-a12b",
            Effort::High,
            Effort::Max
        )]
    );
    assert_eq!(
        parse_model_efforts("nvidia/x(2)")?,
        vec![entry("nvidia/x", Effort::Medium, Effort::Medium)]
    );
    Ok(())
}

#[test]
fn subagent_models_directive_end_to_end() -> TestResult {
    let parsed = parse(
        "$subagent-models(nano(1-2), \"nvidia/nemotron-3-super-120b-a12b\"(high-max)) plan it",
        &ParseLimits::default(),
    )?;
    let got: Vec<_> = parsed.directives.into_iter().map(|d| d.value).collect();
    assert_eq!(
        got,
        vec![SessionDirective::SubagentModels(vec![
            entry("nano", Effort::Low, Effort::Medium),
            entry(
                "nvidia/nemotron-3-super-120b-a12b",
                Effort::High,
                Effort::Max
            ),
        ])]
    );
    assert_eq!(parsed.prose, "plan it");
    Ok(())
}
