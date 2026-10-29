use bytes::Bytes;
use polaris_code_intel::Lang;
use polaris_hacker::{HackerError, Sanitizer, SourceFile, StressPlan, audit, run_dynamic_stress};
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_security_automata::DfaScanner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// Contract fixture only: this records SDK requests and does not provide isolation.
struct FixtureBackend {
    requests: Mutex<Vec<RecordedRequest>>,
    destroyed: AtomicBool,
    destroy_count: std::sync::atomic::AtomicUsize,
    exec_started: Notify,
    destroyed_notify: Notify,
    create_started: Notify,
    create_release: Notify,
    fail_exec: bool,
    pending_exec: bool,
    pending_create: bool,
    exit_code: i32,
    stdout: Vec<u8>,
}

#[derive(Debug)]
struct RecordedRequest {
    argv: Vec<String>,
    stdin: Option<Vec<u8>>,
    timeout: Option<Duration>,
    output_limit_bytes: Option<usize>,
}

impl FixtureBackend {
    fn new(fail_exec: bool, exit_code: i32, stdout: &[u8]) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            destroyed: AtomicBool::new(false),
            destroy_count: std::sync::atomic::AtomicUsize::new(0),
            exec_started: Notify::new(),
            destroyed_notify: Notify::new(),
            create_started: Notify::new(),
            create_release: Notify::new(),
            fail_exec,
            pending_exec: false,
            pending_create: false,
            exit_code,
            stdout: stdout.to_vec(),
        }
    }

    fn pending() -> Self {
        let mut backend = Self::new(false, 0, b"");
        backend.pending_exec = true;
        backend
    }

    fn pending_creation() -> Self {
        let mut backend = Self::new(false, 0, b"");
        backend.pending_create = true;
        backend
    }
}

impl SandboxBackend for FixtureBackend {
    async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        if self.pending_create {
            self.create_started.notify_one();
            self.create_release.notified().await;
        }
        SandboxHandle::new("fixture-sandbox")
            .map_err(|error| SandboxError::Protocol(error.to_string()))
    }

    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        let recorded = RecordedRequest {
            argv: request.argv.clone(),
            stdin: request.stdin.as_ref().map(|bytes| bytes.as_ref().to_vec()),
            timeout: request.timeout,
            output_limit_bytes: request.output_limit_bytes,
        };
        self.requests
            .lock()
            .map_err(|error| SandboxError::Protocol(error.to_string()))?
            .push(recorded);
        if self.pending_exec {
            self.exec_started.notify_one();
            return std::future::pending().await;
        }
        if self.fail_exec {
            return Err(SandboxError::Timeout);
        }
        Ok(ExecOutput {
            exit_code: self.exit_code,
            stdout: self.stdout.clone().into(),
            stderr: Vec::<u8>::new().into(),
            duration: Duration::from_millis(3),
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
        self.destroyed.store(true, Ordering::SeqCst);
        self.destroy_count.fetch_add(1, Ordering::SeqCst);
        self.destroyed_notify.notify_one();
        Ok(())
    }
}

fn plan() -> StressPlan {
    StressPlan {
        sandbox: SandboxSpec::default(),
        argv: vec!["cargo".to_owned(), "test".to_owned()],
        cases: vec![b"0".to_vec(), b"-1".to_vec()],
        timeout: Duration::from_secs(5),
        output_limit_bytes: 256,
        seed: 41,
        sanitizer: Sanitizer::None,
    }
}

