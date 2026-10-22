//! Fuzz target `config_toml_layer` (`polaris-config`): arbitrary bytes as a repository
//! `.polaris/config.toml` must yield `Ok` or `Err` without panicking, and a layer that loads under
//! no trust grant can only have set allow-listed keys (endpoint, credentials and sandbox tier stay
//! at their defaults).
#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_config::{KNOWN_KEYS, Layer, LayerInput, REPO_ALLOWED, SandboxTier, TrustGrant, load};

fn allowed(key: &str) -> bool {
    REPO_ALLOWED.iter().any(|pattern| {
        pattern
            .strip_suffix(".*")
            .map_or(*pattern == key, |prefix| {
                key.starts_with(&format!("{prefix}."))
            })
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(layer) = LayerInput::parse(Layer::Repo, text, None) else {
        return;
    };
    if let Ok(loaded) = load(&[layer], &TrustGrant::none()) {
        for (key, source, _) in loaded.provenance.iter() {
            assert!(KNOWN_KEYS.contains(&key), "unknown key {key} survived");
            assert!(
                source != Layer::Repo || allowed(key),
                "untrusted repo set {key}"
            );
        }
        assert_eq!(loaded.config.sandbox.tier, SandboxTier::Auto);
        assert_eq!(
            loaded.config.nebius.base_url,
            "https://api.tokenfactory.nebius.com/v1/"
        );
        assert!(loaded.config.nebius.api_key.is_none());
    }
});
