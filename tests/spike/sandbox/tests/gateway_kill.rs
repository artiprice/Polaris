//! Kill only a private test gateway during real lifecycles, resume its database, and reap.
use bytes::Bytes;
use polaris_kernel_reaper::{Locator, Reaper, ShutdownConfig};
use polaris_openshell::{
    client::{GatewayTls, OpenShellBackend, OpenShellConfig, PolicyCatalog},
    policy::{AgentRole, ExecTier, policy_for},
};
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_spike_sandbox::{BurstLimits, SpikeError, run_sandboxes};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

type Failure = Box<dyn std::error::Error + Send + Sync>;
const SCRIPT: &str = "/mnt/c/mDev/Polaris/@prime/tools/sandbox/isolated-fault-gateway.sh";

struct Observed {
    inner: Arc<OpenShellBackend<PolicyCatalog>>,
    created: AtomicUsize,
}
impl SandboxBackend for Observed {
    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let handle = self.inner.create(spec).await?;
        self.created.fetch_add(1, Ordering::SeqCst);
        Ok(handle)
    }
    async fn exec(
        &self,
        sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        self.inner.exec(sandbox, request).await
    }
    async fn upload(
        &self,
        sandbox: &SandboxHandle,
        path: &str,
        bytes: Bytes,
    ) -> Result<(), SandboxError> {
        self.inner.upload(sandbox, path, bytes).await
    }
    async fn download(&self, sandbox: &SandboxHandle, path: &str) -> Result<Bytes, SandboxError> {
        self.inner.download(sandbox, path).await
    }
    async fn list_files(
        &self,
        sandbox: &SandboxHandle,
        path: &str,
    ) -> Result<Vec<String>, SandboxError> {
        self.inner.list_files(sandbox, path).await
    }
    async fn destroy(&self, sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        self.inner.destroy(sandbox).await
    }
}

fn control(mode: &str, state: &str) -> Command {
    let mut command = Command::new("wsl.exe");
    command.args(["-d", "Ubuntu-24.04", "--exec", "bash", SCRIPT, mode, state]);
    command.kill_on_drop(true).stdin(Stdio::null());
    command
}

async fn stop(state: &str, child: &mut Child) -> Result<(), Failure> {
    let output =
        tokio::time::timeout(Duration::from_secs(20), control("stop", state).output()).await??;
    if !output.status.success() {
        return Err(format!(
            "owned gateway stop failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let _status = tokio::time::timeout(Duration::from_secs(10), child.wait()).await??;
    Ok(())
}

async fn namespace_ids(namespace: &str) -> Result<Vec<String>, Failure> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new("wsl.exe")
            .args([
                "-d",
                "Ubuntu-24.04",
                "--exec",
                "docker",
                "ps",
                "-aq",
                "--filter",
                &format!("label=openshell.ai/sandbox-namespace={namespace}"),
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if !output.status.success() {
        return Err("Docker namespace probe failed".into());
    }
    let text = String::from_utf8(output.stdout)?;
    let ids: Vec<String> = text.lines().map(str::trim).map(str::to_owned).collect();
    if ids
        .iter()
        .any(|id| id.len() != 12 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("Docker returned invalid container identities".into());
    }
    Ok(ids)
}

async fn wait_absent(
    backend: &OpenShellBackend<PolicyCatalog>,
    handle: &SandboxHandle,
) -> Result<(), Failure> {
    // Pinned gateway v0.1.4 compute/mod.rs: ORPHAN_GRACE_PERIOD = 5 min and
    // RECONCILE_INTERVAL = 1 min. A lost deletion watch event is reconciled within
    // that window; a 30-second metadata disappearance assumption is invalid.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(360);
    loop {
        let observation = tokio::time::timeout_at(deadline, backend.inspect(handle)).await??;
        if observation.is_none() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "resource still present after bounded reconciliation: {observation:?}"
            )
            .into());
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(250)).min(deadline),
        )
        .await;
    }
}

async fn require_no_namespace_resources(namespace: &str) -> Result<(), Failure> {
    if !namespace_ids(namespace).await?.is_empty() {
        return Err("authoritative Docker namespace retains containers".into());
    }
    for resource in ["volume", "network"] {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            Command::new("wsl.exe")
                .args([
                    "-d",
                    "Ubuntu-24.04",
                    "--exec",
                    "docker",
                    resource,
                    "ls",
                    "-q",
                    "--filter",
                    &format!("label=openshell.ai/sandbox-namespace={namespace}"),
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        if !output.status.success() || !output.stdout.is_empty() {
            return Err(format!("authoritative Docker namespace retains {resource} resources or could not be queried").into());
        }
    }
    Ok(())
}

async fn wait_gateway(
    config: &OpenShellConfig,
    child: &mut Child,
) -> Result<OpenShellBackend<PolicyCatalog>, Failure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(format!("private gateway exited during recovery: {status}").into());
        }
        match tokio::time::timeout_at(deadline, OpenShellBackend::connect(config.clone())).await? {
            Ok(backend) => return Ok(backend),
            Err(SandboxError::Unavailable(_) | SandboxError::Timeout) => (),
            Err(error) => return Err(error.into()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("private gateway startup exceeded 120 seconds".into());
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_secs(1)).min(deadline),
        )
        .await;
    }
}

