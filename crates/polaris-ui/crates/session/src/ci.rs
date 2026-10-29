//! `/ci` and `/ci-run` (P8): the workspace's CI pipeline, and a run of it in a local Docker
//! Engine with the cargo home and target directory on named cache volumes, so the second run
//! reuses what the first one downloaded and built. The workspace is mounted read-only; nothing
//! runs on the host, and nothing is pulled — a missing image is reported with its build command.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use polaris_devenv::cicd::{
    CiEngine, CiLogStore, CiReport, Leg, LegAvailability, Matrix, Pipeline, SandboxEngine,
    SandboxEngineConfig, StepReport, StepStatus, Validated, Verdict, WorkspaceFacts,
    rust_test_pipeline_cached,
};
use polaris_devenv::docker::{
    CacheVolume, Docker, DockerEndpoint, DockerSandbox, DockerSandboxConfig,
};
use polaris_kernel::directives::directive::{EnvKind, SessionDirective};
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::SandboxSpec;

use crate::SessionHostError;

/// The CI image (`@prime/tools/ci/Dockerfile`: the pinned toolchain with rustfmt and clippy).
pub const CI_IMAGE: &str = "polaris-ci-rust:1.99.0";
/// How to build [`CI_IMAGE`] from the repository.
pub const CI_IMAGE_BUILD: &str = "docker build -t polaris-ci-rust:1.99.0 @prime/tools/ci";
/// Where the workspace is mounted in the container.
const MOUNT_POINT: &str = "/workspace";
/// Memory ceiling of a CI container.
const MEMORY_BYTES: u64 = 6 * 1024 * 1024 * 1024;
/// CPU ceiling of a CI container, in nanocpus (4 CPUs).
const NANO_CPUS: u64 = 4_000_000_000;
/// Log bytes kept per step (the CI engine's cap).
const MAX_LOG_BYTES: usize = 1024 * 1024;
/// Lines of a failing step's log shown in the report.
const FAILURE_TAIL: usize = 30;

/// The pipeline for a workspace and the cache volumes it uses.
pub struct CiPlan {
    /// Validated pipeline.
    pub pipeline: Pipeline<Validated>,
    /// Cache volumes (key = volume name) and their mount targets.
    pub caches: Vec<CacheVolume>,
}

/// Cache volumes are per workspace: `polaris-ci-<12 hex of the canonical path>`.
fn cache_prefix(workspace: &Path) -> String {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_owned());
    let digest = blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex();
    format!("polaris-ci-{}", digest.get(..12).unwrap_or_default())
}

/// The CI plan of `workspace`: the default Rust pipeline with cached cargo home and target.
/// # Errors
/// No supported project at the workspace root, or an invalid pipeline.
pub fn ci_plan(workspace: &Path) -> Result<CiPlan, SessionHostError> {
    if !workspace.join("Cargo.toml").is_file() {
        return Err(SessionHostError::Unavailable(
            "no CI template for this workspace: only Rust projects (Cargo.toml at the root) are supported",
        ));
    }
    let image = polaris_devenv::cicd::ImageRef::new(CI_IMAGE)
        .map_err(|_| SessionHostError::Unavailable("invalid CI image"))?;
    let facts = WorkspaceFacts {
        workdir: MOUNT_POINT.to_owned(),
        image,
    };
    let pipeline = rust_test_pipeline_cached(&facts, &cache_prefix(workspace))
        .and_then(Pipeline::validate_with_cache_mounts)
        .map_err(|error| SessionHostError::Ui(format!("CI pipeline: {error}")))?;
    let caches = pipeline
        .cache_mounts()
        .map_err(|error| SessionHostError::Ui(format!("CI pipeline: {error}")))?
        .into_iter()
        .map(|mount| CacheVolume {
            name: mount.key,
            target: mount.target,
        })
        .collect();
    Ok(CiPlan { pipeline, caches })
}

fn seconds(duration: Duration) -> String {
    format!("{:.1} s", duration.as_secs_f64())
}

