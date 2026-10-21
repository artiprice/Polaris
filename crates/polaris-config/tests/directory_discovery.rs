//! Directory Discovery (descriptor: specs/descriptors/config_directory_discovery.json).

use std::error::Error;
use std::fs;

use polaris_config::{
    ConfigError, EnvSnapshot, Layer, SessionOverrides, TrustGrant, discover_from, load, read_layers,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn finds_nearest_repo_dir_walking_up() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    let repo = tmp.path().join("work space").join("projet-é");
    let nested = repo.join("crates").join("a").join("src");
    fs::create_dir_all(&nested)?;
    fs::create_dir_all(repo.join(".polaris"))?;
    fs::create_dir_all(&home)?;
    let dirs = discover_from(&nested, &home)?;
    assert_eq!(
        dirs.repo.as_deref(),
        Some(dunce::canonicalize(repo.join(".polaris"))?.as_path())
    );
    assert_eq!(dirs.global, home.join(".polaris"));
    Ok(())
}

#[test]
fn no_repo_dir_is_none() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let cwd = tmp.path().join("plain");
    fs::create_dir_all(&cwd)?;
    let dirs = discover_from(&cwd, tmp.path())?;
    assert_eq!(dirs.repo, None);
    Ok(())
}

#[test]
fn reads_bom_and_crlf_files() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".polaris"))?;
    fs::write(
        home.join(".polaris").join("config.toml"),
        "\u{feff}[ratelimit]\r\nrpm = 77\r\n",
    )?;
    let dirs = discover_from(&home, &home)?;
    let layers = read_layers(&dirs, &EnvSnapshot::empty(), &SessionOverrides::default())?;
    assert_eq!(
        layers.iter().map(|l| l.layer).collect::<Vec<_>>(),
        vec![Layer::Global],
        "missing repo file is skipped"
    );
    assert_eq!(load(&layers, &TrustGrant::none())?.config.ratelimit.rpm, 77);
    Ok(())
}

#[test]
fn env_layer_uses_prefix_and_double_underscore() -> TestResult {
    let env = EnvSnapshot::from_pairs([
        ("POLARIS_RATELIMIT__RPM", "600"),
        ("POLARIS_STORAGE__LEVEL", "isolated"),
        ("PATH", "C:\\Windows"),
        ("POLARISX_IGNORED", "1"),
    ]);
    let layer = env.to_layer()?;
    assert_eq!(layer.layer, Layer::Env);
    let loaded = load(&[layer], &TrustGrant::none())?;
    assert_eq!(loaded.config.ratelimit.rpm, 600);
    assert_eq!(
        loaded.provenance.layer_of("storage.level"),
        Some(Layer::Env)
    );
    Ok(())
}

#[test]
fn session_overrides_roundtrip() -> TestResult {
    let mut session = SessionOverrides::default();
    session.set("ratelimit.rpm", toml::Value::Integer(12));
    session.set("sandbox.tier", toml::Value::String("contree".into()));
    let text = session.to_toml()?;
    let back = SessionOverrides::from_toml(&text)?;
    assert_eq!(back, session);
    let loaded = load(&[back.to_layer()?], &TrustGrant::none())?;
    assert_eq!(loaded.config.ratelimit.rpm, 12);
    assert_eq!(
        loaded.provenance.layer_of("sandbox.tier"),
        Some(Layer::Session)
    );
    Ok(())
}

#[test]
fn home_global_dir_is_not_a_repo_dir() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    let project = home.join("projects").join("demo");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(home.join(".polaris"))?;
    fs::create_dir_all(tmp.path().join(".polaris"))?;
    let dirs = discover_from(&project, &home)?;
    assert_eq!(
        dirs.repo, None,
        "the walk stops at home: ~/.polaris is global, above home is never a project"
    );
    Ok(())
}

#[test]
fn env_conflicting_paths_are_invalid() {
    let env =
        EnvSnapshot::from_pairs([("POLARIS_RATELIMIT", "1"), ("POLARIS_RATELIMIT__RPM", "2")]);
    assert!(matches!(env.to_layer(), Err(ConfigError::Invalid { .. })));
}

#[test]
fn parse_error_reports_file_and_line() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".polaris"))?;
    let file = home.join(".polaris").join("config.toml");
    fs::write(&file, "[ratelimit]\nrpm = 1\n[broken\n")?;
    let dirs = discover_from(&home, &home)?;
    match read_layers(&dirs, &EnvSnapshot::empty(), &SessionOverrides::default()) {
        Err(ConfigError::Parse { file: f, line, .. }) => {
            assert_eq!(f, file);
            assert_eq!(line, 3);
        }
        other => return Err(format!("expected Parse error, got {other:?}").into()),
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_locked_file_is_retried() -> TestResult {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir_all(home.join(".polaris"))?;
    let file = home.join(".polaris").join("config.toml");
    fs::write(&file, "[ratelimit]\nrpm = 41\n")?;
    let held = fs::File::options().read(true).share_mode(0).open(&file)?;
    assert!(
        fs::read_to_string(&file).is_err(),
        "the lock is real: a plain read fails while held"
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(held);
    });
    let dirs = discover_from(&home, &home)?;
    let layers = read_layers(&dirs, &EnvSnapshot::empty(), &SessionOverrides::default());
    release.join().map_err(|_| "release thread panicked")?;
    assert_eq!(
        load(&layers?, &TrustGrant::none())?.config.ratelimit.rpm,
        41
    );
    Ok(())
}

#[test]
fn long_path_repo_dir_is_found() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    fs::create_dir_all(&home)?;
    let repo = tmp.path().join("repo");
    let mut deep = repo.clone();
    for i in 0..12 {
        deep.push(format!("segment-{i:02}-abcdefghijklmnopqrstuvwxyz"));
    }
    assert!(deep.as_os_str().len() > 300, "path must exceed MAX_PATH");
    fs::create_dir_all(&deep)?;
    fs::create_dir_all(repo.join(".polaris"))?;
    let dirs = discover_from(&deep, &home)?;
    assert_eq!(
        dirs.repo.as_deref(),
        Some(dunce::canonicalize(repo.join(".polaris"))?.as_path())
    );
    Ok(())
}
