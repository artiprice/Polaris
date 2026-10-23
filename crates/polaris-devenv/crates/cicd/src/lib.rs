//! Typed pipelines, topological execution, and honest sandbox-backed CI reports.
use polaris_sdk::{
    ids::Cid,
    registry::{
        ExecOutput, ExecRequest, Redactor, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
    },
};
use std::{
    collections::BTreeSet,
    future::Future,
    time::{Duration, Instant, SystemTime},
};
use thiserror::Error;

/// Pipeline step identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepName(String);
impl StepName {
    /// Create a bounded printable step name.
    /// # Errors
    /// Returns InvalidPipeline for empty, oversized, or control-containing names.
    pub fn new(value: impl Into<String>) -> Result<Self, CiError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 96 || value.chars().any(char::is_control) {
            return Err(CiError::InvalidPipeline("invalid step name".into()));
        }
        Ok(Self(value))
    }
    /// Name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Container image reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageRef(String);
impl ImageRef {
    /// Create an image reference.
    /// # Errors
    /// Returns InvalidPipeline for an empty or whitespace-containing reference.
    pub fn new(value: impl Into<String>) -> Result<Self, CiError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
            return Err(CiError::InvalidPipeline("invalid image reference".into()));
        }
        Ok(Self(value))
    }
    /// Image reference text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Named cache mount request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMount {
    /// Stable cache key scoped by the caller.
    pub key: String,
    /// Absolute path inside the sandbox.
    pub target: String,
}
/// One command in a CI pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Unique step name.
    pub name: StepName,
    /// Sandbox image.
    pub image: ImageRef,
    /// Argument vector; no implicit shell is used.
    pub cmd: Vec<String>,
    /// Dependencies that must complete successfully first.
    pub depends_on: Vec<StepName>,
    /// Requested cache mounts. SandboxBackend currently cannot apply them; validation rejects nonempty values.
    pub cache_mounts: Vec<CacheMount>,
    /// Nonsecret environment variables.
    pub env: Vec<(String, String)>,
    /// Per-step execution timeout.
    pub timeout: Duration,
    /// Working directory inside the sandbox, if the command requires one.
    pub workdir: Option<String>,
}
/// Pipeline state marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draft;
/// Pipeline state marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validated;
/// Typed pipeline with validated topological order when S is Validated.
#[derive(Debug, Clone)]
pub struct Pipeline<S> {
    steps: Vec<Step>,
    order: Vec<StepName>,
    marker: core::marker::PhantomData<S>,
}
impl Pipeline<Draft> {
    /// Create a draft pipeline.
    #[must_use]
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps,
            order: Vec::new(),
            marker: core::marker::PhantomData,
        }
    }
    /// Validate names, dependencies, DAG ordering, image consistency, environment, and limits.
    /// # Errors
    /// Returns InvalidPipeline for invalid steps or dependency cycles.
    pub fn validate(self) -> Result<Pipeline<Validated>, CiError> {
        if self.steps.is_empty() {
            return Err(CiError::InvalidPipeline(
                "pipeline needs at least one step".into(),
            ));
        }
        let mut names = BTreeSet::new();
        let mut image = None;
        for step in &self.steps {
            if !names.insert(step.name.clone()) {
                return Err(CiError::InvalidPipeline(format!(
                    "duplicate step {}",
                    step.name.as_str()
                )));
            }
            if !matches!(step.cmd.first(),Some(command) if !command.is_empty()) {
                return Err(CiError::InvalidPipeline(format!(
                    "empty executable in {}",
                    step.name.as_str()
                )));
            }
            if step.timeout.is_zero() || step.timeout > Duration::from_secs(86_400) {
                return Err(CiError::InvalidPipeline(format!(
                    "invalid timeout in {}",
                    step.name.as_str()
                )));
            }
            if !step.cache_mounts.is_empty() {
                return Err(CiError::UnsupportedCacheMount);
            }
            if let Some(current) = &image {
                if current != &step.image {
                    return Err(CiError::InvalidPipeline(
                        "SandboxBackend supports one shared image per pipeline".into(),
                    ));
                }
            } else {
                image = Some(step.image.clone());
            }
            validate_env(&step.name, &step.env)?;
            if let Some(workdir) = &step.workdir {
                if workdir.trim().is_empty() || workdir.chars().any(char::is_control) {
                    return Err(CiError::InvalidPipeline(format!(
                        "invalid workdir in {}",
                        step.name.as_str()
                    )));
                }
            }
            let mut deps = BTreeSet::new();
            for dependency in &step.depends_on {
                if dependency == &step.name || !deps.insert(dependency.clone()) {
                    return Err(CiError::InvalidPipeline(format!(
                        "invalid dependency in {}",
                        step.name.as_str()
                    )));
                }
                if !names.contains(dependency)
                    && !self.steps.iter().any(|prior| prior.name == *dependency)
                {
                    return Err(CiError::InvalidPipeline(format!(
                        "unknown dependency {}",
                        dependency.as_str()
                    )));
                }
            }
        }
        let names_by_step = self
            .steps
            .iter()
            .map(|step| step.name.clone())
            .collect::<BTreeSet<_>>();
        for step in &self.steps {
            for dependency in &step.depends_on {
                if !names_by_step.contains(dependency) {
                    return Err(CiError::InvalidPipeline(format!(
                        "unknown dependency {}",
                        dependency.as_str()
                    )));
                }
            }
        }
        let mut order = Vec::with_capacity(self.steps.len());
        let mut complete = BTreeSet::new();
        while order.len() < self.steps.len() {
            let next = self.steps.iter().find(|step| {
                !complete.contains(&step.name)
                    && step
                        .depends_on
                        .iter()
                        .all(|dependency| complete.contains(dependency))
            });
            let Some(step) = next else {
                return Err(CiError::InvalidPipeline("dependency cycle".into()));
            };
            complete.insert(step.name.clone());
            order.push(step.name.clone());
        }
        Ok(Pipeline {
            steps: self.steps,
            order,
            marker: core::marker::PhantomData,
        })
    }
}
impl Pipeline<Validated> {
    /// Validated steps.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }
    /// Deterministic topological order.
    #[must_use]
    pub fn order(&self) -> &[StepName] {
        &self.order
    }
}
/// One explicit CI matrix leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    /// Stable unique leg name.
    pub name: String,
    /// Whether the leg is required for a complete result.
    pub required: bool,
    /// Caller-detected availability.
    pub availability: LegAvailability,
    /// Additional environment variables for this leg.
    pub env: Vec<(String, String)>,
}
/// Availability of a matrix leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegAvailability {
    /// Runner has the required capabilities.
    Available,
    /// Runner lacks required capability; reason is preserved in report.
    Unavailable(String),
}
/// Explicit matrix definition; discovery is performed by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    /// Configured legs.
    pub legs: Vec<Leg>,
}
impl Matrix {
    /// Validate and create explicit matrix legs.
    /// # Errors
    /// Returns InvalidPipeline for empty or duplicate legs.
    pub fn new(legs: Vec<Leg>) -> Result<Self, CiError> {
        if legs.is_empty() {
            return Err(CiError::InvalidPipeline(
                "matrix needs at least one leg".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for leg in &legs {
            if leg.name.trim().is_empty()
                || leg.name.len() > 96
                || leg.name.chars().any(char::is_control)
                || !names.insert(leg.name.clone())
            {
                return Err(CiError::InvalidPipeline(
                    "invalid or duplicate matrix leg".into(),
                ));
            }
            validate_env_values(&leg.env)?;
        }
        Ok(Self { legs })
    }
    /// Return the configured legs in deterministic order.
    #[must_use]
    pub fn expand(&self) -> Vec<Leg> {
        let mut legs = self.legs.clone();
        legs.sort_by(|a, b| a.name.cmp(&b.name));
        legs
    }
}
/// Reported cache state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// Backend reported a cache hit.
    Hit,
    /// Backend reported a cache miss.
    Miss,
    /// Backend exposes no cache observation.
    Unknown,
}
/// Result of one pipeline command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    /// Command exited with status zero.
    Passed,
    /// Command exited nonzero.
    Failed,
    /// Command exceeded its configured deadline.
    TimedOut,
    /// Step was skipped after a dependency or earlier failure.
    Skipped,
}
/// Per-step evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    /// Step name.
    pub name: StepName,
    /// Outcome.
    pub status: StepStatus,
    /// Exit code, if the backend returned one.
    pub exit_code: Option<i32>,
    /// Observed wall duration.
    pub duration: Duration,
    /// Cache status. SandboxBackend has no cache reporting, so this is Unknown.
    pub cache: CacheStatus,
    /// CAS content id of redacted logs, absent only when timeout provided no output.
    pub log_cid: Option<Cid>,
    /// Whether the stored, already-redacted log byte stream was truncated to the configured cap.
    pub logs_truncated: bool,
}
/// Outcome of one matrix leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegStatus {
    /// Every command passed.
    Passed,
    /// A command failed or timed out.
    Failed,
    /// Optional unavailable leg was skipped.
    Skipped,
    /// Required unavailable leg prevents a complete pass.
    Incomplete,
}
/// Per-leg report including all executed/skipped steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegReport {
    /// Leg name.
    pub name: String,
    /// Required status.
    pub required: bool,
    /// Leg outcome.
    pub status: LegStatus,
    /// Unavailability explanation, if any.
    pub reason: Option<String>,
    /// Step evidence.
    pub steps: Vec<StepReport>,
}
/// Overall CI verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every required leg passed.
    Pass,
    /// A command failed or timed out.
    Fail,
    /// No command failed, but required legs were unavailable.
    Incomplete,
}
/// Overall pipeline report with actual leg and log evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiReport {
    /// Per-leg results.
    pub legs: Vec<LegReport>,
    /// Overall verdict.
    pub verdict: Verdict,
    /// Wall-clock start time.
    pub started: SystemTime,
    /// Engine label.
    pub engine: String,
}
impl CiReport {
    /// Compute the verdict from leg statuses.
    #[must_use]
    pub fn verdict(legs: &[LegReport]) -> Verdict {
        if legs.iter().any(|leg| leg.status == LegStatus::Failed) {
            Verdict::Fail
        } else if legs.iter().any(|leg| leg.status == LegStatus::Incomplete)
            || legs
                .iter()
                .all(|leg| matches!(leg.status, LegStatus::Skipped | LegStatus::Incomplete))
        {
            Verdict::Incomplete
        } else {
            Verdict::Pass
        }
    }
}
/// Pipeline engine contract.
pub trait CiEngine: Send + Sync {
    /// Execute a validated pipeline against its explicit matrix.
    fn run(
        &self,
        pipeline: &Pipeline<Validated>,
        matrix: &Matrix,
    ) -> impl Future<Output = Result<CiReport, CiError>> + Send;
}
/// Injected durable log writer. Implementations must return the CAS id for these bytes.
pub trait CiLogStore: Send + Sync {
    /// Store already-redacted bytes and return their actual content id.
    fn store(&self, redacted_log: &[u8]) -> Result<Cid, String>;
}
/// Sandbox engine settings.
#[derive(Debug, Clone)]
pub struct SandboxEngineConfig {
    /// Sandbox limits and policy.
    pub sandbox: SandboxSpec,
    /// Deadline for sandbox creation and destruction.
    pub lifecycle_timeout: Duration,
    /// Maximum stored bytes after the complete stdout/stderr output has been redacted.
    pub max_log_bytes: usize,
}
/// Executes pipelines through an actual SDK SandboxBackend.
pub struct SandboxEngine<B, R, L> {
    backend: B,
    redactor: R,
    logs: L,
    config: SandboxEngineConfig,
}
impl<B: SandboxBackend, R: Redactor, L: CiLogStore> SandboxEngine<B, R, L> {
    /// Construct a sandbox engine with an injected redactor and CAS log store.
    /// # Errors
    /// Rejects zero lifecycle or log bounds.
    pub fn new(
        backend: B,
        redactor: R,
        logs: L,
        config: SandboxEngineConfig,
    ) -> Result<Self, CiError> {
        if config.lifecycle_timeout.is_zero() || config.max_log_bytes == 0 {
            return Err(CiError::InvalidPipeline(
                "sandbox lifecycle and log limits must be nonzero".into(),
            ));
        }
        Ok(Self {
            backend,
            redactor,
            logs,
            config,
        })
    }
    async fn run_leg(
        &self,
        pipeline: &Pipeline<Validated>,
        leg: &Leg,
    ) -> Result<LegReport, CiError> {
        let image = pipeline
            .steps
            .first()
            .map(|step| step.image.as_str().to_owned())
            .ok_or_else(|| CiError::InvalidPipeline("validated pipeline has no steps".into()))?;
        let mut spec = self.config.sandbox.clone();
        spec.image = Some(image);
        let sandbox =
            tokio::time::timeout(self.config.lifecycle_timeout, self.backend.create(&spec))
                .await
                .map_err(|_| CiError::EngineFailure("sandbox create timed out".into()))?
                .map_err(CiError::Sandbox)?;
        let execution = self.execute_steps(pipeline, leg, &sandbox).await;
        let cleanup = tokio::time::timeout(
            self.config.lifecycle_timeout,
            self.backend.destroy(&sandbox),
        )
        .await
        .map_err(|_| CiError::CleanupFailure("sandbox destroy timed out".into()))
        .and_then(|result| result.map_err(CiError::Sandbox));
        match (execution, cleanup) {
            (Ok(report), Ok(())) => Ok(report),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(primary), Err(cleanup_error)) => Err(CiError::CleanupAfterFailure {
                primary: primary.to_string(),
                cleanup: cleanup_error.to_string(),
            }),
        }
    }
    async fn execute_steps(
        &self,
        pipeline: &Pipeline<Validated>,
        leg: &Leg,
        sandbox: &SandboxHandle,
    ) -> Result<LegReport, CiError> {
        let mut reports = Vec::with_capacity(pipeline.order.len());
        let mut failed = false;
        for name in &pipeline.order {
            let Some(step) = pipeline.steps.iter().find(|step| &step.name == name) else {
                return Err(CiError::InvalidPipeline(
                    "topological order references a missing step".into(),
                ));
            };
            if failed {
                reports.push(skipped(step));
                continue;
            }
            let mut env = step.env.clone();
            env.extend(leg.env.clone());
            validate_env(&step.name, &env)?;
            let request = ExecRequest {
                argv: step.cmd.clone(),
                workdir: step.workdir.clone(),
                stdin: None,
                env,
                timeout: Some(step.timeout),
                output_limit_bytes: None,
            };
            let started = Instant::now();
            let result =
                tokio::time::timeout(step.timeout, self.backend.exec(sandbox, &request)).await;
            match result {
                Err(_) => {
                    reports.push(StepReport {
                        name: step.name.clone(),
                        status: StepStatus::TimedOut,
                        exit_code: None,
                        duration: started.elapsed(),
                        cache: CacheStatus::Unknown,
                        log_cid: None,
                        logs_truncated: false,
                    });
                    failed = true;
                }
                Ok(Err(error)) => return Err(CiError::Sandbox(error)),
                Ok(Ok(output)) => {
                    let (cid, logs_truncated) = self.store_logs(&output)?;
                    let status = if output.exit_code == 0 {
                        StepStatus::Passed
                    } else {
                        StepStatus::Failed
                    };
                    if status == StepStatus::Failed {
                        failed = true;
                    }
                    reports.push(StepReport {
                        name: step.name.clone(),
                        status,
                        exit_code: Some(output.exit_code),
                        duration: output.duration,
                        cache: CacheStatus::Unknown,
                        log_cid: Some(cid),
                        logs_truncated,
                    });
                }
            }
        }
        let status = if reports
            .iter()
            .any(|step| matches!(step.status, StepStatus::Failed | StepStatus::TimedOut))
        {
            LegStatus::Failed
        } else {
            LegStatus::Passed
        };
        Ok(LegReport {
            name: leg.name.clone(),
            required: leg.required,
            status,
            reason: None,
            steps: reports,
        })
    }
    fn store_logs(&self, output: &ExecOutput) -> Result<(Cid, bool), CiError> {
        let mut raw = Vec::with_capacity(
            output
                .stdout
                .len()
                .saturating_add(output.stderr.len())
                .saturating_add(20),
        );
        raw.extend_from_slice(b"[stdout]\n");
        raw.extend_from_slice(&output.stdout);
        raw.extend_from_slice(b"\n[stderr]\n");
        raw.extend_from_slice(&output.stderr);
        let mut redacted = Vec::new();
        let _stats = self.redactor.redact_into(&raw, &mut redacted);
        let truncated = redacted.len() > self.config.max_log_bytes;
        redacted.truncate(self.config.max_log_bytes);
        self.logs
            .store(&redacted)
            .map(|cid| (cid, truncated))
            .map_err(CiError::LogStore)
    }
}
impl<B: SandboxBackend, R: Redactor, L: CiLogStore> CiEngine for SandboxEngine<B, R, L> {
    fn run(
        &self,
        pipeline: &Pipeline<Validated>,
        matrix: &Matrix,
    ) -> impl Future<Output = Result<CiReport, CiError>> + Send {
        async move {
            let started = SystemTime::now();
            let mut legs = Vec::new();
            for leg in matrix.expand() {
                match &leg.availability {
                    LegAvailability::Unavailable(reason) => {
                        let status = if leg.required {
                            LegStatus::Incomplete
                        } else {
                            LegStatus::Skipped
                        };
                        legs.push(LegReport {
                            name: leg.name,
                            required: leg.required,
                            status,
                            reason: Some(reason.clone()),
                            steps: Vec::new(),
                        });
                    }
                    LegAvailability::Available => legs.push(self.run_leg(pipeline, &leg).await?),
                }
            }
            let verdict = CiReport::verdict(&legs);
            Ok(CiReport {
                legs,
                verdict,
                started,
                engine: "sandbox-backend".to_owned(),
            })
        }
    }
}
/// Workspace facts used to form a default Rust check pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFacts {
    /// Workspace-relative root inside the image.
    pub workdir: String,
    /// Rust toolchain image.
    pub image: ImageRef,
}
/// Build a Draft Rust pipeline with formatting, check, and test commands.
pub fn rust_test_pipeline(workspace: &WorkspaceFacts) -> Result<Pipeline<Draft>, CiError> {
    let format = Step {
        name: StepName::new("format")?,
        image: workspace.image.clone(),
        cmd: vec![
            "cargo".into(),
            "fmt".into(),
            "--all".into(),
            "--".into(),
            "--check".into(),
        ],
        depends_on: Vec::new(),
        cache_mounts: Vec::new(),
        env: Vec::new(),
        timeout: Duration::from_secs(600),
        workdir: Some(workspace.workdir.clone()),
    };
    let check = Step {
        name: StepName::new("check")?,
        image: workspace.image.clone(),
        cmd: vec!["cargo".into(), "check".into(), "--workspace".into()],
        depends_on: vec![format.name.clone()],
        cache_mounts: Vec::new(),
        env: Vec::new(),
        timeout: Duration::from_secs(1800),
        workdir: Some(workspace.workdir.clone()),
    };
    let test = Step {
        name: StepName::new("test")?,
        image: workspace.image.clone(),
        cmd: vec!["cargo".into(), "test".into(), "--workspace".into()],
        depends_on: vec![check.name.clone()],
        cache_mounts: Vec::new(),
        env: Vec::new(),
        timeout: Duration::from_secs(3600),
        workdir: Some(workspace.workdir.clone()),
    };
    let steps = vec![format, check, test];
    Ok(Pipeline::new(steps))
}
/// Pipeline validation or sandbox execution failure.
#[derive(Debug, Error)]
pub enum CiError {
    /// Pipeline definition violates a structural or security constraint.
    #[error("invalid pipeline: {0}")]
    InvalidPipeline(String),
    /// Sandbox backend failed.
    #[error("sandbox failure: {0}")]
    Sandbox(#[source] SandboxError),
    /// Sandbox creation failed to complete before its lifecycle deadline.
    #[error("sandbox engine failure: {0}")]
    EngineFailure(String),
    /// Command log could not be stored after redaction.
    #[error("CAS log storage failed: {0}")]
    LogStore(String),
    /// Sandbox destroy failed after another operation failed.
    #[error("operation failed ({primary}); cleanup also failed ({cleanup})")]
    CleanupAfterFailure { primary: String, cleanup: String },
    /// Sandbox lifecycle cleanup failed.
    #[error("sandbox cleanup failed: {0}")]
    CleanupFailure(String),
    /// Sandbox backend did not support requested cache mounts.
    #[error("SandboxBackend cannot apply cache mounts; cache remains unknown")]
    UnsupportedCacheMount,
}
fn validate_env(step: &StepName, values: &[(String, String)]) -> Result<(), CiError> {
    validate_env_name_pairs(values)?;
    if values
        .iter()
        .any(|(key, value)| looks_secret_env(key, value))
    {
        return Err(CiError::InvalidPipeline(format!(
            "secret-looking environment entry rejected in {}",
            step.as_str()
        )));
    }
    Ok(())
}
fn validate_env_name_pairs(values: &[(String, String)]) -> Result<(), CiError> {
    let mut names = BTreeSet::new();
    for (key, _) in values {
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || !names.insert(key)
        {
            return Err(CiError::InvalidPipeline(
                "invalid or duplicate environment name".into(),
            ));
        }
    }
    Ok(())
}
fn validate_env_values(values: &[(String, String)]) -> Result<(), CiError> {
    validate_env_name_pairs(values)?;
    if values
        .iter()
        .any(|(key, value)| looks_secret_env(key, value))
    {
        return Err(CiError::InvalidPipeline(
            "secret-looking environment entry rejected".into(),
        ));
    }
    Ok(())
}
fn looks_secret_env(key: &str, value: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    let value_upper = value.to_ascii_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PRIVATE_KEY",
        "API_KEY",
        "CREDENTIAL",
    ]
    .iter()
    .any(|word| upper.contains(word))
        || looks_secret(value)
        || value_upper.starts_with("-----BEGIN ")
}
fn looks_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("akia")
        || lower.contains("-----begin private key-----")
}
fn skipped(step: &Step) -> StepReport {
    StepReport {
        name: step.name.clone(),
        status: StepStatus::Skipped,
        exit_code: None,
        duration: Duration::ZERO,
        cache: CacheStatus::Unknown,
        log_cid: None,
        logs_truncated: false,
    }
}
