//! Config Layer Merge (descriptor: specs/descriptors/config_layer_merge.json).

use std::error::Error;
use std::path::PathBuf;

use polaris_config::{ConfigError, Layer, LayerInput, SandboxTier, StorageLevel, TrustGrant, load};
use polaris_sdk::effort::Effort;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn layer(layer: Layer, text: &str) -> Result<LayerInput, ConfigError> {
    LayerInput::parse(layer, text, Some(PathBuf::from(format!("{layer:?}.toml"))))
}

#[test]
fn later_layer_overrides_earlier() -> TestResult {
    // Deliberately out of order: precedence comes from the layer, not the slice position.
    let layers = [
        layer(Layer::Session, "[ratelimit]\nrpm = 5")?,
        layer(Layer::Global, "[ratelimit]\nrpm = 2\ntpm = 2000")?,
        layer(Layer::Env, "[ratelimit]\nrpm = 4")?,
        layer(Layer::Repo, "[ratelimit]\nrpm = 3\ntpm = 3000")?,
    ];
    let loaded = load(&layers, &TrustGrant::none())?;
    assert_eq!(loaded.config.ratelimit.rpm, 5);
    assert_eq!(loaded.config.ratelimit.tpm, 3000);
    Ok(())
}

#[test]
fn tables_merge_scalars_replace() -> TestResult {
    let layers = [
        layer(
            Layer::Global,
            "[concurrency]\nweb = 3\ninference = 2\n[storage]\nlevel = \"isolated\"",
        )?,
        layer(Layer::Repo, "[concurrency]\nweb = 9")?,
    ];
    let loaded = load(&layers, &TrustGrant::none())?;
    assert_eq!(loaded.config.concurrency.web, 9, "repo scalar replaces");
    assert_eq!(
        loaded.config.concurrency.inference, 2,
        "sibling key from the global table survives"
    );
    assert_eq!(loaded.config.storage.level, StorageLevel::Isolated);
    Ok(())
}

#[test]
fn provenance_reports_winning_layer() -> TestResult {
    let layers = [
        layer(Layer::Global, "[ratelimit]\nrpm = 2")?,
        layer(Layer::Env, "[ratelimit]\nrpm = 4")?,
    ];
    let loaded = load(&layers, &TrustGrant::none())?;
    assert_eq!(
        loaded.provenance.layer_of("ratelimit.rpm"),
        Some(Layer::Env)
    );
    assert_eq!(
        loaded.provenance.origin_of("ratelimit.rpm"),
        Some(PathBuf::from("Env.toml").as_path())
    );
    assert_eq!(
        loaded.provenance.layer_of("ratelimit.tpm"),
        Some(Layer::Defaults)
    );
    assert_eq!(loaded.provenance.layer_of("no.such.key"), None);
    Ok(())
}

#[test]
fn unknown_key_is_rejected() -> TestResult {
    let layers = [layer(Layer::Global, "[ratelimit]\nrpm = 2\nburst = 9")?];
    assert!(
        matches!(load(&layers, &TrustGrant::none()), Err(ConfigError::UnknownKey(k)) if k == "ratelimit.burst")
    );
    let layers = [layer(Layer::Global, "[nonsense]\na = 1")?];
    assert!(
        matches!(load(&layers, &TrustGrant::none()), Err(ConfigError::UnknownKey(k)) if k == "nonsense.a")
    );
    Ok(())
}

#[test]
fn invalid_value_is_rejected() -> TestResult {
    for (text, key) in [
        ("[ratelimit]\nrpm = \"fast\"", "ratelimit.rpm"),
        ("[sandbox]\ntier = \"host\"", "sandbox.tier"),
        ("[nebius.effort]\nlow = \"has space\"", "nebius.effort.low"),
        ("[budget]\nusd = \"five\"", "budget.usd"),
    ] {
        let layers = [layer(Layer::Global, text)?];
        assert!(
            matches!(load(&layers, &TrustGrant::none()), Err(ConfigError::Invalid { key: k, .. }) if k == key),
            "{text:?} should be Invalid for {key}"
        );
    }
    Ok(())
}

