//! FAKE-GATEWAY scheduler and ownership contracts, plus an explicitly opt-in real run.
use polaris_kernel_reaper::Reaper;
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_spike_sandbox::{BurstLimits, run_sandboxes};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct FakeGateway {
    live: Mutex<BTreeSet<String>>,
    calls: AtomicUsize,
    exec_failure: AtomicBool,
    destroy_failure: AtomicBool,
    collision: AtomicBool,
}
impl SandboxBackend for FakeGateway {
    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.collision.load(Ordering::SeqCst) {
            return Err(SandboxError::AlreadyExists("preexisting fake".into()));
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
        let handle = SandboxHandle::new(
            spec.name
                .as_deref()
                .ok_or_else(|| SandboxError::Protocol("name required".into()))?,
        )
        .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        self.live
            .lock()
            .map_err(|_| SandboxError::Protocol("poisoned".into()))?
            .insert(handle.as_str().to_owned());
        Ok(handle)
    }
    async fn exec(
        &self,
        _sandbox: &SandboxHandle,
        _request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        if self.exec_failure.load(Ordering::SeqCst) {
            return Err(SandboxError::Unavailable("injected exec fault".into()));
        }
        Ok(ExecOutput::default())
    }
    async fn destroy(&self, sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        if self.destroy_failure.load(Ordering::SeqCst) {
            return Err(SandboxError::Unavailable("injected destroy fault".into()));
        }
        self.live
            .lock()
            .map_err(|_| SandboxError::Protocol("poisoned".into()))?
            .remove(sandbox.as_str());
        Ok(())
    }
    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
        _bytes: bytes::Bytes,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied(
            "not part of burst fixture".into(),
        ))
    }
    async fn download(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
    ) -> Result<bytes::Bytes, SandboxError> {
        Err(SandboxError::PolicyDenied(
            "not part of burst fixture".into(),
        ))
    }
    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Err(SandboxError::PolicyDenied(
            "not part of burst fixture".into(),
        ))
    }
}
fn command() -> ExecRequest {
    ExecRequest {
        argv: vec!["printf".into(), "p2-burst".into()],
        timeout: Some(Duration::from_secs(10)),
        ..ExecRequest::default()
    }
}
fn limits() -> BurstLimits {
    BurstLimits {
        concurrent: 4,
        cycles: 5,
        deadline: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn peak_concurrency_within_cap_and_latency_recorded() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let backend = Arc::new(FakeGateway::default());
    let report = run_sandboxes(
        Arc::clone(&backend),
        Arc::clone(&reaper),
        SandboxSpec::default(),
        limits(),
        command(),
    )
    .await?;
    assert_eq!(report.requested, 20);
    assert_eq!(report.created, 20);
    assert_eq!(report.executed, 20);
    assert_eq!(report.destroyed, 20);
    assert_eq!(report.create_latency_us.len(), 20);
    assert!(report.create_latency_us.iter().all(|micros| *micros > 0));
    assert!(report.peak_concurrent > 1 && report.peak_concurrent <= 4);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.remaining_resources, 0);
    assert!(backend.live.lock().map_err(|_| "mutex")?.is_empty());
    assert!(
        Reaper::new(directory.path())?
            .leak_report()
            .entries
            .is_empty()
    );
    Ok(())
}
#[tokio::test]
async fn failures_are_typed_and_owned_cleanup_survives_exec_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let backend = Arc::new(FakeGateway::default());
    backend.exec_failure.store(true, Ordering::SeqCst);
    let report = run_sandboxes(
        Arc::clone(&backend),
        reaper,
        SandboxSpec::default(),
        limits(),
        command(),
    )
    .await?;
    assert_eq!(report.created, 20);
    assert_eq!(report.executed, 0);
    assert_eq!(report.destroyed, 20);
    assert_eq!(report.failures.len(), 20);
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.stage == "exec")
    );
    assert_eq!(report.remaining_resources, 0);
    assert!(backend.live.lock().map_err(|_| "mutex")?.is_empty());
    Ok(())
}
#[tokio::test]
async fn failed_destroy_remains_journaled_until_reaper_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let backend = Arc::new(FakeGateway::default());
    backend.destroy_failure.store(true, Ordering::SeqCst);
    let report = run_sandboxes(
        Arc::clone(&backend),
        Arc::clone(&reaper),
        SandboxSpec::default(),
        BurstLimits {
            concurrent: 2,
            cycles: 1,
            ..limits()
        },
        command(),
    )
    .await?;
    assert_eq!(report.remaining_resources, 2);
    assert_eq!(report.destroyed, 0);
    assert_eq!(
        Reaper::new(directory.path())?.leak_report().entries.len(),
        2
    );
    backend.destroy_failure.store(false, Ordering::SeqCst);
    let shutdown = reaper
        .shutdown_all(&polaris_kernel_reaper::ShutdownConfig::default())
        .await?;
    assert!(!shutdown.deadline_exceeded);
    assert!(shutdown.journal_error.is_none());
    assert!(reaper.leak_report().entries.is_empty());
    assert!(backend.live.lock().map_err(|_| "mutex")?.is_empty());
    Ok(())
}
#[tokio::test]
async fn preexisting_identity_is_never_destroyed() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let backend = Arc::new(FakeGateway::default());
    backend.collision.store(true, Ordering::SeqCst);
    let report = run_sandboxes(
        backend,
        Arc::clone(&reaper),
        SandboxSpec::default(),
        limits(),
        command(),
    )
    .await?;
    assert_eq!(report.created, 0);
    assert_eq!(report.destroyed, 0);
    assert_eq!(report.remaining_resources, 0);
    assert_eq!(report.failures.len(), 20);
    assert!(report.failures.iter().all(|failure| matches!(
        failure.error,
        polaris_spike_sandbox::SpikeError::Backend(SandboxError::AlreadyExists(_))
    )));
    Ok(())
}
#[ignore = "requires actual local OpenShell v0.1.4, mTLS, and immutable POLARIS_OPEN_SHELL_IMAGE"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_twenty_by_five() -> Result<(), Box<dyn std::error::Error>> {
    use polaris_openshell::{
        client::{GatewayTls, OpenShellBackend, OpenShellConfig},
        policy::{AgentRole, ExecTier, policy_for},
    };
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| "TLS provider")?;
    }
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path())?);
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE")?;
    let mtls = std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR")
        .map(std::path::PathBuf::from)
        .ok_or("set POLARIS_OPEN_SHELL_MTLS_DIR")?;
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    };
    let backend = Arc::new(OpenShellBackend::connect(OpenShellConfig::wsl2(tls)).await?);
    let policy = backend.register_policy(&policy_for(AgentRole::Reader, ExecTier::Docker)?)?;
    let report = run_sandboxes(
        backend,
        Arc::clone(&reaper),
        SandboxSpec {
            image: Some(image),
            policy: Some(policy),
            memory_mib: Some(128),
            cpu_millis: Some(250),
            ..SandboxSpec::default()
        },
        BurstLimits::default(),
        command(),
    )
    .await?;
    println!("REAL_OPEN_SHELL_BURST {report:?}");
    let cleanup = reaper
        .shutdown_all(&polaris_kernel_reaper::ShutdownConfig {
            overall: Duration::from_secs(120),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(30),
            ..Default::default()
        })
        .await?;
    assert!(!cleanup.deadline_exceeded, "{cleanup:?}");
    assert!(cleanup.journal_error.is_none(), "{cleanup:?}");
    assert!(
        reaper.leak_report().entries.is_empty(),
        "{:?}",
        reaper.leak_report()
    );
    assert_eq!(report.requested, 100);
    assert_eq!(report.created, 100);
    assert_eq!(report.executed, 100);
    assert_eq!(report.destroyed, 100);
    assert!(report.peak_concurrent <= 20 && report.peak_concurrent > 1);
    assert_eq!(report.create_latency_us.len(), 100);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.remaining_resources, 0);
    Ok(())
}
