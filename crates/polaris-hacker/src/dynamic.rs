use crate::HackerError;
use polaris_sdk::registry::{ExecRequest, Redactor, SandboxBackend, SandboxSpec};
use polaris_security_automata::DfaRedactor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

const MAX_DYNAMIC_CASES: usize = 64;
const MAX_DYNAMIC_CASE_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_EXEC_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_ARG_COUNT: usize = 128;
const MAX_ARG_BYTES: usize = 64 * 1024;

/// Requested sanitizer label; command configuration remains explicit in `argv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sanitizer {
    /// No sanitizer diagnosis is expected from the command.
    None,
    /// The caller configured a real ThreadSanitizer build/run command.
    Thread,
    /// The caller configured a real AddressSanitizer build/run command.
    Address,
    /// The caller configured a real Miri command.
    Miri,
    /// The caller configured a Loom test command.
    Loom,
}

/// Caller-owned command and resource request; argv is sent directly to the SDK backend.
#[derive(Debug, Clone)]
pub struct StressPlan {
    /// Sandbox resource request. Backend policy must provide the isolation and limits.
    pub sandbox: SandboxSpec,
    /// Executable and arguments; no implicit shell is added.
    pub argv: Vec<String>,
    /// Bounded stdin cases, normally from [`crate::BoundaryGen`].
    pub cases: Vec<Vec<u8>>,
    /// Per-case execution timeout.
    pub timeout: Duration,
    /// Combined output byte cap per case.
    pub output_limit_bytes: usize,
    /// Seed recorded with runs for reproduction.
    pub seed: u64,
    /// Sanitizer command label, which is not independently verified by this crate.
    pub sanitizer: Sanitizer,
}

/// One backend execution result, preserving the exact bounded stdin for replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicRun {
    /// Exact argv sent to the backend.
    pub argv: Vec<String>,
    /// Reproduction seed.
    pub seed: u64,
    /// Input index and bytes submitted as stdin.
    pub input_index: usize,
    /// Exact caller-supplied stdin bytes, bounded by `MAX_DYNAMIC_CASE_BYTES`.
    pub stdin: Vec<u8>,
    /// Sanitizer label supplied by the caller.
    pub sanitizer: Sanitizer,
    /// Observed process status from the backend.
    pub exit_code: i32,
    /// Bounded stdout bytes.
    pub stdout: Vec<u8>,
    /// Bounded stderr bytes.
    pub stderr: Vec<u8>,
    /// Backend reported duration.
    pub duration: Duration,
}

/// Results of executing a stress plan through the real SDK backend interface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DynamicReport {
    /// Per-case execution observations.
    pub runs: Vec<DynamicRun>,
    /// Wall duration measured by this caller around backend executions and cleanup.
    pub duration: Duration,
}

/// Create a backend sandbox and execute bounded inputs with bounded output and deadlines.
///
/// Nonzero exits remain observations. This function does not parse sanitizer text or claim a
/// reproduced bug. No host process execution or local adapter is provided. Create/destroy calls
/// rely on the backend's own lifecycle bounds. The owned cleanup task waits for create to resolve
/// (so ambiguous creation is not abandoned), then runs until destroy finishes even if this future
/// is dropped. Each exec receives the timeout in its request and is also wrapped in a caller-side
/// Tokio timeout, so a backend that ignores the request timeout cannot hang the audit task.
///
/// # Errors
/// Returns an invalid-request error before sandbox creation, or a typed sandbox error. Destruction
/// is attempted after any execution failure once creation succeeded.
pub async fn run_dynamic_stress<B: SandboxBackend + 'static>(
    plan: &StressPlan,
    backend: Arc<B>,
) -> Result<DynamicReport, HackerError> {
    if plan.argv.is_empty() || plan.argv.first().is_none_or(String::is_empty) {
        return Err(HackerError::InvalidRequest(
            "argv must contain an executable",
        ));
    }
    if plan.timeout.is_zero() || plan.timeout > MAX_EXEC_TIMEOUT {
        return Err(HackerError::InvalidRequest(
            "timeout is outside supported bounds",
        ));
    }
    if plan.output_limit_bytes == 0 || plan.output_limit_bytes > MAX_OUTPUT_BYTES {
        return Err(HackerError::InvalidRequest(
            "output cap is outside supported bounds",
        ));
    }
    if plan.cases.len() > MAX_DYNAMIC_CASES
        || plan
            .cases
            .iter()
            .any(|case| case.len() > MAX_DYNAMIC_CASE_BYTES)
    {
        return Err(HackerError::InvalidRequest(
            "dynamic input corpus exceeds supported bounds",
        ));
    }
    if plan.argv.len() > MAX_ARG_COUNT
        || plan.argv.iter().fold(0usize, |total, argument| {
            total.saturating_add(argument.len())
        }) > MAX_ARG_BYTES
    {
        return Err(HackerError::InvalidRequest("argv exceeds supported bounds"));
    }
    if plan
        .sandbox
        .name
        .as_ref()
        .is_some_and(|name| name.len() > 128)
        || plan
            .sandbox
            .image
            .as_ref()
            .is_some_and(|image| image.len() > 1024)
    {
        return Err(HackerError::InvalidRequest(
            "sandbox name or image exceeds supported bounds",
        ));
    }
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| HackerError::InvalidRequest("a Tokio runtime is required"))?;
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    let _cancel_on_drop = CancelOnDrop(Some(cancel_sender));
    let owned_plan = plan.clone();
    let owner =
        runtime.spawn(async move { run_owned_stress(owned_plan, backend, cancel_receiver).await });
    owner
        .await
        .map_err(|error| HackerError::Sandbox(format!("stress owner task failed: {error}")))?
}

