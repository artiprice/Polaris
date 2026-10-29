// Unit tests of crates/polaris-devenv/crates/nix/src/lib.rs (`tests`); compiled into that module via #[path].
use super::{
    DEFAULT_MAX_JSON_BYTES, EnvMap, FlakeLint, NixError, ProjectManifests, cache_key, eval_command,
    flake_draft_prompt, load_cached_env, parse_dev_env, sanitize, store_cached_env,
    validate_flake_text,
};
use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct MemoryCache(Mutex<HashMap<[u8; 32], Vec<u8>>>);

impl ExactCacheStore for MemoryCache {
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
        self.0
            .lock()
            .map(|entries| entries.get(key).cloned())
            .map_err(|error| CacheStoreError::Io(error.to_string()))
    }

    fn get_bounded(
        &self,
        key: &[u8; 32],
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let entries = self
            .0
            .lock()
            .map_err(|error| CacheStoreError::Io(error.to_string()))?;
        let Some(value) = entries.get(key) else {
            return Ok(None);
        };
        if value.len() > max_bytes {
            return Err(CacheStoreError::TooLarge {
                bytes: u64::try_from(value.len()).unwrap_or(u64::MAX),
                limit: u64::try_from(max_bytes).unwrap_or(u64::MAX),
            });
        }
        Ok(Some(value.clone()))
    }

    fn put(&self, key: [u8; 32], value: &[u8], _scope: CacheScope) -> Result<(), CacheStoreError> {
        self.0
            .lock()
            .map(|mut entries| {
                entries.insert(key, value.to_vec());
            })
            .map_err(|error| CacheStoreError::Io(error.to_string()))
    }

    fn clear_scope(&self, _scope: &CacheScope) -> Result<u64, CacheStoreError> {
        Ok(0)
    }
}

#[test]
fn parses_only_exported_string_values() {
    let input = br#"{"variables":{"PATH":{"type":"exported","value":"/nix/store/bin"},"functions":{"type":"function","value":"() {}"},"items":{"type":"exported","value":["x"]},"EMPTY":{"type":"exported","value":""}}}"#;
    let parsed = parse_dev_env(input, DEFAULT_MAX_JSON_BYTES);
    assert!(parsed.is_ok());
    let Ok(parsed) = parsed else { return };
    assert_eq!(
        parsed.variables().get("PATH").map(String::as_str),
        Some("/nix/store/bin")
    );
    assert_eq!(
        parsed.variables().get("EMPTY").map(String::as_str),
        Some("")
    );
    assert!(!parsed.variables().contains_key("functions"));
    assert!(!parsed.variables().contains_key("items"));
}

#[test]
fn rejects_oversized_json_before_parsing() {
    let result = parse_dev_env(b"{}", 1);
    assert!(matches!(
        result,
        Err(NixError::JsonTooLarge {
            limit: 1,
            actual: 2
        })
    ));
}