#[ignore = "destructively interrupts only an explicitly supplied private fault gateway"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_gateway_kill_retains_ownership_and_recovers_without_leaks() -> Result<(), Failure> {
    if !cfg!(windows) {
        return Err("this fault harness controls the Windows-owned WSL gateway".into());
    }
    let wsl = polaris_platform::wsl::Wsl::locate()?;
    let distro = polaris_platform::wsl::DistroName::new("Ubuntu-24.04")?;
    let keepalive = wsl.keepalive(&distro).await?;
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| "TLS provider")?;
    }
    let state = std::env::var("POLARIS_FAULT_GATEWAY_STATE")?;
    let token = state
        .strip_prefix("/home/dev/.local/state/polaris/p2-gateway.")
        .ok_or("not a private test state")?;
    if token.len() != 10 || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("invalid private gateway identity".into());
    }
    let namespace = format!("p2-fault-{token}");
    let mtls = PathBuf::from(
        std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR").ok_or("mTLS directory missing")?,
    );
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    };
    let mut config = OpenShellConfig::wsl2(tls);
    config.endpoint = "https://localhost:17771".into();
    config.ready_timeout = Duration::from_secs(30);
    let inner = Arc::new(OpenShellBackend::connect(config.clone()).await?);
    let policy = inner.register_policy(&policy_for(AgentRole::Reader, ExecTier::Docker)?)?;
    let backend = Arc::new(Observed {
        inner,
        created: AtomicUsize::new(0),
    });
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path().join("run"))?);
    let burst = run_sandboxes(
        Arc::clone(&backend),
        Arc::clone(&reaper),
        SandboxSpec {
            image: Some(std::env::var("POLARIS_OPEN_SHELL_IMAGE")?),
            policy: Some(policy),
            cpu_millis: Some(250),
            memory_mib: Some(128),
            ..Default::default()
        },
        BurstLimits {
            concurrent: 4,
            cycles: 2,
            deadline: Duration::from_secs(35),
        },
        ExecRequest {
            argv: vec!["sleep".into(), "30".into()],
            timeout: Some(Duration::from_secs(35)),
            ..Default::default()
        },
    );
    tokio::pin!(burst);
    let started = Instant::now();
    let observed = async {
        while backend.created.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let admitted = tokio::select! {
        () = observed => Ok(()),
        result = &mut burst => Err(format!("burst ended before the fault: {result:?}")),
        () = tokio::time::sleep(Duration::from_secs(45)) => Err("no two real sandboxes became ready".into()),
    };
    let fault_result: Result<_, Failure> = async {
        admitted.map_err(|error| -> Failure { error.into() })?;
        let before = namespace_ids(&namespace).await?;
        if before.len() < 2 {
            return Err("namespace leak probe did not observe the live owned containers".into());
        }
        println!(
            "FAULT_POSITIVE_CONTROL namespace_containers={}",
            before.len()
        );
        let output =
            tokio::time::timeout(Duration::from_secs(20), control("crash", &state).output())
                .await??;
        if !output.status.success() {
            return Err(format!(
                "private gateway crash failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let report = tokio::time::timeout(Duration::from_secs(100), &mut burst).await??;
        if report.created < 2
            || report.failures.is_empty()
            || report.executed == report.requested
            || report.remaining_resources == 0
        {
            return Err(format!("fault did not retain unresolved ownership: {report:?}").into());
        }
        if !report
            .failures
            .iter()
            .any(|failure| matches!(failure.error, SpikeError::Backend(_) | SpikeError::Deadline))
        {
            return Err("no typed backend/deadline failure was observed".into());
        }
        Ok(report)
    }
    .await;

    // Resume the same private DB, CA and namespace even when a fault assertion failed.
    let mut resumed = control("resume", &state)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;
    let recovery: Result<(), Failure> = async {
        let _fresh = wait_gateway(&config, &mut resumed).await?;
        let cleanup = reaper
            .shutdown_all(&ShutdownConfig {
                overall: Duration::from_secs(90),
                graceful: Duration::from_secs(60),
                forced: Duration::from_secs(20),
                ..Default::default()
            })
            .await?;
        if cleanup.deadline_exceeded || cleanup.journal_error.is_some() {
            return Err(format!("recovery left journal ownership: {cleanup:?}").into());
        }
        let remaining = reaper.leak_report().entries;
        if !remaining.is_empty() {
            // A lost deletion watch event leaves a Deleting row until the gateway's
            // periodic orphan reconciliation. Retain ownership throughout that bounded
            // window; never repeat deletion by a potentially reused name.
            println!(
                "FAULT_RECONCILIATION_WAIT unresolved={} first_cleanup={cleanup:?}",
                remaining.len()
            );
            let fresh = OpenShellBackend::connect(config).await?;
            for entry in remaining {
                let Locator::Label(name) = entry.locator else {
                    return Err("unexpected recovery locator".into());
                };
                let handle = SandboxHandle::new(&name)?;
                wait_absent(&fresh, &handle).await?;
                reaper.release(entry.id)?;
            }
        }
        if !reaper.leak_report().entries.is_empty() {
            return Err("recovery journal still has unresolved entries".into());
        }
        require_no_namespace_resources(&namespace).await?;
        Ok(())
    }
    .await;
    let stopped = stop(&state, &mut resumed).await;
    if recovery.is_err() {
        let retained = directory.keep();
        eprintln!("FAULT_RECOVERY_JOURNAL {}", retained.display());
    }
    recovery?;
    stopped?;
    let report = fault_result?;
    keepalive.shutdown().await?;
    println!(
        "REAL_GATEWAY_KILL created={} failures={} retained_before_recovery={} remaining_resources=0 namespace_empty=true gateway_stopped=true elapsed_ms={}",
        report.created,
        report.failures.len(),
        report.remaining_resources,
        started.elapsed().as_millis()
    );
    Ok(())
}