struct CancelOnDrop(Option<watch::Sender<bool>>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _sent = sender.send(true);
        }
    }
}

async fn run_owned_stress<B: SandboxBackend + 'static>(
    plan: StressPlan,
    backend: Arc<B>,
    mut cancelled: watch::Receiver<bool>,
) -> Result<DynamicReport, HackerError> {
    let started = Instant::now();
    let redactor = DfaRedactor::default_redactor()?;
    let sandbox = backend
        .create(&plan.sandbox)
        .await
        .map_err(|error| HackerError::Sandbox(error.to_string()))?;
    let mut report = DynamicReport::default();
    let mut execution_error = None;

    for (input_index, input) in plan.cases.iter().enumerate() {
        if *cancelled.borrow() {
            execution_error = Some("audit caller cancelled".to_owned());
            break;
        }
        let request = ExecRequest {
            argv: plan.argv.clone(),
            stdin: Some(input.clone().into()),
            timeout: Some(plan.timeout),
            output_limit_bytes: Some(plan.output_limit_bytes),
            ..ExecRequest::default()
        };
        let execution = tokio::select! {
            biased;
            changed = cancelled.changed() => {
                let _ = changed;
                Err("audit caller cancelled".to_owned())
            }
            result = tokio::time::timeout(plan.timeout, backend.exec(&sandbox, &request)) => {
                match result {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(_) => Err("sandbox exec exceeded caller-side deadline".to_owned()),
                }
            }
        };
        match execution {
            Ok(output) => {
                if output.stdout.len().saturating_add(output.stderr.len()) > plan.output_limit_bytes
                {
                    execution_error = Some("backend output exceeded requested cap".to_owned());
                    break;
                }
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _stdout_stats = redactor.redact_into(&output.stdout, &mut stdout);
                let _stderr_stats = redactor.redact_into(&output.stderr, &mut stderr);
                if stdout.len().saturating_add(stderr.len()) > plan.output_limit_bytes {
                    execution_error = Some("redacted output exceeded requested cap".to_owned());
                    break;
                }
                report.runs.push(DynamicRun {
                    argv: plan.argv.clone(),
                    seed: plan.seed,
                    input_index,
                    stdin: input.clone(),
                    sanitizer: plan.sanitizer,
                    exit_code: output.exit_code,
                    stdout,
                    stderr,
                    duration: output.duration,
                });
            }
            Err(error) => {
                execution_error = Some(error);
                break;
            }
        }
    }
    let cleanup_error = backend
        .destroy(&sandbox)
        .await
        .err()
        .map(|error| error.to_string());
    report.duration = started.elapsed();
    match (execution_error, cleanup_error) {
        (Some(run), Some(cleanup)) => Err(HackerError::Sandbox(format!(
            "execution failed: {run}; sandbox cleanup failed: {cleanup}"
        ))),
        (Some(error), None) | (None, Some(error)) => Err(HackerError::Sandbox(error)),
        (None, None) => Ok(report),
    }
}