#[test]
fn rejects_missing_variables_object() {
    assert!(matches!(
        parse_dev_env(br#"{"other":{}}"#, 64),
        Err(NixError::MissingVariables)
    ));
}

#[test]
fn removes_unsafe_and_secret_named_environment_variables() {
    let input = br#"{"variables":{"PATH":{"type":"exported","value":"/nix/store/bin:/usr/bin"},"HOME":{"type":"exported","value":"/homeless-shelter"},"NIX_BUILD_CORES":{"type":"exported","value":"8"},"API_TOKEN":{"type":"exported","value":"secret"},"ACCESS":{"type":"exported","value":"sk-live-secret"},"SHELL":{"type":"exported","value":"/bin/bash"}}}"#;
    let parsed = parse_dev_env(input, DEFAULT_MAX_JSON_BYTES);
    assert!(parsed.is_ok());
    let Ok(parsed) = parsed else { return };
    let clean = sanitize(parsed);
    assert_eq!(clean.variables().len(), 1);
    assert_eq!(
        clean.variables().get("PATH").map(String::as_str),
        Some("/nix/store/bin")
    );
}

#[test]
fn builds_argv_without_shell_interpolation() {
    let command = eval_command("/workspace with spaces#devShells.x86_64-linux.default");
    assert!(command.is_ok());
    let Ok(command) = command else { return };
    assert_eq!(command.program, "nix");
    assert_eq!(
        command.args,
        [
            "print-dev-env",
            "--json",
            "--extra-experimental-features",
            "nix-command flakes",
            "/workspace with spaces#devShells.x86_64-linux.default"
        ]
    );
}

#[test]
fn rejects_control_characters_in_flake_reference() {
    assert!(matches!(
        eval_command("flake\n--option"),
        Err(NixError::InvalidFlakeRef)
    ));
}

#[test]
fn rejects_leading_option_in_flake_reference() {
    assert!(matches!(
        eval_command("--option sandbox false"),
        Err(NixError::InvalidFlakeRef)
    ));
}

#[test]
fn cache_key_changes_with_lock_system_and_version() {
    let baseline = cache_key(b"{}", "x86_64-linux", "2.33.2");
    let changed_lock = cache_key(b"{\"nodes\":{}}", "x86_64-linux", "2.33.2");
    let changed_system = cache_key(b"{}", "aarch64-linux", "2.33.2");
    let changed_version = cache_key(b"{}", "x86_64-linux", "2.34.0");
    assert!(
        baseline.is_ok()
            && changed_lock.is_ok()
            && changed_system.is_ok()
            && changed_version.is_ok()
    );
    if let (Ok(base), Ok(lock), Ok(system), Ok(version)) =
        (baseline, changed_lock, changed_system, changed_version)
    {
        assert_ne!(base, lock);
        assert_ne!(base, system);
        assert_ne!(base, version);
    }
}

#[test]
fn cache_key_rejects_empty_system_and_version() {
    assert!(matches!(
        cache_key(b"{}", "", "2.33.2"),
        Err(NixError::InvalidCacheInput("system"))
    ));
    assert!(matches!(
        cache_key(b"{}", "x86_64-linux", " "),
        Err(NixError::InvalidCacheInput("Nix version"))
    ));
}

#[test]
fn exact_cache_round_trip_revalidates_sanitized_environment() {
    let key = cache_key(b"{}", "x86_64-linux", "2.33.2");
    assert!(key.is_ok());
    let Ok(key) = key else { return };
    let env = EnvMap(std::collections::BTreeMap::from([
        ("PATH".to_owned(), "/nix/store/abc/bin".to_owned()),
        ("RUSTC".to_owned(), "/nix/store/rustc/bin/rustc".to_owned()),
    ]));
    let cache = MemoryCache::default();
    assert!(store_cached_env(&cache, &key, &env, CacheScope::Global).is_ok());
    assert_eq!(load_cached_env(&cache, &key).ok().flatten(), Some(env));
}

#[test]
fn exact_cache_rejects_unsanitized_environment_values() {
    let key = cache_key(b"{}", "x86_64-linux", "2.33.2");
    assert!(key.is_ok());
    let Ok(key) = key else { return };
    let cache = MemoryCache::default();
    if let Ok(mut entries) = cache.0.lock() {
        entries.insert(*key.as_bytes(), br#"{"HOME":"/host/home"}"#.to_vec());
    }
    assert!(matches!(
        load_cached_env(&cache, &key),
        Err(NixError::InvalidCachedEnv)
    ));
}

#[test]
fn draft_prompt_bounds_and_encodes_untrusted_project_data() {
    let manifests = ProjectManifests {
        cargo_toml: "[package]\nname = \"example\"".to_owned(),
        requirements_txt: Some("numpy==2.1.0".to_owned()),
        cuda_sources: vec![("src/kernel.cu".to_owned(), "// sample".to_owned())],
    };
    let prompt = flake_draft_prompt(&manifests);
    assert!(prompt.is_ok());
    let Ok(prompt) = prompt else { return };
    assert!(prompt.contains("PROJECT_MANIFESTS_JSON"));
    assert!(prompt.contains("\"src/kernel.cu\""));
    assert!(matches!(
        flake_draft_prompt(&ProjectManifests {
            cuda_sources: vec![("../secrets.txt".to_owned(), "secret".to_owned())],
            ..manifests
        }),
        Err(NixError::UnsafeSourcePath)
    ));
}

#[test]
fn rejects_unhashed_fetchurl_and_host_paths() {
    assert!(matches!(
        validate_flake_text("builtins.fetchurl { url = \"https://example.invalid/x\"; }"),
        Err(NixError::UnsafeFlake(_))
    ));
    assert!(matches!(
        validate_flake_text("{ src = /home/alice/source; }"),
        Err(NixError::UnsafeFlake(_))
    ));
}

#[test]
fn accepts_pinned_fetchurl_and_simple_flake_text() {
    let result = validate_flake_text(
        "{ src = builtins.fetchurl { url = \"https://example.invalid/x\"; sha256 = \"abc\"; }; }",
    );
    assert!(matches!(result, Ok(FlakeLint { warnings }) if warnings.is_empty()));
}

#[test]
fn builds_flake_prompt_from_bounded_manifest_data() {
    let manifests = ProjectManifests {
        cargo_toml: "[package]\nname = \"sample\"".to_owned(),
        requirements_txt: Some("numpy==2.1.0".to_owned()),
        cuda_sources: vec![(
            "src/kernel.cu".to_owned(),
            "__global__ void f() {}".to_owned(),
        )],
    };
    let result = flake_draft_prompt(&manifests);
    assert!(
        matches!(result, Ok(prompt) if prompt.contains("PROJECT_MANIFESTS_JSON") && prompt.contains("src/kernel.cu"))
    );
}

#[test]
fn rejects_escape_paths_in_flake_prompt_sources() {
    let manifests = ProjectManifests {
        cargo_toml: "[package]".to_owned(),
        requirements_txt: None,
        cuda_sources: vec![("../outside.cu".to_owned(), "source".to_owned())],
    };
    assert!(matches!(
        flake_draft_prompt(&manifests),
        Err(NixError::UnsafeSourcePath)
    ));
}
