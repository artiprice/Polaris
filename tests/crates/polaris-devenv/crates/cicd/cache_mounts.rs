//! Cache mounts (P8): a generic SandboxBackend cannot apply them, so plain validation rejects
//! them; a backend that mounts them itself validates with `validate_with_cache_mounts` and reads
//! the distinct mounts back for its configuration.

use std::time::Duration;

use polaris_devenv_cicd::{
    CacheMount, CiError, ImageRef, Pipeline, Step, StepName, WorkspaceFacts,
    rust_test_pipeline_cached,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn facts() -> Result<WorkspaceFacts, CiError> {
    Ok(WorkspaceFacts {
        workdir: "/workspace".to_owned(),
        image: ImageRef::new("polaris-ci-rust:1.99.0")?,
    })
}

fn step(name: &str, mounts: Vec<CacheMount>) -> Result<Step, CiError> {
    Ok(Step {
        name: StepName::new(name)?,
        image: ImageRef::new("polaris-ci-rust:1.99.0")?,
        cmd: vec!["cargo".to_owned(), "check".to_owned()],
        depends_on: Vec::new(),
        cache_mounts: mounts,
        env: Vec::new(),
        timeout: Duration::from_secs(60),
        workdir: None,
    })
}

fn mount(key: &str, target: &str) -> CacheMount {
    CacheMount {
        key: key.to_owned(),
        target: target.to_owned(),
    }
}

#[test]
fn the_cached_rust_pipeline_mounts_cargo_home_and_target() -> TestResult {
    let draft = rust_test_pipeline_cached(&facts()?, "polaris-ci-abc")?;
    assert!(matches!(
        rust_test_pipeline_cached(&facts()?, "polaris-ci-abc")?.validate(),
        Err(CiError::UnsupportedCacheMount)
    ));
    let pipeline = draft.validate_with_cache_mounts()?;
    let order: Vec<&str> = pipeline.order().iter().map(StepName::as_str).collect();
    assert_eq!(order, ["format", "check", "test"]);
    assert_eq!(
        pipeline.cache_mounts()?,
        [
            mount("polaris-ci-abc-cargo", "/cache/cargo"),
            mount("polaris-ci-abc-target", "/cache/target"),
        ]
    );
    for step in pipeline.steps() {
        assert!(
            step.env
                .contains(&("CARGO_HOME".to_owned(), "/cache/cargo".to_owned()))
        );
        assert!(
            step.env
                .contains(&("CARGO_TARGET_DIR".to_owned(), "/cache/target".to_owned()))
        );
    }
    Ok(())
}

#[test]
fn invalid_or_conflicting_cache_mounts_are_rejected() -> TestResult {
    for bad in [
        mount("", "/cache"),
        mount("-dash", "/cache"),
        mount("ok", "relative"),
        mount("ok", "/a/../b"),
        mount("ok", "C:/cache"),
    ] {
        let result =
            Pipeline::new(vec![step("check", vec![bad.clone()])?]).validate_with_cache_mounts();
        assert!(
            matches!(result, Err(CiError::InvalidPipeline(_))),
            "{bad:?}"
        );
    }
    let mut test = step("test", vec![mount("other", "/cache/cargo")])?;
    test.depends_on = vec![StepName::new("check")?];
    let conflicting = Pipeline::new(vec![
        step("check", vec![mount("cargo", "/cache/cargo")])?,
        test,
    ])
    .validate_with_cache_mounts()?;
    assert!(matches!(
        conflicting.cache_mounts(),
        Err(CiError::InvalidPipeline(_))
    ));
    Ok(())
}
