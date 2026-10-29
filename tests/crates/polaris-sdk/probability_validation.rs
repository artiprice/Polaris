//! Probability Validation (descriptor: specs/descriptors/sdk_probability_validation.json).

use std::error::Error;

use polaris_sdk::system_one::Probability;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn rejects_nan() {
    assert!(Probability::new(f32::NAN).is_err());
}

#[test]
fn rejects_above_one_and_below_zero() {
    for bad in [1.0001_f32, -0.0001, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(Probability::new(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn accepts_bounds() -> TestResult {
    assert_eq!(Probability::new(0.0)?.get(), 0.0);
    assert_eq!(Probability::new(1.0)?.get(), 1.0);
    assert_eq!(Probability::new(0.25)?.get(), 0.25);
    Ok(())
}

#[test]
fn ordering_is_total_for_valid_values() -> TestResult {
    let mut values = [0.9_f32, 0.0, 0.5, 1.0, 0.1, 0.5]
        .into_iter()
        .map(Probability::new)
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    let sorted: Vec<f32> = values.iter().map(|p| p.get()).collect();
    assert_eq!(sorted, vec![0.0, 0.1, 0.5, 0.5, 0.9, 1.0]);
    assert_eq!(values.iter().max().map(|p| p.get()), Some(1.0));
    Ok(())
}

#[test]
fn serde_rejects_out_of_range() -> TestResult {
    assert!(serde_json::from_str::<Probability>("1.5").is_err());
    assert!(serde_json::from_str::<Probability>("-0.5").is_err());
    let p: Probability = serde_json::from_str("0.75")?;
    assert_eq!(p.get(), 0.75);
    assert_eq!(serde_json::to_string(&p)?, "0.75");
    Ok(())
}
