//! Bounded resident-memory pressure in the reference VM, without exhausting the host.
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
use polaris_spike_sandbox::{BurstLimits, MemoryInfo, run_sandboxes};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{process::Command, time::Instant};

type Failure = Box<dyn std::error::Error + Send + Sync>;
const MIB: u64 = 1024 * 1024;
const RESIDENT_MIN: u64 = 1400 * MIB;

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

async fn wsl(arguments: &[&str]) -> Result<String, Failure> {
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new("wsl.exe")
            .args(["-d", "Ubuntu-24.04", "--exec"])
            .args(arguments)
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if !output.status.success() {
        return Err(format!(
            "independent WSL probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
async fn memory() -> Result<MemoryInfo, Failure> {
    Ok(MemoryInfo::parse(&wsl(&["cat", "/proc/meminfo"]).await?)?)
}
async fn docker(arguments: &[&str]) -> Result<String, Failure> {
    let mut argv = vec!["docker"];
    argv.extend_from_slice(arguments);
    wsl(&argv).await
}
fn counter(text: &str, key: &str) -> Result<u64, Failure> {
    let mut result = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some(key) {
            if result.is_some() {
                return Err(format!("duplicate cgroup counter {key}").into());
            }
            result = Some(fields.next().ok_or("missing cgroup value")?.parse()?);
            if fields.next().is_some() {
                return Err("extra cgroup counter fields".into());
            }
        }
    }
    result.ok_or_else(|| format!("missing cgroup counter {key}").into())
}
async fn anonymous_bytes(container: &str) -> Result<u64, Failure> {
    counter(
        &docker(&["exec", container, "cat", "/sys/fs/cgroup/memory.stat"]).await?,
        "anon",
    )
}
async fn oom_kills(container: &str) -> Result<u64, Failure> {
    counter(
        &docker(&["exec", container, "cat", "/sys/fs/cgroup/memory.events"]).await?,
        "oom_kill",
    )
}

// One fixed allocation, every page touched. Release is cooperative and has a hard one-minute
// fallback even if the client disappears; destroying the owned sandbox is the final boundary.
const ALLOCATOR: &[u8] = br#"#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
int main(void) {
    const size_t n = 1536u * 1024u * 1024u;
    volatile unsigned char *p = malloc(n);
    if (!p) return 42;
    for (size_t i = 0; i < n; i += 4096u) p[i] = 1;
    FILE *ready = fopen("/sandbox/pressure.ready", "wb");
    if (!ready) { free((void *)p); return 43; }
    if (fputs("resident", ready) < 0 || fclose(ready) != 0) { free((void *)p); return 44; }
    for (unsigned int tick = 0; tick < 240u; ++tick) {
        if (access("/sandbox/pressure.release", F_OK) == 0) {
            free((void *)p);
            return 0;
        }
        usleep(250000u);
    }
    free((void *)p);
    return 45;
}
"#;

#[ignore = "requires an explicitly supplied private fault gateway and immutable P2 image"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sandbox_lifecycle_survives_bounded_wsl_resident_pressure() -> Result<(), Failure> {
    if !cfg!(windows) {
        return Err("this reference workload uses Windows-owned WSL".into());
    }
    let wsl_owner = polaris_platform::wsl::Wsl::locate()?;
    let distro = polaris_platform::wsl::DistroName::new("Ubuntu-24.04")?;
    let keepalive = wsl_owner.keepalive(&distro).await?;
    let baseline = memory().await?;
    if !baseline.within_six_gib_reference() || baseline.available_kib < 2304 * 1024 {
        return Err(format!(
            "refusing allocation: six-GiB VM with 2304 MiB available required: {baseline:?}"
        )
        .into());
    }
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
    let namespace_filter = format!("label=openshell.ai/sandbox-namespace={namespace}");
    // Require an empty private namespace so a final empty-scope assertion cannot erase or
    // misattribute another test's resources. This test never deletes resources by this filter.
    if !docker(&["ps", "-aq", "--filter", &namespace_filter])
        .await?
        .is_empty()
    {
        return Err("private namespace must be idle before resident-pressure workload".into());
    }
    let mtls =
        PathBuf::from(std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR").ok_or("mTLS missing")?);
    let mut config = OpenShellConfig::wsl2(GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    });
    config.endpoint = "https://localhost:17771".into();
    let backend = Arc::new(OpenShellBackend::connect(config).await?);
    let policy = backend.register_policy(&policy_for(AgentRole::Reader, ExecTier::Docker)?)?;
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE")?;
    let suffix: String = CommandId::new_v7()
        .as_uuid()
        .simple()
        .to_string()
        .chars()
        .rev()
        .take(16)
        .collect();
    let handle = SandboxHandle::new(&format!("rp-{suffix}"))?;
    let directory = tempfile::tempdir()?;
    let reaper = Arc::new(Reaper::new(directory.path().join("run"))?);
    let registration = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label(handle.as_str().to_owned()),
        Remote {
            backend: Arc::clone(&backend),
            handle: handle.clone(),
        },
    )?;
    let mut allocator_task = None;
    let operation: Result<(), Failure> = async {
        let spec = SandboxSpec { name: Some(handle.as_str().to_owned()), image: Some(image.clone()), policy: Some(policy), cpu_millis: Some(500), memory_mib: Some(2048) };
        match backend.create(&spec).await {
            Err(SandboxError::AlreadyExists(detail)) => { reaper.release(registration)?; return Err(format!("refused ownership collision: {detail}").into()); }
            Ok(actual) if actual == handle => (),
            Ok(actual) => { let _registration = reaper.register_journaled(ResourceKind::Sandbox, Locator::Label(actual.as_str().to_owned()), Remote { backend: Arc::clone(&backend), handle: actual })?; return Err("backend changed requested identity".into()); }
            Err(error) => return Err(error.into()),
        }
        let name_filter = format!("label=openshell.ai/sandbox-name={}", handle.as_str());
        let container = docker(&["ps", "-aq", "--no-trunc", "--filter", &namespace_filter, "--filter", &name_filter, "--filter", "label=openshell.ai/isolation-role=sandbox"]).await?;
        if container.len() != 64 || !container.bytes().all(|byte| byte.is_ascii_hexdigit()) { return Err("expected one exact owned workload container".into()); }
        let memory_max: u64 = docker(&["exec", &container, "cat", "/sys/fs/cgroup/memory.max"]).await?.parse()?;
        let swap_max: u64 = docker(&["exec", &container, "cat", "/sys/fs/cgroup/memory.swap.max"]).await?.parse()?;
        if memory_max != 2048 * MIB || swap_max > 2048 * MIB { return Err("independent cgroup bounds differ; allocator refused".into()); }
        let oom_before = oom_kills(&container).await?;
        backend.upload(&handle, "/sandbox/resident-pressure.c", Bytes::from_static(ALLOCATOR)).await?;
        let compiled = backend.exec(&handle, &ExecRequest { argv: vec!["cc".into(), "-O0".into(), "-o".into(), "/sandbox/resident-pressure".into(), "/sandbox/resident-pressure.c".into()], timeout: Some(Duration::from_secs(30)), output_limit_bytes: Some(65536), ..Default::default() }).await?;
        if compiled.exit_code != 0 { return Err(format!("resident allocator compile failed: {}", String::from_utf8_lossy(&compiled.stderr)).into()); }
        let allocator_backend = Arc::clone(&backend);
        let allocator_handle = handle.clone();
        allocator_task = Some(tokio::spawn(async move {
            allocator_backend.exec(&allocator_handle, &ExecRequest { argv: vec!["/sandbox/resident-pressure".into()], timeout: Some(Duration::from_secs(75)), output_limit_bytes: Some(65536), ..Default::default() }).await
        }));
        let ready_deadline = Instant::now() + Duration::from_secs(25);
        let resident_before = loop {
            let resident = anonymous_bytes(&container).await?;
            if resident >= RESIDENT_MIN { break resident; }
            if Instant::now() >= ready_deadline { return Err(format!("allocator did not reach resident threshold: {resident}").into()); }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        let during = memory().await?;
        if during.total_kib != baseline.total_kib { return Err("VM memory cap changed during workload".into()); }
        let report = run_sandboxes(Arc::clone(&backend), Arc::clone(&reaper),
            SandboxSpec { image: Some(image), policy: Some(policy), cpu_millis: Some(250), memory_mib: Some(128), ..Default::default() },
            BurstLimits { concurrent: 4, cycles: 1, deadline: Duration::from_secs(45) },
            ExecRequest { argv: vec!["printf".into(), "pressure-lifecycle".into()], timeout: Some(Duration::from_secs(10)), output_limit_bytes: Some(4096), ..Default::default() }).await?;
        let resident_after = anonymous_bytes(&container).await?;
        if resident_after < RESIDENT_MIN || report.created != 4 || report.executed != 4 || report.destroyed != 4 || !report.failures.is_empty() || report.remaining_resources != 1 {
            return Err(format!("lifecycle progress under maintained pressure failed: resident={resident_after}, {report:?}").into());
        }
        backend.upload(&handle, "/sandbox/pressure.release", Bytes::from_static(b"release")).await?;
        let task = allocator_task.take().ok_or("missing allocator owner")?;
        let result = task.await??;
        if result.exit_code != 0 || oom_kills(&container).await? != oom_before { return Err("allocator failed, exhausted cgroup or exceeded cooperative deadline".into()); }
        let resident_released = anonymous_bytes(&container).await?;
        if resident_released >= RESIDENT_MIN { return Err("anonymous allocation was not released".into()); }
        println!("REAL_WSL_RESIDENT_PRESSURE mem_total_kib={} baseline_available_kib={} during_available_kib={} allocator_bytes=1610612736 resident_before={} resident_after={} resident_released={} lifecycle_created=4 executed=4 destroyed=4 oom_delta=0", baseline.total_kib, baseline.available_kib, during.available_kib, resident_before, resident_after, resident_released);
        Ok(())
    }.await;
    if let Some(task) = allocator_task {
        task.abort();
        let _joined = task.await;
    }
    let cleanup = reaper
        .shutdown_all(&ShutdownConfig {
            overall: Duration::from_secs(120),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(30),
            ..Default::default()
        })
        .await;
    let clean = matches!(&cleanup, Ok(report) if !report.deadline_exceeded && report.journal_error.is_none() && reaper.leak_report().entries.is_empty());
    if !clean {
        let retained = directory.keep();
        return Err(format!(
            "unresolved pressure cleanup; journal retained at {}: {cleanup:?}",
            retained.display()
        )
        .into());
    }
    operation?;
    for kind in ["container", "volume", "network"] {
        let arguments = if kind == "container" {
            vec!["ps", "-aq", "--filter", &namespace_filter]
        } else {
            vec![kind, "ls", "-q", "--filter", &namespace_filter]
        };
        if !docker(&arguments).await?.is_empty() {
            return Err(format!("private namespace retains {kind} resources").into());
        }
    }
    println!("WSL_RESIDENT_PRESSURE_CLEANUP journal=0 containers=0 volumes=0 networks=0");
    keepalive.shutdown().await?;
    Ok(())
}
