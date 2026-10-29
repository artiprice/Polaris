use xtask::calibration::{CalibrationRunConfig, create_binary_calibration};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn config(root: &std::path::Path) -> CalibrationRunConfig {
    CalibrationRunConfig {
        fixtures: root.join("labels.jsonl"),
        predictions: root.join("outputs.jsonl"),
        checkpoint: root.join("checkpoint"),
        output: root.join("candidate.toml"),
        schema: "safety.command".into(),
        version: 1,
        minimum_precision_ppm: 1_000_000,
        minimum_positive: 1,
    }
}
fn inputs(root: &std::path::Path) -> TestResult {
    std::fs::write(
        root.join("labels.jsonl"),
        b"{\"id\":\"a\",\"positive\":true}\n{\"id\":\"b\",\"positive\":false}\n",
    )?;
    std::fs::write(
        root.join("outputs.jsonl"),
        b"{\"id\":\"a\",\"positive_ppm\":900000}\n{\"id\":\"b\",\"positive_ppm\":100000}\n",
    )?;
    std::fs::write(
        root.join("checkpoint"),
        b"synthetic checkpoint bytes for artifact contract only",
    )?;
    Ok(())
}
#[test]
fn runner_hashes_exact_joined_inputs_and_refuses_clobber() -> TestResult {
    let root = tempfile::tempdir()?;
    inputs(root.path())?;
    let config = config(root.path());
    create_binary_calibration(&config)?;
    let raw = std::fs::read(&config.output)?;
    let report: toml::Value = toml::from_str(std::str::from_utf8(&raw)?)?;
    assert_eq!(
        report.get("status").and_then(toml::Value::as_str),
        Some("measured_candidate")
    );
    assert_eq!(
        report
            .get("predictions_blake3")
            .and_then(toml::Value::as_str),
        Some(
            blake3::hash(&std::fs::read(&config.predictions)?)
                .to_hex()
                .as_str()
        )
    );
    assert_eq!(
        report
            .get("report")
            .and_then(|v| v.get("threshold_ppm"))
            .and_then(toml::Value::as_integer),
        Some(900000)
    );
    assert!(create_binary_calibration(&config).is_err());
    assert_eq!(std::fs::read(&config.output)?, raw);
    Ok(())
}
#[test]
fn runner_refuses_incomplete_observations_and_unmet_precision() -> TestResult {
    let root = tempfile::tempdir()?;
    inputs(root.path())?;
    let mut config = config(root.path());
    config.minimum_positive = 2;
    assert!(create_binary_calibration(&config).is_err());
    assert!(!config.output.exists());
    config.minimum_positive = 1;
    std::fs::write(
        &config.predictions,
        b"{\"id\":\"a\",\"positive_ppm\":900000}\n",
    )?;
    assert!(create_binary_calibration(&config).is_err());
    assert!(!config.output.exists());
    Ok(())
}

#[test]
fn calibration_cli_requires_explicit_unique_options() -> TestResult {
    let root = tempfile::tempdir()?;
    inputs(root.path())?;
    let config = config(root.path());
    let args = vec![
        "--fixtures".to_owned(),
        config.fixtures.to_string_lossy().into_owned(),
        "--predictions".into(),
        config.predictions.to_string_lossy().into_owned(),
        "--checkpoint".into(),
        config.checkpoint.to_string_lossy().into_owned(),
        "--output".into(),
        config.output.to_string_lossy().into_owned(),
        "--schema".into(),
        config.schema,
        "--version".into(),
        "1".into(),
        "--minimum-precision-ppm".into(),
        "1000000".into(),
        "--minimum-positive".into(),
        "1".into(),
    ];
    let mut duplicate = args.clone();
    duplicate.extend(["--version".into(), "2".into()]);
    assert!(xtask::calibration::run_cli(duplicate.into_iter()).is_err());
    assert!(!config.output.exists());
    xtask::calibration::run_cli(args.into_iter())?;
    assert!(config.output.exists());
    Ok(())
}
