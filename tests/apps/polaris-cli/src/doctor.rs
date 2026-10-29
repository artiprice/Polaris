// Unit tests of apps/polaris-cli/src/doctor.rs (`tests`); compiled into that module via #[path].
use super::*;

#[test]
fn every_setup_item_has_a_probe() -> Result<(), CliError> {
    let report = assemble(BTreeMap::new())?;
    let inventory: toml::Value =
        toml::from_str(INVENTORY).map_err(|_| CliError::Unavailable("inventory"))?;
    let entries = inventory
        .get("member_setup")
        .and_then(toml::Value::as_array)
        .ok_or(CliError::Unavailable("items"))?;
    let items = report
        .get("items")
        .and_then(serde_json::Value::as_array)
        .ok_or(CliError::Unavailable("report"))?;
    assert_eq!(
        items.len(),
        entries.len(),
        "inventory cannot silently shrink"
    );
    for (item, source) in items.iter().zip(entries) {
        assert_eq!(
            item.get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or(CliError::Unavailable("report id missing"))?,
            source
                .get("id")
                .and_then(toml::Value::as_str)
                .ok_or(CliError::Unavailable("inventory id missing"))?,
            "every authoritative id retained"
        );
        assert_eq!(
            item.get("observation")
                .and_then(|value| value.get("status"))
                .and_then(serde_json::Value::as_str),
            Some("UNKNOWN"),
            "absent probe is not success"
        );
        assert!(
            item.get("onboarding")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| !text.is_empty()),
            "concrete action retained"
        );
    }
    assert_eq!(
        report.get("ready").and_then(serde_json::Value::as_bool),
        Some(false),
        "unknown required item prevents readiness"
    );
    assert!(
        !is_ready(&report),
        "unknown required item blocks CLI readiness"
    );
    Ok(())
}

#[test]
fn optional_missing_item_does_not_block_readiness() -> Result<(), CliError> {
    let inventory: toml::Value =
        toml::from_str(INVENTORY).map_err(|_| CliError::Unavailable("inventory"))?;
    let entries = inventory
        .get("member_setup")
        .and_then(toml::Value::as_array)
        .ok_or(CliError::Unavailable("items"))?;
    let mut probes = BTreeMap::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or(CliError::Unavailable("setup item has no id"))?;
        probes.insert(
            id.to_owned(),
            observation("PASS", "synthetic contract fixture"),
        );
    }
    probes.insert(
        "win-keyring".to_owned(),
        observation("MISSING", "optional credentials are not configured"),
    );

    let report = assemble(probes)?;

    assert_eq!(
        report.get("ready").and_then(serde_json::Value::as_bool),
        Some(true),
        "optional absence does not block setup"
    );
    assert_eq!(
        report
            .get("required_missing")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert!(is_ready(&report));
    Ok(())
}

#[test]
fn credential_metadata_probe_never_renders_secret_values() {
    let secret = std::ffi::OsStr::new("sensitive-value-do-not-render-4950");
    let present = environment_presence(Some(secret));
    assert_eq!(
        present.get("status").and_then(serde_json::Value::as_str),
        Some("PASS"),
        "nonempty is present"
    );
    assert!(
        !present.to_string().contains("4950"),
        "secret payload must never render"
    );
    assert_eq!(
        environment_presence(None)
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("MISSING"),
        "absent is missing"
    );
    assert_eq!(
        environment_presence(Some(std::ffi::OsStr::new("")))
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("MISSING"),
        "empty is missing"
    );
}
