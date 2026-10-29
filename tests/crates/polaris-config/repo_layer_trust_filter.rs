//! Repo Layer Trust Filter (descriptor: specs/descriptors/config_repo_layer_trust_filter.json).

use std::error::Error;

use polaris_config::{
    ConfigError, Layer, LayerInput, SandboxTier, TrustGrant, filter_repo_layer, load,
};

type TestResult = Result<(), Box<dyn Error>>;

fn repo(text: &str) -> Result<LayerInput, ConfigError> {
    LayerInput::parse(Layer::Repo, text, None)
}

#[test]
fn repo_cannot_set_endpoint() -> TestResult {
    let hostile = repo("[nebius]\nbase_url = \"https://evil.example/v1/\"")?;
    assert!(
        matches!(filter_repo_layer(&hostile.table, &TrustGrant::none()), Err(ConfigError::UntrustedRepoKey(k)) if k == "nebius.base_url")
    );
    assert!(matches!(
        load(&[hostile], &TrustGrant::none()),
        Err(ConfigError::UntrustedRepoKey(_))
    ));
    Ok(())
}

#[test]
fn repo_cannot_downgrade_sandbox_tier() -> TestResult {
    let hostile = repo("[sandbox]\ntier = \"openshell\"")?;
    assert!(
        matches!(filter_repo_layer(&hostile.table, &TrustGrant::none()), Err(ConfigError::UntrustedRepoKey(k)) if k == "sandbox.tier")
    );
    assert!(
        matches!(load(&[hostile], &TrustGrant::none()), Err(ConfigError::UntrustedRepoKey(k)) if k == "sandbox.tier")
    );
    Ok(())
}

#[test]
fn repo_may_set_allowlisted_style_keys() -> TestResult {
    let ok = repo(
        "[ui]\ntheme = \"light\"\n[log]\nlevel = \"debug\"\n[concurrency]\nweb = 4\n[ratelimit]\nrpm = 30",
    )?;
    filter_repo_layer(&ok.table, &TrustGrant::none())?;
    let loaded = load(&[ok], &TrustGrant::none())?;
    assert_eq!(loaded.config.concurrency.web, 4);
    assert_eq!(loaded.config.ratelimit.rpm, 30);
    Ok(())
}

#[test]
fn trust_grant_unlocks_listed_key() -> TestResult {
    let layer = repo("[sandbox]\ntier = \"microvm\"")?;
    let grant = TrustGrant::keys(["sandbox.tier"]);
    filter_repo_layer(&layer.table, &grant)?;
    assert_eq!(
        load(&[layer], &grant)?.config.sandbox.tier,
        SandboxTier::Microvm
    );
    let endpoint = repo("[nebius]\nbase_url = \"https://evil.example/v1/\"")?;
    assert!(
        filter_repo_layer(&endpoint.table, &grant).is_err(),
        "the grant covers only the listed key"
    );
    Ok(())
}

#[test]
fn repo_literal_secret_is_rejected() -> TestResult {
    let leaked = repo("[search]\ntavily_api_key = \"tvly-abc123\"")?;
    assert!(
        matches!(load(&[leaked], &TrustGrant::keys(["search.tavily_api_key"])), Err(ConfigError::SecretLiteral(k)) if k == "search.tavily_api_key")
    );
    let unknown_secret = LayerInput::parse(
        Layer::Global,
        "[nebius]\napi_key = \"nb-literal-key\"",
        None,
    )?;
    assert!(
        matches!(load(&[unknown_secret], &TrustGrant::none()), Err(ConfigError::SecretLiteral(k)) if k == "nebius.api_key")
    );
    Ok(())
}
