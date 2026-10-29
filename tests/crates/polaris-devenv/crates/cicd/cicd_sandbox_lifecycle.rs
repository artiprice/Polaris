use bytes::Bytes;
use polaris_devenv_cicd::{
    CacheMount, CiEngine, CiError, CiLogStore, Draft, ImageRef, Leg, LegAvailability, Matrix,
    Pipeline, SandboxEngine, SandboxEngineConfig, Step, StepName, Validated,
};
use polaris_sdk::{
    ids::Cid,
    registry::{
        ExecOutput, ExecRequest, RedactStats, Redactor, SandboxBackend, SandboxError,
        SandboxHandle, SandboxSpec,
    },
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use std::{error::Error, io};
use tokio::sync::Notify;

type TestResult = Result<(), Box<dyn Error>>;

fn test_error(message: &str) -> Box<dyn Error> {
    Box::new(io::Error::other(message))
}

struct State {
    create_started: Notify,
    create_release: Notify,
    exec_started: Notify,
    exec_release: Notify,
    destroyed: Notify,
    create_count: AtomicUsize,
    destroy_count: AtomicUsize,
    exec_count: AtomicUsize,
    output_size: usize,
    pending_create: bool,
    pending_exec: bool,
    fail_exec: bool,
    fail_destroy: bool,
    destroy_delay: Mutex<Duration>,
    request_limit: Mutex<Option<usize>>,
    stored_logs: AtomicUsize,
}

#[derive(Clone)]
struct Backend(Arc<State>);

impl SandboxBackend for Backend {
    async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        self.0.create_count.fetch_add(1, Ordering::SeqCst);
        if self.0.pending_create {
            self.0.create_started.notify_one();
            self.0.create_release.notified().await;
        }
        SandboxHandle::new("cicd-fixture")
            .map_err(|error| SandboxError::Protocol(error.to_string()))
    }

    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        self.0.exec_count.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut limit) = self.0.request_limit.lock() {
            *limit = request.output_limit_bytes;
        }
        if self.0.pending_exec {
            self.0.exec_started.notify_one();
            self.0.exec_release.notified().await;
        }
        if self.0.fail_exec {
            return Err(SandboxError::Unavailable("fixture exec failure".into()));
        }
        Ok(ExecOutput {
            exit_code: 0,
            stdout: vec![b'x'; self.0.output_size].into(),
            stderr: Bytes::new(),
            duration: Duration::from_millis(1),
        })
    }

    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
        _bytes: Bytes,
    ) -> Result<(), SandboxError> {
        Ok(())
    }

    async fn download(&self, _sandbox: &SandboxHandle, _path: &str) -> Result<Bytes, SandboxError> {
        Ok(Bytes::new())
    }

    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Ok(Vec::new())
    }

    async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        self.0.destroy_count.fetch_add(1, Ordering::SeqCst);
        self.0.destroyed.notify_one();
        let delay = match self.0.destroy_delay.lock() {
            Ok(duration) => *duration,
            Err(_) => Duration::ZERO,
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if self.0.fail_destroy {
            return Err(SandboxError::Unavailable("fixture destroy failure".into()));
        }
        Ok(())
    }
}

struct Passthrough;

impl Redactor for Passthrough {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}

struct Logs(Arc<State>);

impl CiLogStore for Logs {
    fn store(&self, _redacted_log: &[u8]) -> Result<Cid, String> {
        self.0.stored_logs.fetch_add(1, Ordering::SeqCst);
        Ok(Cid::from_bytes([7_u8; 32]))
    }
}

fn state(
    pending_create: bool,
    pending_exec: bool,
    fail_exec: bool,
    fail_destroy: bool,
    output_size: usize,
) -> Arc<State> {
    Arc::new(State {
        create_started: Notify::new(),
        create_release: Notify::new(),
        exec_started: Notify::new(),
        exec_release: Notify::new(),
        destroyed: Notify::new(),
        create_count: AtomicUsize::new(0),
        destroy_count: AtomicUsize::new(0),
        exec_count: AtomicUsize::new(0),
        output_size,
        pending_create,
        pending_exec,
        fail_exec,
        fail_destroy,
        destroy_delay: Mutex::new(Duration::ZERO),
        request_limit: Mutex::new(None),
        stored_logs: AtomicUsize::new(0),
    })
}

