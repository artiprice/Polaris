//! Live S0-7 batch and bounded large-file sync contract.
use std::error::Error;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};
use polaris_openshell::{
    client::{GatewayTls, OpenShellBackend, OpenShellConfig, PolicyCatalog},
    policy::{AgentRole, ExecTier, policy_for},
};
use polaris_sdk::{
    ids::CommandId,
    registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use polaris_storage_sync::{
    ApplyOpts, HashMode, IgnoreRules, Manifest, RelPath, SandboxSyncBackend, apply, plan, scan,
};

type Failure = Box<dyn Error + Send + Sync>;

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

#[ignore = "requires the immutable P2 OpenShell image, local gateway and mTLS certificates"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_s0_7_large_incremental_and_noop_sync() -> Result<(), Failure> {
    const LARGE_BYTES: u64 = 61 * 1024 * 1024;
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
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE")?;
    let mtls = std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls",
            )
        });
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".to_owned(),
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
        .take(12)
        .collect();
    let handle = SandboxHandle::new(&format!("p2sync-{suffix}"))?;
    if handle.as_str().len() != 19 {
        return Err("sandbox name does not meet the 19-character ownership contract".into());
    }
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
    let root = directory.path().join("workspace");
    std::fs::create_dir(&root)?;
    let mut large = std::fs::File::create(root.join("large.bin"))?;
    let block = vec![0x53_u8; 64 * 1024];
    let mut left = LARGE_BYTES;
    while left > 0 {
        let count = usize::try_from(left.min(block.len() as u64))?;
        large.write_all(&block[..count])?;
        left -= u64::try_from(count)?;
    }
    large.sync_all()?;
    std::fs::write(root.join("crlf.txt"), b"first\r\nsecond\r\n")?;
    std::fs::create_dir(root.join("MixedCase"))?;
    std::fs::write(root.join("MixedCase/ReadMe.TXT"), b"case-preserved\n")?;
    std::fs::write(root.join("run.sh"), b"#!/bin/sh\necho exact\n")?;
    std::fs::write(root.join(".env"), b"P2_DENY_TEST=must-not-upload\n")?;
    std::fs::create_dir(root.join(".aws"))?;
    std::fs::write(root.join(".aws/credentials"), b"P2_DENY_TEST=never\n")?;

    let operation = async {
        let spec = SandboxSpec {
            name: Some(handle.as_str().to_owned()),
            image: Some(image),
            policy: Some(policy),
            cpu_millis: Some(1_000),
            memory_mib: Some(512),
        };
        let actual = match backend.create(&spec).await {
            Err(SandboxError::AlreadyExists(error)) => {
                reaper.release(registration)?;
                return Err::<(), Failure>(format!("refused ownership collision: {error}").into());
            }
            result => result?,
        };
        if actual != handle {
            return Err("backend changed the requested sandbox identity".into());
        }
        match backend.create(&spec).await {
            Err(SandboxError::AlreadyExists(_)) => {}
            Err(error) => return Err(format!("duplicate create was not typed AlreadyExists: {error}").into()),
            Ok(_) => return Err("duplicate create unexpectedly succeeded".into()),
        }
        let initialized = backend
            .exec(
                &handle,
                &ExecRequest {
                    argv: vec!["mkdir".into(), "-p".into(), format!("/sandbox/{}", handle.as_str())],
                    timeout: Some(Duration::from_secs(10)),
                    ..Default::default()
                },
            )
            .await?;
        if initialized.exit_code != 0 {
            return Err("workspace mkdir failed".into());
        }
        let remote_root = format!("/sandbox/{}", handle.as_str());
        let opts = ApplyOpts {
            remote_root: remote_root.clone(),
            concurrency: 1,
            cancellation: None,
        };
        let rules = IgnoreRules::from_workspace(&root)?;
        let initial_scan = scan(&root, &rules, None, HashMode::Paranoid)?;
        let initial = initial_scan.manifest;
        if initial.entries.len() != 4 || initial.entries.get(&RelPath::new("large.bin")?).map(|entry| entry.size) != Some(LARGE_BYTES) {
            return Err(format!("unexpected initial manifest: {} entries", initial.entries.len()).into());
        }
        if !initial_scan.denied.iter().any(|path| path.as_str() == ".env")
            || !initial_scan.denied.iter().any(|path| path.as_str() == ".aws")
        {
            return Err("secret deny-list did not withhold .env and .aws".into());
        }
        let first_started = Instant::now();
        let first = apply(
            backend.as_ref(),
            &handle,
            &root,
            &initial,
            &plan(&initial, &Manifest::default()),
            &opts,
        )
        .await?;
        let initial_apply_elapsed = first_started.elapsed();
        if first.uploaded.len() != 4 || !first.failed.is_empty() || first.rescan_required {
            return Err(format!("initial sync incomplete: {first:?}").into());
        }
        let first_remote = SandboxSyncBackend::new(backend.as_ref(), &handle, &remote_root, 4)?
            .snapshot()
            .await?;
        if first_remote != initial {
            return Err("initial remote snapshot differs from local manifest".into());
        }
        let crlf = backend.download(&handle, &format!("{remote_root}/crlf.txt")).await?;
        if crlf.as_ref() != b"first\r\nsecond\r\n" {
            return Err("CRLF bytes changed during transport".into());
        }
        if !first_remote.entries.get(&RelPath::new("run.sh")?).is_some_and(|entry| entry.executable) {
            return Err("executable mode was not preserved".into());
        }
        for denied in [".env", ".aws/credentials"] {
            let output = backend
                .exec(
                    &handle,
                    &ExecRequest {
                        argv: vec!["test".into(), "-e".into(), format!("{remote_root}/{denied}")],
                        timeout: Some(Duration::from_secs(10)),
                        ..Default::default()
                    },
                )
                .await?;
            if output.exit_code != 1 {
                return Err(format!("denied remote path exists: {denied}").into());
            }
        }

        let mut large = std::fs::OpenOptions::new().write(true).open(root.join("large.bin"))?;
        large.seek(SeekFrom::Start(4096))?;
        large.write_all(b"INCREMENTAL-CHANGE")?;
        large.sync_all()?;
        std::fs::remove_file(root.join("MixedCase/ReadMe.TXT"))?;
        std::fs::write(root.join("new.txt"), b"incremental add\n")?;
        let second_scan = scan(&root, &rules, Some(&initial_scan.cache), HashMode::Paranoid)?;
        let second = second_scan.manifest;
        let incremental = plan(&second, &first_remote);
        if incremental.upload.len() != 2 || incremental.delete.len() != 1 {
            return Err(format!("unexpected incremental plan: {incremental:?}").into());
        }
        let second_started = Instant::now();
        let second_report = apply(backend.as_ref(), &handle, &root, &second, &incremental, &opts).await?;
        let incremental_apply_elapsed = second_started.elapsed();
        if second_report.uploaded.len() != 2 || second_report.deleted.len() != 1 || !second_report.failed.is_empty() || second_report.rescan_required {
            return Err(format!("incremental sync incomplete: {second_report:?}").into());
        }
        let second_remote = SandboxSyncBackend::new(backend.as_ref(), &handle, &remote_root, 4)?
            .snapshot()
            .await?;
        if second_remote != second {
            return Err("incremental remote snapshot differs from local manifest".into());
        }
        let noop_started = Instant::now();
        let noop_scan = scan(&root, &rules, Some(&second_scan.cache), HashMode::Fast)?;
        let noop_local_scan_elapsed = noop_started.elapsed();
        let noop_remote_started = Instant::now();
        let noop_remote = SandboxSyncBackend::new(backend.as_ref(), &handle, &remote_root, 4)?
            .snapshot().await?;
        let noop_remote_elapsed = noop_remote_started.elapsed();
        if noop_remote != second_remote { return Err("no-op remote manifest changed".into()); }
        let noop_plan = plan(&noop_scan.manifest, &noop_remote);
        if !noop_plan.is_empty() {
            return Err(format!("no-op produced mutations: {noop_plan:?}").into());
        }
        let noop_report = apply(backend.as_ref(), &handle, &root, &noop_scan.manifest, &noop_plan, &opts).await?;
        let noop_elapsed = noop_started.elapsed();
        if noop_elapsed >= Duration::from_secs(5) || !noop_report.uploaded.is_empty() || !noop_report.deleted.is_empty() {
            return Err(format!("no-op contract missed: elapsed={noop_elapsed:?}, report={noop_report:?}").into());
        }
        println!(
            "REAL_S0_7 large_bytes={LARGE_BYTES} files=4 initial_uploaded={} initial_apply_ms={} incremental_upload={} incremental_delete={} incremental_apply_ms={} noop_roundtrip_ms={} noop_local_scan_ms={} noop_remote_snapshot_ms={} content_ids_match=true crlf=true executable=true deny=true",
            first.uploaded.len(), initial_apply_elapsed.as_millis(), second_report.uploaded.len(), second_report.deleted.len(), incremental_apply_elapsed.as_millis(), noop_elapsed.as_millis(), noop_local_scan_elapsed.as_millis(), noop_remote_elapsed.as_millis()
        );
        Ok(())
    }
    .await;
    let shutdown = reaper
        .shutdown_all(&polaris_kernel_reaper::ShutdownConfig {
            overall: Duration::from_secs(90),
            graceful: Duration::from_secs(60),
            forced: Duration::from_secs(20),
            ..Default::default()
        })
        .await;
    let shutdown = match shutdown {
        Ok(shutdown) => shutdown,
        Err(error) => {
            let retained = directory.keep();
            return Err(format!(
                "sandbox shutdown failed; retained journal at {}: {error}",
                retained.display()
            )
            .into());
        }
    };
    if shutdown.deadline_exceeded
        || shutdown.journal_error.is_some()
        || !reaper.leak_report().entries.is_empty()
    {
        let retained = directory.keep();
        return Err(format!(
            "sandbox cleanup unresolved; retained journal at {}: {shutdown:?}",
            retained.display()
        )
        .into());
    }
    if let Some(owner) = keepalive {
        owner.shutdown().await?;
    }
    operation
}