#[test]
fn defaults_only_is_valid() -> TestResult {
    let loaded = load(&[], &TrustGrant::none())?;
    let cfg = &loaded.config;
    assert_eq!(cfg.sandbox.tier, SandboxTier::Auto);
    assert_eq!(cfg.storage.level, StorageLevel::Global);
    assert_eq!(
        cfg.nebius.base_url,
        "https://api.tokenfactory.nebius.com/v1/"
    );
    assert_eq!(
        cfg.nebius.effort.model(Effort::Low).as_str(),
        "nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B"
    );
    assert_eq!(
        cfg.nebius.effort.model(Effort::Max).as_str(),
        "nvidia/Nemotron-3-Ultra-550b-a55b"
    );
    assert!(
        cfg.nebius.api_key.is_none(),
        "no secret reference until configured"
    );
    assert!(cfg.concurrency.inference > 0 && cfg.ratelimit.rpm > 0);
    Ok(())
}

#[test]
fn budget_usd_accepts_integer_and_decimal_text() -> TestResult {
    let text = load(
        &[layer(Layer::Global, "[budget]\nusd = \"12.50\"")?],
        &TrustGrant::none(),
    )?;
    assert_eq!(text.config.budget.usd.as_deref(), Some("12.50"));
    let int = load(
        &[layer(Layer::Global, "[budget]\nusd = 5")?],
        &TrustGrant::none(),
    )?;
    assert_eq!(int.config.budget.usd.as_deref(), Some("5"));
    let float = load(
        &[layer(Layer::Global, "[budget]\nusd = 0.1")?],
        &TrustGrant::none(),
    );
    assert!(
        matches!(float, Err(ConfigError::Invalid { key, .. }) if key == "budget.usd"),
        "floats round"
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_merge_with_itself_is_idempotent(rpm in 1u32..100_000, web in 1u32..64, isolated in any::<bool>()) {
        let text = format!(
            "[ratelimit]\nrpm = {rpm}\n[concurrency]\nweb = {web}\n[storage]\nlevel = \"{}\"",
            if isolated { "isolated" } else { "global" }
        );
        let once = layer(Layer::Global, &text).and_then(|l| load(&[l], &TrustGrant::none()));
        let twice = layer(Layer::Global, &text).and_then(|a| layer(Layer::Global, &text).and_then(|b| load(&[a, b], &TrustGrant::none())));
        prop_assert!(once.is_ok() && twice.is_ok());
        if let (Ok(once), Ok(twice)) = (once, twice) {
            prop_assert_eq!(once.config, twice.config);
        }
    }
    #[test]
    fn prop_highest_layer_wins_per_key(
        rpm in proptest::collection::vec(proptest::option::of(1u32..10_000), 4),
        web in proptest::collection::vec(proptest::option::of(1u32..64), 4),
    ) {
        let order = [Layer::Global, Layer::Repo, Layer::Env, Layer::Session];
        let mut layers = Vec::new();
        for (i, kind) in order.iter().enumerate().rev() {
            let mut text = String::new();
            if let Some(Some(v)) = rpm.get(i) { text.push_str(&format!("[ratelimit]
rpm = {v}
")); }
            if let Some(Some(v)) = web.get(i) { text.push_str(&format!("[concurrency]
web = {v}
")); }
            layers.push(layer(*kind, &text).map_err(|e| TestCaseError::fail(e.to_string()))?);
        }
        let loaded = load(&layers, &TrustGrant::none()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let defaults = load(&[], &TrustGrant::none()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let winner = |values: &[Option<u32>]| order.iter().zip(values).rev().find_map(|(l, v)| v.map(|v| (*l, v)));
        match winner(&rpm) {
            Some((l, v)) => { prop_assert_eq!(loaded.config.ratelimit.rpm, v); prop_assert_eq!(loaded.provenance.layer_of("ratelimit.rpm"), Some(l)); }
            None => { prop_assert_eq!(loaded.config.ratelimit.rpm, defaults.config.ratelimit.rpm); prop_assert_eq!(loaded.provenance.layer_of("ratelimit.rpm"), Some(Layer::Defaults)); }
        }
        match winner(&web) {
            Some((l, v)) => { prop_assert_eq!(loaded.config.concurrency.web, v); prop_assert_eq!(loaded.provenance.layer_of("concurrency.web"), Some(l)); }
            None => { prop_assert_eq!(loaded.config.concurrency.web, defaults.config.concurrency.web); prop_assert_eq!(loaded.provenance.layer_of("concurrency.web"), Some(Layer::Defaults)); }
        }
    }
}