#[tokio::test]
async fn sends_bounded_seeded_requests_and_keeps_nonzero_exit_as_observation()
-> Result<(), Box<dyn std::error::Error>> {
    let secret_output = b"token = \"abcdefghijklmnopqrstuvwx\"";
    let backend = Arc::new(FixtureBackend::new(false, 7, secret_output));
    let report = run_dynamic_stress(&plan(), Arc::clone(&backend)).await?;
    assert_eq!(report.runs.len(), 2);
    assert!(backend.destroyed.load(Ordering::SeqCst));
    assert!(report.runs.iter().all(|run| run.exit_code == 7));
    assert_eq!(
        report.runs.first().map(|run| run.stdin.as_slice()),
        Some(b"0".as_slice())
    );
    assert!(
        report.runs.iter().all(|run| {
            !String::from_utf8_lossy(&run.stdout).contains("abcdefghijklmnopqrstuvwx")
        })
    );
    let requests = backend.requests.lock().map_err(|error| error.to_string())?;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .first()
            .and_then(|request| request.argv.first())
            .map(String::as_str),
        Some("cargo")
    );
    assert_eq!(
        requests
            .first()
            .and_then(|request| request.argv.get(1))
            .map(String::as_str),
        Some("test")
    );
    assert_eq!(
        requests
            .first()
            .and_then(|request| request.stdin.as_deref()),
        Some(b"0".as_slice())
    );
    assert!(
        requests
            .iter()
            .all(|request| request.timeout == Some(Duration::from_secs(5)))
    );
    assert!(
        requests
            .iter()
            .all(|request| request.output_limit_bytes == Some(256))
    );
    Ok(())
}

#[tokio::test]
async fn execution_error_attempts_sandbox_cleanup() {
    let backend = Arc::new(FixtureBackend::new(true, 0, b""));
    let result = run_dynamic_stress(&plan(), Arc::clone(&backend)).await;
    assert!(matches!(result, Err(HackerError::Sandbox(_))));
    assert!(backend.destroyed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn backend_output_over_cap_is_rejected_and_sandbox_is_destroyed() {
    let backend = Arc::new(FixtureBackend::new(false, 0, b"0123456789abcdefghij"));
    let mut bounded_plan = plan();
    bounded_plan.output_limit_bytes = 8;
    let result = run_dynamic_stress(&bounded_plan, Arc::clone(&backend)).await;
    assert!(matches!(result, Err(HackerError::Sandbox(_))));
    assert!(backend.destroyed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn aggregate_pipeline_reports_zero_model_tokens() -> Result<(), Box<dyn std::error::Error>> {
    let source = SourceFile {
        path: "src/bug.rs".to_owned(),
        lang: Lang::Rust,
        bytes: b"async fn bug() { std::thread::sleep(Duration::ZERO); }".to_vec(),
    };
    let files = [source];
    let rules = DfaScanner::default_pack()?;
    let backend = Arc::new(FixtureBackend::new(false, 0, b""));
    let report = audit(&files, &rules, Some(&plan()), backend).await?;
    assert_eq!(report.tokens_used(), 0);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.runs.len(), 2);
    Ok(())
}

#[tokio::test]
async fn aborting_caller_during_exec_still_destroys_sandbox()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = Arc::new(FixtureBackend::pending());
    let runner_backend = Arc::clone(&backend);
    let caller = tokio::spawn(async move { run_dynamic_stress(&plan(), runner_backend).await });
    tokio::time::timeout(Duration::from_secs(1), backend.exec_started.notified()).await?;
    caller.abort();
    let _caller_result = caller.await;
    tokio::time::timeout(Duration::from_secs(1), backend.destroyed_notify.notified()).await?;
    assert_eq!(backend.destroy_count.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn backend_ignoring_request_timeout_is_still_bounded_and_destroyed()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = Arc::new(FixtureBackend::pending());
    let mut bounded_plan = plan();
    bounded_plan.timeout = Duration::from_millis(50);
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        run_dynamic_stress(&bounded_plan, Arc::clone(&backend)),
    )
    .await?;
    assert!(matches!(result, Err(HackerError::Sandbox(_))));
    assert_eq!(backend.destroy_count.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn caller_cancellation_does_not_abandon_ambiguous_create()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = Arc::new(FixtureBackend::pending_creation());
    let runner_backend = Arc::clone(&backend);
    let caller = tokio::spawn(async move { run_dynamic_stress(&plan(), runner_backend).await });
    tokio::time::timeout(Duration::from_secs(1), backend.create_started.notified()).await?;
    caller.abort();
    let _caller_result = caller.await;
    assert_eq!(backend.destroy_count.load(Ordering::SeqCst), 0);
    backend.create_release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), backend.destroyed_notify.notified()).await?;
    assert_eq!(backend.destroy_count.load(Ordering::SeqCst), 1);
    Ok(())
}
