//! Real ENOSPC in a quota-limited guest tmpfs; never fill the gateway or host filesystem.
use bytes::Bytes;
use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};
use polaris_openshell::{
    client::{GatewayTls, OpenShellBackend, OpenShellConfig, PolicyCatalog, ScratchQuota},
    policy::{AgentRole, ExecTier, policy_for},
};
use polaris_sdk::{
    ids::CommandId,
    registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use std::{path::PathBuf, sync::Arc, time::Duration};

type Failure = Box<dyn std::error::Error + Send + Sync>;
struct Remote {
    backend: Arc<OpenShellBackend<PolicyCatalog>>,
    handle: SandboxHandle,
}
impl Teardown for Remote {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))?;
        Ok(Disposition::Clean)
    }
    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))
    }
}

async fn scratch_size(
    backend: &OpenShellBackend<PolicyCatalog>,
    handle: &SandboxHandle,
) -> Result<u64, Failure> {
    let output = backend
        .exec(
            handle,
            &ExecRequest {
                argv: vec![
                    "stat".into(),
                    "-f".into(),
                    "-c".into(),
                    "%S:%b:%T".into(),
                    ScratchQuota::PATH.into(),
                ],
                timeout: Some(Duration::from_secs(10)),
                ..Default::default()
            },
        )
        .await?;
    if output.exit_code != 0 {
        return Err("scratch filesystem probe failed".into());
    }
    let text = std::str::from_utf8(&output.stdout)?.trim();
    let mut fields = text.split(':');
    let block_bytes = fields.next().ok_or("missing block bytes")?.parse::<u64>()?;
    let blocks = fields.next().ok_or("missing block count")?.parse::<u64>()?;
    if fields.next() != Some("tmpfs") || fields.next().is_some() {
        return Err("scratch is not the required tmpfs".into());
    }
    Ok(block_bytes
        .checked_mul(blocks)
        .ok_or("scratch capacity overflow")?)
}

#[ignore = "requires the immutable P2 image, local OpenShell gateway and mTLS credentials"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_enospc_is_confined_and_cleanup_is_authoritative() -> Result<(), Failure> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| "TLS provider")?;
    }
    let keepalive = if cfg!(windows) {
        let wsl = polaris_platform::wsl::Wsl::locate()?;
        let distro = polaris_platform::wsl::DistroName::new("Ubuntu-24.04")?;
        Some(wsl.keepalive(&distro).await?)
    } else {
        None
    };
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE")?;
    let mtls = PathBuf::from(
        std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR").ok_or("set POLARIS_OPEN_SHELL_MTLS_DIR")?,
    );
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    };
    let mut config = OpenShellConfig::wsl2(tls);
    if let Ok(endpoint) = std::env::var("POLARIS_OPEN_SHELL_ENDPOINT") {
        config.endpoint = endpoint;
    }
    let backend = Arc::new(OpenShellBackend::connect(config).await?);
    let policy = backend.register_policy(&policy_for(AgentRole::Reader, ExecTier::Docker)?)?;
    let suffix: String = CommandId::new_v7()
        .as_uuid()
        .simple()
        .to_string()
        .chars()
        .rev()
        .take(16)
        .collect();
    let handle = SandboxHandle::new(&format!("fs-{suffix}"))?;
    let directory = tempfile::tempdir()?;
    let reaper = Reaper::new(directory.path().join("run"))?;
    let registration = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label(handle.as_str().to_owned()),
        Remote {
            backend: Arc::clone(&backend),
            handle: handle.clone(),
        },
    )?;
    let operation: Result<(), Failure> = async {
        let spec = SandboxSpec { name: Some(handle.as_str().to_owned()), image: Some(image), policy: Some(policy), cpu_millis: Some(1_000), memory_mib: Some(128) };
        let actual = match backend.create_with_scratch(&spec, ScratchQuota::new(1024 * 1024)?).await {
            Err(SandboxError::AlreadyExists(error)) => { reaper.release(registration)?; return Err(format!("refused ownership collision: {error}").into()); },
            result => result?,
        };
        if actual != handle {
            let _actual_registration = reaper.register_journaled(ResourceKind::Sandbox, Locator::Label(actual.as_str().to_owned()), Remote { backend: Arc::clone(&backend), handle: actual })?;
            return Err("backend changed requested identity".into());
        }
        let capacity = scratch_size(backend.as_ref(), &handle).await?;
        if capacity != 1024 * 1024 { return Err("scratch quota was not applied exactly".into()); }
        let limited = backend.exec(&handle, &ExecRequest {
            argv: vec!["sh".into(), "-c".into(), "printf 12345; printf 67890 >&2".into()],
            output_limit_bytes: Some(9), timeout: Some(Duration::from_secs(10)), ..Default::default()
        }).await;
        if !matches!(limited, Err(SandboxError::LimitExceeded(_))) {
            return Err("combined execution capture cap was not enforced".into());
        }
        // Each stdin request is below the gateway's per-message bound. The second write exceeds
        // the independently observed filesystem capacity; stdout/stderr remain bounded by adapter.
        backend.upload(&handle, "/sandbox/scratch/first.bin", Bytes::from(vec![0x41; 768 * 1024])).await?;
        let failed = backend.exec(&handle, &ExecRequest {
            argv: vec!["sh".into(), "-c".into(), "cat > /sandbox/scratch/second.bin".into()],
            stdin: Some(Bytes::from(vec![0x42; 768 * 1024])),
            env: vec![("LC_ALL".into(), "C".into())],
            timeout: Some(Duration::from_secs(10)), ..Default::default()
        }).await?;
        if failed.exit_code == 0 || !std::str::from_utf8(&failed.stderr)?.contains("No space left on device") {
            return Err("write did not produce an actual ENOSPC diagnostic".into());
        }
        backend.upload(&handle, "/sandbox/healthy.txt", Bytes::from_static(b"outside-quota-remains-writable\n")).await?;
        let removed = backend.exec(&handle, &ExecRequest { argv: vec!["rm".into(), "-f".into(), "/sandbox/scratch/first.bin".into(), "/sandbox/scratch/second.bin".into()], timeout: Some(Duration::from_secs(10)), ..Default::default() }).await?;
        if removed.exit_code != 0 { return Err("owned scratch cleanup failed".into()); }
        backend.upload(&handle, "/sandbox/scratch/recovered.bin", Bytes::from(vec![0x43; 768 * 1024])).await?;
        println!("REAL_ENOSPC capacity_bytes={capacity} exit_code={} diagnostic=true outside_quota_writable=true capacity_recovered=true combined_capture_limit=true", failed.exit_code);
        Ok(())
    }.await;
    let cleanup = reaper
        .shutdown_all(&polaris_kernel_reaper::ShutdownConfig {
            overall: Duration::from_secs(90),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(20),
            ..Default::default()
        })
        .await;
    let leaks = reaper.leak_report();
    let unresolved = match &cleanup {
        Ok(report) => {
            report.deadline_exceeded || report.journal_error.is_some() || !leaks.entries.is_empty()
        }
        Err(_) => true,
    };
    if unresolved {
        let journal = directory.keep();
        return Err(format!(
            "cleanup unresolved; recovery journal preserved at {}: {cleanup:?}",
            journal.display()
        )
        .into());
    }
    if let Some(owner) = keepalive {
        owner.shutdown().await?;
    }
    operation
}
