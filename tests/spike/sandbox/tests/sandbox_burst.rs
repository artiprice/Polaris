//! FAKE-GATEWAY scheduler and ownership contracts, plus an explicitly opt-in real run.
use polaris_kernel_reaper::Reaper;
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_spike_sandbox::MemoryInfo;
use polaris_spike_sandbox::{BurstLimits, run_sandboxes};
use std::{
    collections::BTreeSet,
    error::Error,
    io,
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

#[derive(Debug, Default)]
struct MemoryEvidence {
    initial_total_kib: u64,
    min_total_kib: u64,
    max_total_kib: u64,
    peak_used_kib: u64,
    peak_swap_used_kib: u64,
    sample_count: u64,
}

impl MemoryEvidence {
    fn observe(&mut self, sample: MemoryInfo) {
        if self.sample_count == 0 {
            self.initial_total_kib = sample.total_kib;
            self.min_total_kib = sample.total_kib;
            self.max_total_kib = sample.total_kib;
        } else {
            self.min_total_kib = self.min_total_kib.min(sample.total_kib);
            self.max_total_kib = self.max_total_kib.max(sample.total_kib);
        }
        self.peak_used_kib = self.peak_used_kib.max(sample.used_kib());
        self.peak_swap_used_kib = self.peak_swap_used_kib.max(sample.swap_used_kib());
        self.sample_count = self.sample_count.saturating_add(1);
    }
}

async fn sample_meminfo() -> io::Result<MemoryInfo> {
    let contents = if cfg!(windows) {
        let wsl = polaris_platform::wsl::Wsl::locate().map_err(io::Error::other)?;
        let distro =
            polaris_platform::wsl::DistroName::new("Ubuntu-24.04").map_err(io::Error::other)?;
        let output = wsl
            .exec(
                &distro,
                &["/usr/bin/cat", "/proc/meminfo"],
                None,
                Duration::from_secs(2),
            )
            .await
            .map_err(io::Error::other)?;
        if output.exit_code != 0 {
            return Err(io::Error::other("WSL meminfo probe failed"));
        }
        String::from_utf8(output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    } else {
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::fs::read_to_string("/proc/meminfo"),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "/proc/meminfo read timed out"))??
    };
    MemoryInfo::parse(&contents).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn monitor_meminfo(
    initial: MemoryInfo,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> io::Result<MemoryEvidence> {
    let mut evidence = MemoryEvidence::default();
    evidence.observe(initial);
    loop {
        tokio::select! {
            _ = &mut stop => return Ok(evidence),
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                evidence.observe(sample_meminfo().await?);
            }
        }
    }
}

fn latency_quantiles(values: &[u64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |numerator: usize, denominator: usize| {
        if sorted.is_empty() {
            return None;
        }
        let rank = sorted
            .len()
            .saturating_mul(numerator)
            .div_ceil(denominator)
            .max(1);
        sorted.get(rank.saturating_sub(1)).copied()
    };
    serde_json::json!({
        "count": sorted.len(),
        "p50_us": at(50, 100),
        "p90_us": at(90, 100),
        "p95_us": at(95, 100),
        "p99_us": at(99, 100),
        "max_us": sorted.last().copied(),
    })
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
async fn live_twenty_by_five() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use polaris_openshell::{
        client::{GatewayTls, OpenShellBackend, OpenShellConfig},
        policy::{AgentRole, ExecTier, policy_for},
    };
    let keepalive = if cfg!(windows) {
        let wsl = polaris_platform::wsl::Wsl::locate()?;
        let distro = polaris_platform::wsl::DistroName::new("Ubuntu-24.04")?;
        Some(wsl.keepalive(&distro).await?)
    } else {
        None
    };
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| "TLS provider")?;
    }
    let initial_memory = sample_meminfo().await?;
    if !initial_memory.within_six_gib_reference() {
        return Err(format!(
            "refusing live sandbox creation: MemTotal {} KiB is outside the 5.5–6 GiB reference range",
            initial_memory.total_kib
        )
        .into());
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
    let mut config = OpenShellConfig::wsl2(tls);
    if let Some(endpoint) = std::env::var_os("POLARIS_OPEN_SHELL_ENDPOINT") {
        config.endpoint = endpoint.to_string_lossy().into_owned();
    }
    let backend = Arc::new(OpenShellBackend::connect(config).await?);
    let policy = backend.register_policy(&policy_for(AgentRole::Reader, ExecTier::Docker)?)?;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let mut monitor = tokio::spawn(monitor_meminfo(initial_memory, stop_rx));
    let burst_result = run_sandboxes(
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
    .await;
    let _monitor_stop_sent = stop_tx.send(());
    let monitor_result = match tokio::time::timeout(Duration::from_secs(5), &mut monitor).await {
        Ok(Ok(result)) => result.map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>),
        Ok(Err(error)) => Err(Box::new(error) as Box<dyn Error + Send + Sync>),
        Err(_) => {
            monitor.abort();
            let _joined = monitor.await;
            Err(Box::new(io::Error::new(
                io::ErrorKind::TimedOut,
                "memory monitor did not stop within 5 seconds",
            )) as Box<dyn Error + Send + Sync>)
        }
    };
    let cleanup_result = reaper
        .shutdown_all(&polaris_kernel_reaper::ShutdownConfig {
            overall: Duration::from_secs(120),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(30),
            ..Default::default()
        })
        .await;
    let unresolved = match &cleanup_result {
        Ok(cleanup) => {
            cleanup.deadline_exceeded
                || cleanup.journal_error.is_some()
                || !reaper.leak_report().entries.is_empty()
        }
        Err(_) => true,
    };
    if unresolved {
        let retained = directory.keep();
        return Err(format!(
            "cleanup unresolved; recovery journal retained at {}: {cleanup_result:?}",
            retained.display()
        )
        .into());
    }
    let _cleanup = cleanup_result?;
    let report = burst_result?;
    let memory = monitor_result?;
    assert!(memory.sample_count >= 2, "insufficient meminfo samples");
    assert_eq!(
        memory.initial_total_kib, initial_memory.total_kib,
        "initial memory sample was lost"
    );
    println!(
        "REAL_OPEN_SHELL_BURST_EVIDENCE {}",
        serde_json::json!({
            "memory_total_kib_initial": memory.initial_total_kib,
            "memory_total_kib_min": memory.min_total_kib,
            "memory_total_kib_max": memory.max_total_kib,
            "peak_used_kib": memory.peak_used_kib,
            "peak_swap_used_kib": memory.peak_swap_used_kib,
            "sample_count": memory.sample_count,
            "create_latency": latency_quantiles(&report.create_latency_us),
        })
    );
    println!("REAL_OPEN_SHELL_BURST {report:?}");
    assert_eq!(report.requested, 100);
    assert_eq!(report.created, 100);
    assert_eq!(report.executed, 100);
    assert_eq!(report.destroyed, 100);
    assert!(report.peak_concurrent <= 20 && report.peak_concurrent > 1);
    assert_eq!(report.create_latency_us.len(), 100);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.remaining_resources, 0);
    if let Some(owner) = keepalive {
        owner.shutdown().await?;
    }
    Ok(())
}
