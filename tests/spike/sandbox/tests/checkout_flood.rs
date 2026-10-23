//! Live git checkout and watcher overflow recovery; no synthetic event injection.
use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};
use polaris_openshell::{
    client::{GatewayTls, OpenShellBackend, OpenShellConfig, PolicyCatalog},
    policy::{AgentRole, ExecTier, policy_for},
};
use polaris_platform::process::{
    FsEvent, RescanReason, WatchConfig, resolve_program, run_probe, watch,
};
use polaris_sdk::{
    ids::CommandId,
    registry::{ExecRequest, SandboxBackend, SandboxHandle, SandboxSpec},
};
use polaris_storage_sync::{
    ApplyOpts, HashMode, IgnoreRules, Manifest, SandboxSyncBackend, apply, plan, scan,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

type Failure = Box<dyn std::error::Error + Send + Sync>;
fn git(root: &Path, args: &[&str]) -> Result<(), Failure> {
    let hooks = root.join(".fixture-hooks");
    std::fs::create_dir_all(&hooks)?;
    let program = resolve_program(std::ffi::OsStr::new("git"), root)?;
    let mut argv = vec![
        "-c".to_owned(),
        format!("core.hooksPath={}", hooks.display()),
    ];
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));
    let output = run_probe(&program.path, &argv, root, Duration::from_secs(90), 65536)?;
    if output.exit_code != Some(0)
        || output.timed_out
        || output.stdout_truncated
        || output.stderr_truncated
    {
        return Err(format!(
            "fixture git failed: {args:?} {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}
fn fixture(root: &Path) -> Result<(), Failure> {
    git(root, &["init", "--quiet", "--initial-branch=main"])?;
    git(
        root,
        &["config", "user.name", "Polaris P2 isolated fixture"],
    )?;
    git(
        root,
        &["config", "user.email", "p2-fixture@example.invalid"],
    )?;
    git(root, &["config", "core.autocrlf", "false"])?;
    git(root, &["config", "core.longpaths", "true"])?;
    for index in 0..10_000_u32 {
        std::fs::write(
            root.join(format!("file-{index:05}.txt")),
            format!("branch-main {index}\r\n"),
        )?;
    }
    std::fs::write(
        root.join(".env"),
        b"SYNTHETIC_DENIED_FIXTURE=never_export\n",
    )?;
    std::fs::create_dir(root.join(".aws"))?;
    std::fs::write(root.join(".aws/credentials"), b"SYNTHETIC_DENIED_FIXTURE\n")?;
    git(root, &["add", "--all"])?;
    git(root, &["commit", "--quiet", "-m", "main fixture"])?;
    git(root, &["checkout", "--quiet", "-b", "variant"])?;
    for index in 0..10_000_u32 {
        std::fs::write(
            root.join(format!("file-{index:05}.txt")),
            format!("branch-variant {index}\r\n"),
        )?;
    }
    git(root, &["add", "--all"])?;
    git(root, &["commit", "--quiet", "-m", "variant fixture"])?;
    git(root, &["checkout", "--quiet", "main"])?;
    Ok(())
}
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

#[ignore = "requires real OpenShell, mTLS and immutable image containing sync-batch"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_checkout_overflow_rescan_preserves_manifest() -> Result<(), Failure> {
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
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("workspace");
    std::fs::create_dir(&root)?;
    let build_root = root.clone();
    tokio::task::spawn_blocking(move || fixture(&build_root)).await??;
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE")?;
    let mtls = std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR")
        .map(PathBuf::from)
        .ok_or("set POLARIS_OPEN_SHELL_MTLS_DIR")?;
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
    let handle = SandboxHandle::new(&format!("fl-{suffix}"))?;
    let reaper = Reaper::new(directory.path().join("run"))?;
    let registration = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label(handle.as_str().to_owned()),
        Remote {
            backend: Arc::clone(&backend),
            handle: handle.clone(),
        },
    )?;
    let operation = async {
        let spec = SandboxSpec {name:Some(handle.as_str().to_owned()), image:Some(image), policy:Some(policy), cpu_millis:Some(1000), memory_mib:Some(512)};
        let actual = match backend.create(&spec).await {
            Err(polaris_sdk::registry::SandboxError::AlreadyExists(error)) => {reaper.release(registration)?; return Err::<(), Failure>(format!("refused ownership: {error}").into());},
            result => result?,
        };
        if actual != handle {return Err("backend changed requested identity".into());}
        let initialized = backend.exec(&handle, &ExecRequest {argv:vec!["mkdir".into(), "-p".into(), "/sandbox/ws".into()], timeout:Some(Duration::from_secs(10)), ..Default::default()}).await?;
        if initialized.exit_code != 0 {return Err("workspace initialization failed".into());}
        let rules = IgnoreRules::from_workspace(&root)?;
        let initial = scan(&root, &rules, None, HashMode::Paranoid)?.manifest;
        if initial.entries.len() != 10_000 {return Err("fixture must contain exactly 10000 allowed files".into());}
        let watcher = watch(&root, WatchConfig {capacity:1, debounce:Duration::from_millis(5), ..Default::default()})?;
        let checkout_root = root.clone();
        let checkout = tokio::task::spawn_blocking(move || git(&checkout_root, &["checkout", "--quiet", "variant"]));
        let started = Instant::now();
        let first = apply(backend.as_ref(), &handle, &root, &initial, &plan(&initial, &Manifest::default()), &ApplyOpts {remote_root:"/sandbox/ws".into(), concurrency:8, cancellation:None}).await;
        checkout.await??;
        // Any first-pass result is superseded by a fresh full scan after actual event loss.
        let overflow_deadline = Instant::now() + Duration::from_secs(10);
        let mut overflow = false;
        while Instant::now() < overflow_deadline {
            match watcher.recv_timeout(Duration::from_millis(50))? {
                Some(FsEvent::RescanRequired {reason:RescanReason::QueueOverflow | RescanReason::BackendOverflow}) => {overflow=true; break;},
                Some(FsEvent::RescanRequired {..}) => return Err("backend error is not overflow evidence".into()),
                _ => {},
            }
        }
        if !overflow {return Err("checkout did not produce required overflow evidence".into());}
        let after = scan(&root, &rules, None, HashMode::Paranoid)?.manifest;
        if after.entries.len()!=10_000 || after.content_id()==initial.content_id() {return Err("checkout did not change the exact allowed tree".into());}
        let remote = SandboxSyncBackend::new(backend.as_ref(), &handle, "/sandbox/ws", 8)?.snapshot().await?;
        let repair = apply(backend.as_ref(), &handle, &root, &after, &plan(&after, &remote), &ApplyOpts {remote_root:"/sandbox/ws".into(), concurrency:8, cancellation:None}).await?;
        if repair.rescan_required || !repair.failed.is_empty() || !repair.changed.is_empty() || !repair.conflicts.is_empty() || !repair.cancelled.is_empty() {return Err(format!("repair incomplete: {repair:?}").into());}
        let final_remote = SandboxSyncBackend::new(backend.as_ref(), &handle, "/sandbox/ws", 8)?.snapshot().await?;
        if final_remote != after {return Err("post-overflow remote manifest differs".into());}
        for denied in ["/sandbox/ws/.env", "/sandbox/ws/.aws/credentials", "/sandbox/ws/.git/HEAD"] {
            let output = backend.exec(&handle, &ExecRequest {argv:vec!["test".into(), "-e".into(), denied.into()], timeout:Some(Duration::from_secs(10)), ..Default::default()}).await?;
            if output.exit_code != 1 {return Err(format!("denied path present or check failed: {denied}").into());}
        }
        let first_pass = match &first {
            Ok(report) => format!("uploaded={} failed={} conflicts={} changed={} rescan_required={}", report.uploaded.len(), report.failed.len(), report.conflicts.len(), report.changed.len(), report.rescan_required),
            Err(error) => format!("error={error}"),
        };
        println!("REAL_CHECKOUT_FLOOD files=10000 overflow=true full_rescan=true manifest_equal=true deny_paths_absent=true elapsed_ms={} first_pass={first_pass}", started.elapsed().as_millis());
        drop(watcher);
        Ok(())
    }.await;
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
            "cleanup unresolved; retained journal at {}: {shutdown:?}",
            retained.display()
        )
        .into());
    }
    if let Some(owner) = keepalive {
        owner.shutdown().await?;
    }
    operation
}