fn engine(
    state: Arc<State>,
    lifecycle_timeout: Duration,
) -> Result<SandboxEngine<Backend, Passthrough, Logs>, CiError> {
    let config = SandboxEngineConfig {
        sandbox: SandboxSpec::default(),
        lifecycle_timeout,
        max_log_bytes: 64,
    };
    SandboxEngine::new(
        Backend(Arc::clone(&state)),
        Passthrough,
        Logs(state),
        config,
    )
}

fn pipeline_with_steps(count: usize) -> Result<Pipeline<Validated>, CiError> {
    let mut steps = Vec::with_capacity(count);
    for index in 0..count {
        steps.push(Step {
            name: StepName::new(format!("step-{index}"))?,
            image: ImageRef::new("rust:stable")?,
            cmd: vec!["cargo".into(), "check".into()],
            depends_on: Vec::new(),
            cache_mounts: Vec::<CacheMount>::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(1),
            workdir: None,
        });
    }
    Pipeline::<Draft>::new(steps).validate()
}

fn matrix(legs: usize) -> Matrix {
    Matrix {
        legs: (0..legs)
            .map(|index| Leg {
                name: format!("leg-{index}"),
                required: true,
                availability: LegAvailability::Available,
                env: Vec::new(),
            })
            .collect(),
    }
}

async fn wait_for_destroy(state: &State) -> bool {
    tokio::time::timeout(Duration::from_secs(1), state.destroyed.notified())
        .await
        .is_ok()
}

#[tokio::test]
async fn caller_abort_during_exec_keeps_cleanup_owned() -> TestResult {
    let state = state(false, true, false, false, 1);
    let engine = engine(Arc::clone(&state), Duration::from_secs(1))?;
    let pipeline = pipeline_with_steps(2)?;
    let run = tokio::spawn(async move { engine.run(&pipeline, &matrix(1)).await });
    if tokio::time::timeout(Duration::from_secs(1), state.exec_started.notified())
        .await
        .is_err()
    {
        return Err(test_error("exec did not start"));
    }
    run.abort();
    let _cancelled_run = run.await;
    state.exec_release.notify_one();
    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            _ = state.exec_started.notified() => false,
            _ = state.destroyed.notified() => true,
        }
    })
    .await;
    match observed {
        Ok(false) => {
            return Err(test_error(
                "a later pipeline step started after caller abort",
            ));
        }
        Ok(true)
            if state.destroy_count.load(Ordering::SeqCst) == 1
                && state.exec_count.load(Ordering::SeqCst) == 1 => {}
        _ => return Err(test_error("owned cleanup was not observed")),
    }
    Ok(())
}

#[tokio::test]
async fn caller_abort_during_create_destroys_late_handle_without_exec() -> TestResult {
    let state = state(true, false, false, false, 1);
    let engine = engine(Arc::clone(&state), Duration::from_secs(1))?;
    let pipeline = pipeline_with_steps(1)?;
    let run = tokio::spawn(async move { engine.run(&pipeline, &matrix(1)).await });
    if tokio::time::timeout(Duration::from_secs(1), state.create_started.notified())
        .await
        .is_err()
    {
        return Err(test_error("create did not start"));
    }
    run.abort();
    let _cancelled_run = run.await;
    state.create_release.notify_one();
    if !wait_for_destroy(&state).await
        || state.destroy_count.load(Ordering::SeqCst) != 1
        || state.exec_count.load(Ordering::SeqCst) != 0
    {
        return Err(test_error("late create was executed or not destroyed"));
    }
    Ok(())
}

#[tokio::test]
async fn create_timeout_preserves_late_create_result_for_cleanup() -> TestResult {
    let state = state(true, false, false, false, 1);
    let engine = engine(Arc::clone(&state), Duration::from_millis(20))?;
    let pipeline = pipeline_with_steps(1)?;
    let run = tokio::spawn(async move { engine.run(&pipeline, &matrix(1)).await });
    if tokio::time::timeout(Duration::from_secs(1), state.create_started.notified())
        .await
        .is_err()
    {
        return Err(test_error("create did not start"));
    }
    let result = run.await;
    if !matches!(result, Ok(Err(CiError::CreateTimedOutCleanupPending)))
        || state.destroy_count.load(Ordering::SeqCst) != 0
    {
        return Err(test_error("create timeout accounting changed"));
    }
    state.create_release.notify_one();
    if !wait_for_destroy(&state).await || state.destroy_count.load(Ordering::SeqCst) != 1 {
        return Err(test_error("late sandbox was not destroyed"));
    }
    Ok(())
}

