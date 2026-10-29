//! Golden Provenance (descriptor: specs/descriptors/config_golden_provenance.json).

use std::error::Error;
use std::fmt::Write as _;
use std::fs;

use polaris_config::{EnvSnapshot, SessionOverrides, TrustGrant, discover_from, load, read_layers};

type TestResult = Result<(), Box<dyn Error>>;

const GLOBAL: &str = "[nebius]\napi_key = \"keyring:polaris/nebius\"\n[ratelimit]\nrpm = 40\ntpm = 90000\n[log]\nlevel = \"warn\"\n";
const REPO: &str = "[ratelimit]\nrpm = 20\n[ui]\ntheme = \"light\"\n";

#[test]
fn golden_provenance_snapshot() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(home.join(".polaris"))?;
    fs::create_dir_all(repo.join(".polaris"))?;
    fs::write(home.join(".polaris").join("config.toml"), GLOBAL)?;
    fs::write(repo.join(".polaris").join("config.toml"), REPO)?;

    let dirs = discover_from(&repo, &home)?;
    let env = EnvSnapshot::from_pairs([
        ("POLARIS_CONCURRENCY__WEB", "12"),
        ("POLARIS_BUDGET__USD", "5"),
    ]);
    let mut session = SessionOverrides::default();
    session.set("sandbox.tier", toml::Value::String("contree".into()));
    let loaded = load(&read_layers(&dirs, &env, &session)?, &TrustGrant::none())?;

    let global_file = dirs.global.join("config.toml");
    let repo_file = dirs.repo.as_ref().map(|r| r.join("config.toml"));
    let mut out = String::new();
    for (key, layer, origin) in loaded.provenance.iter() {
        let label = match origin {
            None => "-",
            Some(path) if path == global_file => "~/.polaris/config.toml",
            Some(path) if Some(path) == repo_file.as_deref() => ".polaris/config.toml",
            Some(_) => "<other>",
        };
        writeln!(out, "{key:<24} {layer:?} {label}")?;
    }
    writeln!(out, "\n{:#?}", loaded.config)?;
    insta::assert_snapshot!("golden_provenance", out);
    Ok(())
}
