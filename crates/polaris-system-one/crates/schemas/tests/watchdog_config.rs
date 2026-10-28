use polaris_system_one_schemas::{SchemaError, SchemaSet};

#[test]
fn builtin_watchdog_config_is_explicit_and_bounded() -> Result<(), String> {
    let schemas = SchemaSet::load_builtin().map_err(|error| error.to_string())?;
    let config = schemas
        .watchdog_config()
        .ok_or_else(|| "embedded deterministic watchdog config missing".to_owned())?;
    if config.version != 1
        || config.max_step_bytes == 0
        || config.max_step_bytes > 1_048_576
        || config.window_size == 0
        || config.window_size > 128
        || config.shingle_size == 0
        || config.shingle_size > 128
        || config.near_duplicate.numerator == 0
        || config.near_duplicate.denominator == 0
        || config.near_duplicate.numerator > config.near_duplicate.denominator
    {
        return Err(format!(
            "embedded watchdog config outside contract: {config:?}"
        ));
    }
    Ok(())
}

#[test]
fn invalid_deterministic_watchdog_config_is_rejected() -> Result<(), String> {
    let source = include_str!("../src/builtin.toml");
    let invalid = source.replace(
        "near_duplicate={numerator=4,denominator=5}",
        "near_duplicate={numerator=0,denominator=5}",
    );
    let result = SchemaSet::from_toml(&invalid);
    if !matches!(result, Err(SchemaError::InvalidSchema { .. })) {
        return Err("zero near-duplicate ratio should fail schema validation".to_owned());
    }
    Ok(())
}
