//! P8 `/ci` and `/ci-run` on the sample in `tests/fixtures/ci/rust-lib`: the plan needs no Docker;
//! the live run needs Docker Desktop (or an engine) and the CI image, so it only runs with
//! `POLARIS_CI_DOCKER_LIVE=1` (build the image first: `docker build -t polaris-ci-rust:1.99.0
//! @prime/tools/ci`).

use std::path::{Path, PathBuf};

use polaris_ui_session::ci::{CI_IMAGE, ci_plan, remove_cache, run_report};
use polaris_ui_session::commands::LocalCommand;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tests/fixtures")
}

/// Copy a sample repository into a temporary workspace, dropping the `.fixture` suffix.
fn materialize(name: &str) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    let root = fixtures().join(name);
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
            let relative = relative.strip_suffix(".fixture").unwrap_or(&relative);
            let destination = target.path().join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, destination)?;
        }
    }
    Ok(target)
}

#[test]
fn the_plan_of_a_rust_workspace_caches_cargo_per_workspace() -> TestResult {
    let workspace = materialize("ci/rust-lib")?;
    let plan = ci_plan(workspace.path())?;
    let order: Vec<&str> = plan
        .pipeline
        .order()
        .iter()
        .map(|name| name.as_str())
        .collect();
    assert_eq!(order, ["format", "check", "test"]);
    assert_eq!(plan.caches.len(), 2);
    let prefix = plan
        .caches
        .first()
        .and_then(|cache| cache.name.strip_suffix("-cargo"))
        .ok_or("cargo cache volume")?
        .to_owned();
    assert!(prefix.starts_with("polaris-ci-") && prefix.len() == "polaris-ci-".len() + 12);
    assert_eq!(
        plan.caches.get(1).map(|cache| cache.name.clone()),
        Some(format!("{prefix}-target"))
    );
    assert!(
        plan.pipeline
            .steps()
            .iter()
            .all(|step| step.image.as_str() == CI_IMAGE)
    );
    // Another workspace gets its own caches.
    let other = materialize("ci/rust-lib")?;
    assert_ne!(ci_plan(other.path())?.caches, plan.caches);
    Ok(())
}

#[test]
fn a_workspace_without_cargo_has_no_pipeline() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    std::fs::remove_file(workspace.path().join("Cargo.toml"))?;
    let error = ci_plan(workspace.path()).err().ok_or("expected no plan")?;
    assert!(error.to_string().contains("only Rust projects"), "{error}");
    assert_eq!(LocalCommand::parse("/ci"), Some(LocalCommand::Ci));
    assert_eq!(LocalCommand::parse("/ci-run"), Some(LocalCommand::CiRun));
    assert_eq!(LocalCommand::CiRun.title(), "CI run");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_run_passes_and_the_second_run_is_warm() -> TestResult {
    if std::env::var_os("POLARIS_CI_DOCKER_LIVE").as_deref() != Some("1".as_ref()) {
        return Ok(());
    }
    let workspace = materialize("ci/rust-lib")?;
    let first = run_report(workspace.path(), None).await?;
    assert!(
        first
            .first()
            .is_some_and(|line| line.starts_with("CI run: PASS")),
        "{first:#?}"
    );
    assert!(
        first
            .iter()
            .any(|line| line.starts_with("Cache before the run:") && line.contains("(cold)"))
    );
    let second = run_report(workspace.path(), None).await?;
    assert!(
        second
            .first()
            .is_some_and(|line| line.starts_with("CI run: PASS")),
        "{second:#?}"
    );
    assert!(
        second
            .iter()
            .any(|line| line.starts_with("Cache before the run:") && !line.contains("(cold)")),
        "{second:#?}"
    );
    // The temporary workspace's caches go with it.
    remove_cache(workspace.path()).await?;
    let third = polaris_ui_session::ci::plan_report(workspace.path(), None).await?;
    assert!(
        third
            .iter()
            .any(|line| line.contains("(cold)") && !line.contains("(warm)")),
        "{third:#?}"
    );
    Ok(())
}
