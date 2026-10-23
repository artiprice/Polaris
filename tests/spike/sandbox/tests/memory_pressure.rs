//! Actual cgroup OOM, with independently checked limits and a bounded 512 MiB workload.
use bytes::Bytes;
use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
    TeardownError,
};
use polaris_openshell::{
    client::{GatewayTls, OpenShellBackend, OpenShellConfig, PolicyCatalog},
    policy::{AgentRole, ExecTier, policy_for},
};
use polaris_sdk::{
    ids::CommandId,
    registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::process::Command;

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
async fn docker(arguments: &[&str]) -> Result<String, Failure> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new("wsl.exe")
            .args(["-d", "Ubuntu-24.04", "--exec", "docker"])
            .args(arguments)
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if !output.status.success() {
        return Err(format!(
            "independent Docker probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
async fn oom_kills(container: &str) -> Result<u64, Failure> {
    let events = docker(&["exec", container, "cat", "/sys/fs/cgroup/memory.events"]).await?;
    let mut found = None;
    for line in events.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some("oom_kill") {
            if found.is_some() {
                return Err("duplicate oom_kill field".into());
            }
            found = Some(fields.next().ok_or("missing OOM count")?.parse()?);
            if fields.next().is_some() {
                return Err("invalid OOM counter".into());
            }
        }
    }
    Ok(found.ok_or("cgroup v2 OOM counter unavailable")?)
}

#[ignore = "requires an explicitly supplied private fault gateway and the immutable P2 image"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_memory_pressure_is_contained_and_recoverable() -> Result<(), Failure> {
    if !cfg!(windows) {
        return Err("this independent quota probe uses the Windows-owned WSL daemon".into());
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
        .ok_or("private gateway required")?;
    if token.len() != 10 || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("invalid private gateway identity".into());
    }
    let namespace = format!("p2-fault-{token}");
    let mtls = PathBuf::from(
        std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR").ok_or("mTLS directory missing")?,
    );
    let mut config = OpenShellConfig::wsl2(GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    });
    config.endpoint = "https://localhost:17771".into();
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
    let handle = SandboxHandle::new(&format!("mp-{suffix}"))?;
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
        let spec = SandboxSpec { name: Some(handle.as_str().to_owned()), image: Some(std::env::var("POLARIS_OPEN_SHELL_IMAGE")?), policy: Some(policy), cpu_millis: Some(500), memory_mib: Some(128) };
        match backend.create(&spec).await {
            Err(SandboxError::AlreadyExists(detail)) => { reaper.release(registration)?; return Err(format!("refused ownership collision: {detail}").into()); }
            Ok(actual) if actual == handle => (),
            Ok(actual) => { let _registration = reaper.register_journaled(ResourceKind::Sandbox, Locator::Label(actual.as_str().to_owned()), Remote { backend: Arc::clone(&backend), handle: actual })?; return Err("backend changed requested identity".into()); }
            Err(error) => return Err(error.into()),
        }
        let container = docker(&["ps", "-aq", "--filter", &format!("label=openshell.ai/sandbox-namespace={namespace}"), "--filter", &format!("label=openshell.ai/sandbox-name={}", handle.as_str()), "--filter", "label=openshell.ai/isolation-role=sandbox"]).await?;
        if container.len() != 12 || !container.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("expected exactly one owned Docker container; matching ID bytes={}", container.len()).into());
        }
        let memory_max: u64 = docker(&["exec", &container, "cat", "/sys/fs/cgroup/memory.max"]).await?.parse()?;
        let swap_max: u64 = docker(&["exec", &container, "cat", "/sys/fs/cgroup/memory.swap.max"]).await?.parse()?;
        // Never start the allocator unless independently observed kernel limits bound the fault.
        if memory_max != 128 * 1024 * 1024 || swap_max > 128 * 1024 * 1024 { return Err("cgroup memory/swap is not safely bounded".into()); }
        let before = oom_kills(&container).await?;
        let source = b"#include <stdlib.h>\n#include <stddef.h>\nint main(void) { const size_t n = 512u * 1024u * 1024u; volatile unsigned char *p = malloc(n); if (!p) return 42; for (size_t i = 0; i < n; i += 4096u) p[i] = 1; free((void *)p); return 43; }\n";
        backend.upload(&handle, "/sandbox/memory-pressure.c", Bytes::from_static(source)).await?;
        let compiled = backend.exec(&handle, &ExecRequest { argv: vec!["cc".into(), "-O0".into(), "-o".into(), "/sandbox/memory-pressure".into(), "/sandbox/memory-pressure.c".into()], timeout: Some(Duration::from_secs(30)), output_limit_bytes: Some(65536), ..Default::default() }).await?;
        if compiled.exit_code != 0 { return Err(format!("bounded allocator fixture did not compile: {}", String::from_utf8_lossy(&compiled.stderr)).into()); }
        let killed = backend.exec(&handle, &ExecRequest { argv: vec!["/sandbox/memory-pressure".into()], timeout: Some(Duration::from_secs(30)), output_limit_bytes: Some(65536), ..Default::default() }).await?;
        let after = oom_kills(&container).await?;
        if killed.exit_code == 0 || killed.exit_code == 42 || killed.exit_code == 43 || after <= before { return Err(format!("actual cgroup OOM not proved: exit={} before={before} after={after}", killed.exit_code).into()); }
        let recovered = backend.exec(&handle, &ExecRequest { argv: vec!["printf".into(), "memory-recovered".into()], timeout: Some(Duration::from_secs(10)), ..Default::default() }).await?;
        if recovered.exit_code != 0 || recovered.stdout.as_ref() != b"memory-recovered" { return Err("sandbox did not recover after the bounded OOM".into()); }
        println!("REAL_CGROUP_OOM memory_max={memory_max} swap_max={swap_max} allocator_max=536870912 exit_code={} oom_kill_before={before} oom_kill_after={after} subsequent_exec=true", killed.exit_code);
        Ok(())
    }.await;
    let cleanup = reaper
        .shutdown_all(&ShutdownConfig {
            overall: Duration::from_secs(90),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(20),
            ..Default::default()
        })
        .await;
    let unresolved = match &cleanup {
        Ok(report) => {
            report.deadline_exceeded
                || report.journal_error.is_some()
                || !reaper.leak_report().entries.is_empty()
        }
        Err(_) => true,
    };
    if unresolved {
        let retained = directory.keep();
        return Err(format!(
            "cleanup unresolved; retained journal {}: {cleanup:?}",
            retained.display()
        )
        .into());
    }
    operation?;
    let remaining = docker(&[
        "ps",
        "-aq",
        "--filter",
        &format!("label=openshell.ai/sandbox-namespace={namespace}"),
        "--filter",
        &format!("label=openshell.ai/sandbox-name={}", handle.as_str()),
    ])
    .await?;
    if !remaining.is_empty() {
        return Err("owned Docker container remains after cleanup".into());
    }
    println!("MEMORY_PRESSURE_CLEANUP remaining_resources=0 owned_container_absent=true");
    keepalive.shutdown().await?;
    Ok(())
}
