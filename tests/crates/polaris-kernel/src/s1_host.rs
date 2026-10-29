// Unit tests of crates/polaris-kernel/src/s1_host.rs (`tests`); compiled into that module via #[path].
#[cfg(windows)]
use std::path::PathBuf;
use std::{error::Error, fs};

use super::{LocalS1Config, S1Normalization};
use polaris_sdk::{ids::SessionId, registry::SpawnSpec};

fn configured_profile() -> Result<(tempfile::TempDir, LocalS1Config), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let worker = root.path().join("polaris-worker.exe");
    let runtime = root.path().join("onnxruntime.dll");
    let model = root.path().join("model");
    fs::create_dir(&model)?;
    fs::write(&worker, b"worker fixture")?;
    fs::write(&runtime, b"runtime fixture")?;
    #[cfg(unix)]
    let endpoint = root.path().join("s1-worker.sock");
    #[cfg(windows)]
    let endpoint = PathBuf::from(r"\\.\pipe\polaris-host-test");
    let profile = LocalS1Config::new(worker, model, runtime, endpoint);
    Ok((root, profile))
}

fn process_spec(profile: &LocalS1Config) -> Result<SpawnSpec, Box<dyn Error>> {
    Ok(profile.worker_spec(SessionId::from_u128(7))?.process)
}

#[test]
fn configured_worker_has_immutable_paths_and_no_bootstrap_material() -> Result<(), Box<dyn Error>> {
    let (_root, mut profile) = configured_profile()?;
    profile.normalization = S1Normalization::Gliner2Python21;
    profile.validate()?;
    let spec = process_spec(&profile)?;
    assert_eq!(
        spec.program, profile.worker_executable,
        "worker executable must remain configured"
    );
    assert_eq!(
        spec.cwd.as_ref(),
        Some(&profile.model_dir),
        "worker cwd must be the configured model directory"
    );
    assert_eq!(
        spec.env,
        [(
            std::ffi::OsString::from("GLINER2_DEVICE"),
            std::ffi::OsString::from("cpu")
        )],
        "worker receives only the fixed CPU device pin, never ambient environment"
    );
    assert!(
        spec.args
            .iter()
            .all(|argument| !argument.to_string_lossy().contains("secret")),
        "bootstrap material must never appear in process arguments"
    );
    assert!(
        spec.args
            .iter()
            .any(|argument| argument == "gliner2-python-2.1"),
        "the configured normalization contract must reach the worker"
    );
    Ok(())
}

#[test]
fn invalid_paths_and_unbounded_deadlines_are_rejected_before_spawn() -> Result<(), Box<dyn Error>> {
    let (_root, mut profile) = configured_profile()?;
    profile.worker_executable = profile.worker_executable.with_file_name("missing-worker");
    assert!(
        profile.validate().is_err(),
        "missing executable must reject the profile"
    );
    let (_root, mut profile) = configured_profile()?;
    profile.startup_timeout = std::time::Duration::from_secs(121);
    assert!(
        profile.validate().is_err(),
        "startup timeout above the hard cap must reject the profile"
    );
    let (_root, mut profile) = configured_profile()?;
    profile.idle_ttl = std::time::Duration::from_micros(500);
    assert!(
        profile.validate().is_err(),
        "submillisecond idle TTL must not be rounded to a zero worker argument"
    );
    let (_root, mut profile) = configured_profile()?;
    profile.poll_interval = std::time::Duration::MAX;
    assert!(
        profile.validate().is_err(),
        "unbounded process polling interval must be rejected before Tokio scheduling"
    );
    Ok(())
}
