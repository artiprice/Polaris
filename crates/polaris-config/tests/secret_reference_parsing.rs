//! Secret Reference Parsing (descriptor: specs/descriptors/config_secret_reference_parsing.json).

use std::error::Error;

use polaris_config::{ConfigError, Layer, LayerInput, SecretRef, TrustGrant, load};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn parses_keyring_scheme() -> TestResult {
    assert_eq!(
        SecretRef::parse("keyring:polaris/nebius")?,
        SecretRef::Keyring {
            service: "polaris".into(),
            user: "nebius".into()
        }
    );
    let global = LayerInput::parse(
        Layer::Global,
        "[nebius]\napi_key = \"keyring:polaris/nebius\"",
        None,
    )?;
    let loaded = load(&[global], &TrustGrant::none())?;
    assert_eq!(
        loaded.config.nebius.api_key,
        Some(SecretRef::Keyring {
            service: "polaris".into(),
            user: "nebius".into()
        })
    );
    Ok(())
}

#[test]
fn parses_env_scheme() -> TestResult {
    assert_eq!(
        SecretRef::parse("env:NEBIUS_API_KEY")?,
        SecretRef::Env("NEBIUS_API_KEY".into())
    );
    assert!(SecretRef::parse("env:lower_case").is_err());
    Ok(())
}

#[test]
fn rejects_bare_literal() {
    for literal in ["nb-1234567890abcdef", "sk-live-xyz", "Bearer abc"] {
        assert!(
            matches!(
                SecretRef::parse(literal),
                Err(ConfigError::SecretLiteral(_))
            ),
            "{literal}"
        );
    }
}

#[test]
fn rejects_empty_name() {
    for bad in [
        "keyring:",
        "keyring:/user",
        "keyring:service/",
        "keyring:service",
        "env:",
    ] {
        assert!(SecretRef::parse(bad).is_err(), "{bad}");
    }
}