fn plan_lines(plan: &CiPlan) -> Vec<String> {
    let mut lines = vec![format!(
        "Pipeline (Rust): image {CI_IMAGE}, workspace mounted read-only at {MOUNT_POINT}"
    )];
    for (index, name) in plan.pipeline.order().iter().enumerate() {
        let Some(step) = plan.pipeline.steps().iter().find(|step| &step.name == name) else {
            continue;
        };
        let after = if step.depends_on.is_empty() {
            String::new()
        } else {
            format!(
                ", after {}",
                step.depends_on
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        lines.push(format!(
            "  {}. {}: {} (timeout {} min{after})",
            index.saturating_add(1),
            name.as_str(),
            step.cmd.join(" "),
            step.timeout.as_secs() / 60
        ));
    }
    lines
}

async fn connect() -> Result<Docker, String> {
    match tokio::time::timeout(
        Duration::from_secs(10),
        Docker::connect(&DockerEndpoint::local()),
    )
    .await
    {
        Ok(Ok(docker)) => Ok(docker),
        Ok(Err(error)) => Err(format!(
            "Docker is not reachable ({error}); start Docker Desktop"
        )),
        Err(_) => Err("Docker did not answer within 10 s; start Docker Desktop".to_owned()),
    }
}

fn make_sandbox(docker: Docker, workspace: &Path, plan: &CiPlan) -> Result<DockerSandbox, String> {
    let image =
        polaris_devenv::docker::ImageRef::new(CI_IMAGE).map_err(|error| error.to_string())?;
    DockerSandbox::new(
        docker,
        DockerSandboxConfig {
            workspace: workspace.to_owned(),
            mount_point: MOUNT_POINT.to_owned(),
            caches: plan.caches.clone(),
            default_image: image,
            memory_bytes: MEMORY_BYTES,
            nano_cpus: NANO_CPUS,
            network: true,
        },
    )
    .map_err(|error| error.to_string())
}

/// Per volume: `true` when it already exists (a warm cache).
async fn cache_state(sandbox: &DockerSandbox) -> Vec<(String, Option<bool>)> {
    let mut states = Vec::new();
    for cache in &sandbox.config().caches {
        states.push((
            cache.name.clone(),
            sandbox.volume_exists(&cache.name).await.ok(),
        ));
    }
    states
}

fn describe_caches(states: &[(String, Option<bool>)], targets: &[CacheVolume]) -> String {
    states
        .iter()
        .map(|(name, warm)| {
            let target = targets
                .iter()
                .find(|cache| &cache.name == name)
                .map_or("", |cache| cache.target.as_str());
            let state = match warm {
                Some(true) => "warm",
                Some(false) => "cold",
                None => "unknown",
            };
            format!("{name} at {target} ({state})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn env_note(directive: Option<&SessionDirective>) -> Option<String> {
    match directive {
        Some(SessionDirective::Env(EnvKind::Nix)) => Some(
            "$env(nix) is set, but no Nix is installed here: CI runs in the Docker image instead."
                .to_owned(),
        ),
        Some(SessionDirective::Env(EnvKind::Host)) => Some(
            "$env(host) is set; CI still runs in Docker — workspace code never runs on the host."
                .to_owned(),
        ),
        _ => None,
    }
}

/// `/ci`: the pipeline, its caches and whether Docker can run it.
/// # Errors
/// No supported project, or an invalid pipeline.
pub async fn plan_report(
    workspace: &Path,
    env: Option<&SessionDirective>,
) -> Result<Vec<String>, SessionHostError> {
    let plan = ci_plan(workspace)?;
    let mut lines = plan_lines(&plan);
    lines.extend(env_note(env));
    match connect().await {
        Err(reason) => lines.push(reason),
        Ok(docker) => match make_sandbox(docker, workspace, &plan) {
            Err(reason) => lines.push(format!("Docker sandbox: {reason}")),
            Ok(sandbox) => {
                let states = cache_state(&sandbox).await;
                lines.push(format!(
                    "  cache: {}",
                    describe_caches(&states, &plan.caches)
                ));
                let image = polaris_devenv::docker::ImageRef::new(CI_IMAGE)
                    .map_err(|_| SessionHostError::Unavailable("invalid CI image"))?;
                match sandbox.image_present(&image).await {
                    Ok(true) => lines.push(format!(
                        "Docker {} is ready; run the pipeline with /ci-run.",
                        sandbox.engine_version()
                    )),
                    Ok(false) => {
                        lines.push(format!("The CI image is not built yet: {CI_IMAGE_BUILD}"))
                    }
                    Err(error) => {
                        lines.push(format!("Docker could not inspect the image: {error}"))
                    }
                }
            }
        },
    }
    Ok(lines)
}

/// Remove the workspace's CI cache volumes, so the next `/ci-run` starts cold.
/// # Errors
/// No supported project, Docker is unreachable, or a volume is still in use.
pub async fn remove_cache(workspace: &Path) -> Result<(), SessionHostError> {
    let plan = ci_plan(workspace)?;
    let docker = connect().await.map_err(SessionHostError::Ui)?;
    let sandbox = make_sandbox(docker, workspace, &plan).map_err(SessionHostError::Ui)?;
    sandbox
        .remove_caches()
        .await
        .map_err(|error| SessionHostError::Ui(format!("removing the CI cache: {error}")))
}

/// Step logs kept in memory for the report (redacted by the engine before they arrive).
#[derive(Default, Clone)]
struct MemoryLogs(Arc<Mutex<HashMap<Cid, Vec<u8>>>>);

impl CiLogStore for MemoryLogs {
    fn store(&self, redacted_log: &[u8]) -> Result<Cid, String> {
        let cid = Cid::from_bytes(*blake3::hash(redacted_log).as_bytes());
        self.0
            .lock()
            .map_err(|_| "log store poisoned".to_owned())?
            .insert(cid, redacted_log.to_vec());
        Ok(cid)
    }
}

fn status_name(step: &StepReport) -> &'static str {
    match step.status {
        StepStatus::Passed => "passed",
        StepStatus::Failed => "FAILED",
        StepStatus::TimedOut => "TIMED OUT",
        _ => "skipped",
    }
}

/// `/ci-run`: run the pipeline in Docker and report each step, the cache state before and after,
/// and the tail of a failing step's log.
/// # Errors
/// No supported project, or an invalid pipeline.
pub async fn run_report(
    workspace: &Path,
    env: Option<&SessionDirective>,
) -> Result<Vec<String>, SessionHostError> {
    let plan = ci_plan(workspace)?;
    let mut lines = Vec::new();
    lines.extend(env_note(env));
    let docker = match connect().await {
        Ok(docker) => docker,
        Err(reason) => {
            lines.push(reason);
            return Ok(lines);
        }
    };
    let sandbox = match make_sandbox(docker, workspace, &plan) {
        Ok(sandbox) => sandbox,
        Err(reason) => {
            lines.push(format!("Docker sandbox: {reason}"));
            return Ok(lines);
        }
    };
    let image = polaris_devenv::docker::ImageRef::new(CI_IMAGE)
        .map_err(|_| SessionHostError::Unavailable("invalid CI image"))?;
    match sandbox.image_present(&image).await {
        Ok(true) => {}
        Ok(false) => {
            lines.push(format!("The CI image is not built yet: {CI_IMAGE_BUILD}"));
            return Ok(lines);
        }
        Err(error) => {
            lines.push(format!("Docker could not inspect the image: {error}"));
            return Ok(lines);
        }
    }
    let before = cache_state(&sandbox).await;
    let engine_version = sandbox.engine_version().to_owned();
    let logs = MemoryLogs::default();
    let redactor = polaris_security::automata::DfaRedactor::default_redactor()
        .map_err(|_| SessionHostError::Unavailable("log redactor failed to compile"))?;
    let engine = SandboxEngine::new(
        sandbox,
        redactor,
        logs.clone(),
        SandboxEngineConfig {
            sandbox: SandboxSpec::default(),
            lifecycle_timeout: Duration::from_secs(120),
            max_log_bytes: MAX_LOG_BYTES,
        },
    )
    .map_err(|error| SessionHostError::Ui(format!("CI engine: {error}")))?;
    let matrix = Matrix::new(vec![Leg {
        name: "linux-x86_64".to_owned(),
        required: true,
        availability: LegAvailability::Available,
        env: Vec::new(),
    }])
    .map_err(|error| SessionHostError::Ui(format!("CI matrix: {error}")))?;
    let started = Instant::now();
    let report: CiReport = match engine.run(&plan.pipeline, &matrix).await {
        Ok(report) => report,
        Err(error) => {
            lines.push(format!("CI run failed to complete: {error}"));
            return Ok(lines);
        }
    };
    let elapsed = started.elapsed();
    // The engine owned the sandbox; a fresh handle reads the volumes the run left.
    let after = match connect().await {
        Ok(docker) => match make_sandbox(docker, workspace, &plan) {
            Ok(probe) => cache_state(&probe).await,
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let verdict = match report.verdict {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Incomplete => "INCOMPLETE",
    };
    lines.insert(
        0,
        format!(
            "CI run: {verdict} in {} (Docker {engine_version})",
            seconds(elapsed)
        ),
    );
    for leg in &report.legs {
        for step in &leg.steps {
            let exit = step
                .exit_code
                .map(|code| format!(", exit {code}"))
                .unwrap_or_default();
            lines.push(format!(
                "  {:<7} {:<9} {}{exit}",
                step.name.as_str(),
                status_name(step),
                seconds(step.duration)
            ));
        }
    }
    lines.push(format!(
        "Cache before the run: {}",
        describe_caches(&before, &plan.caches)
    ));
    if !after.is_empty() {
        lines.push(format!(
            "Cache after the run: {}",
            describe_caches(&after, &plan.caches)
        ));
    }
    if let Some(step) = report
        .legs
        .iter()
        .flat_map(|leg| &leg.steps)
        .find(|step| matches!(step.status, StepStatus::Failed | StepStatus::TimedOut))
        && let Some(cid) = step.log_cid
        && let Ok(stored) = logs.0.lock()
        && let Some(log) = stored.get(&cid)
    {
        let text = String::from_utf8_lossy(log);
        let tail: Vec<&str> = text.lines().rev().take(FAILURE_TAIL).collect();
        lines.push(format!("Last lines of {}:", step.name.as_str()));
        lines.extend(tail.into_iter().rev().map(|line| format!("  | {line}")));
    }
    Ok(lines)
}