#[tokio::test]
async fn exec_requests_have_finite_output_caps_and_overflow_is_rejected() -> TestResult {
    let state = state(false, false, false, false, 65);
    let engine = engine(Arc::clone(&state), Duration::from_secs(1))?;
    let pipeline = pipeline_with_steps(1)?;
    let result = engine.run(&pipeline, &matrix(1)).await;
    let limit = state.request_limit.lock().ok().and_then(|limit| *limit);
    if !matches!(result, Err(CiError::OutputLimitExceeded))
        || limit != Some(64)
        || state.stored_logs.load(Ordering::SeqCst) != 0
        || state.destroy_count.load(Ordering::SeqCst) != 1
    {
        return Err(test_error("output did not fail closed"));
    }
    Ok(())
}

#[tokio::test]
async fn cleanup_failure_is_not_reported_as_success() -> TestResult {
    let state = state(false, false, true, true, 1);
    let engine = engine(Arc::clone(&state), Duration::from_secs(1))?;
    let pipeline = pipeline_with_steps(1)?;
    let result = engine.run(&pipeline, &matrix(1)).await;
    if !matches!(result, Err(CiError::CleanupAfterFailure { .. }))
        || state.destroy_count.load(Ordering::SeqCst) != 1
    {
        return Err(test_error("cleanup failure was not surfaced"));
    }
    Ok(())
}

#[tokio::test]
async fn cleanup_timeout_is_reported_without_claiming_destruction() -> TestResult {
    let state = state(false, false, false, false, 1);
    if let Ok(mut delay) = state.destroy_delay.lock() {
        *delay = Duration::from_secs(1);
    } else {
        return Err(test_error("fixture destroy delay lock failed"));
    }
    let engine = engine(Arc::clone(&state), Duration::from_millis(20))?;
    let pipeline = pipeline_with_steps(1)?;
    let result = engine.run(&pipeline, &matrix(1)).await;
    if !matches!(result, Err(CiError::CleanupFailure(_)))
        || state.destroy_count.load(Ordering::SeqCst) != 1
    {
        return Err(test_error("destroy timeout was not surfaced"));
    }
    Ok(())
}

#[tokio::test]
async fn oversized_matrix_is_rejected_before_sandbox_creation() -> TestResult {
    let state = state(false, false, false, false, 1);
    let engine = engine(Arc::clone(&state), Duration::from_secs(1))?;
    let pipeline = pipeline_with_steps(1)?;
    if !matches!(
        engine.run(&pipeline, &matrix(65)).await,
        Err(CiError::InvalidPipeline(_))
    ) || state.create_count.load(Ordering::SeqCst) != 0
    {
        return Err(test_error("oversized matrix reached backend"));
    }
    Ok(())
}

#[test]
fn oversized_pipeline_and_argv_are_rejected() -> TestResult {
    if !matches!(pipeline_with_steps(129), Err(CiError::InvalidPipeline(_))) {
        return Err(test_error("oversized pipeline was accepted"));
    }
    let mut step = Step {
        name: StepName::new("argv")?,
        image: ImageRef::new("rust:stable")?,
        cmd: vec!["cargo".into(), "x".repeat(65_536)],
        depends_on: Vec::new(),
        cache_mounts: Vec::new(),
        env: Vec::new(),
        timeout: Duration::from_secs(1),
        workdir: None,
    };
    if !matches!(
        Pipeline::<Draft>::new(vec![step.clone()]).validate(),
        Err(CiError::InvalidPipeline(_))
    ) {
        return Err(test_error("oversized argv was accepted"));
    }
    step.cmd = vec!["cargo".into()];
    step.env = vec![("VALUE".into(), "x".repeat(65_536))];
    if !matches!(
        Pipeline::<Draft>::new(vec![step]).validate(),
        Err(CiError::InvalidPipeline(_))
    ) {
        return Err(test_error("oversized env was accepted"));
    }
    Ok(())
}
