//! Non-UI P1 application behavior.
use clap::{Parser, Subcommand};
use polaris_kernel::{Kernel, LocalKernelConfig};
use polaris_sdk::{
    ids::{FrameId, Seq, SessionId},
    registry::{EventSource, KernelApi},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// Validated arguments for local P1 operations.
#[derive(Debug, Parser)]
#[command(name = "polaris", version, about = "Polaris local kernel controls")]
pub struct Args {
    /// Durable state root.
    #[arg(long, default_value = ".polaris/state")]
    pub state_dir: PathBuf,
    /// Session workspace.
    #[arg(long, default_value = ".")]
    pub workspace: PathBuf,
    /// Recover this session; otherwise create a new session.
    #[arg(long)]
    pub session: Option<SessionId>,
    /// Operation to perform.
    #[command(subcommand)]
    pub command: Action,
}
/// Non-UI operations available in the local profile.
#[derive(Debug, Subcommand)]
pub enum Action {
    /// Inspect every setup item and show concrete onboarding actions; installs nothing.
    #[command(alias = "/onboarding")]
    Onboarding {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Review and explicitly approve actual sandbox test execution.
    Sandbox(crate::SandboxArgs),
    /// Filter piped compiler diagnostics and reconcile the real issue ledger.
    Rtk(crate::RtkArgs),
    /// Commit one $directive and emit real session events as JSONL.
    Run {
        /// Exactly one session directive.
        task: String,
    },
    /// Inspect recovery resources and configured credential references.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}
/// Application errors never contain bearer credentials.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Compiler diagnostic pipeline failure.
    #[error(transparent)]
    Rtk(#[from] crate::RtkCliError),
    /// Configuration rejection.
    #[error(transparent)]
    Config(#[from] polaris_config::ConfigError),
    /// Filesystem, console or signal error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Kernel storage/lifecycle failure.
    #[error(transparent)]
    Kernel(#[from] polaris_kernel::KernelError),
    /// Local control rejection.
    #[error(transparent)]
    Control(#[from] polaris_sdk::registry::KernelError),
    /// Logging failure.
    #[error(transparent)]
    Log(#[from] polaris_log::LogError),
    /// JSON encoding failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Unsupported operation or startup invariant failure.
    #[error("{0}")]
    Unavailable(&'static str),
}
/// Install aws-lc-rs before any network clients are constructed.
/// # Errors
/// A provider was already installed; its provenance cannot be assumed.
pub fn install_crypto_provider() -> Result<(), CliError> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| CliError::Unavailable("TLS crypto provider was already installed"))
}
/// Load config, start redacted logs, execute the command and flush accepted logs.
/// # Errors
/// Configuration, command, storage, signal, encoding or cleanup failure.
pub async fn run(args: Args) -> Result<(), CliError> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        return Err(CliError::Unavailable(
            "install the TLS provider before startup",
        ));
    }
    let dirs = polaris_config::discover(&args.workspace)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )?;
    let loaded = polaris_config::load(&layers, &polaris_config::TrustGrant::none())?;
    let redactor = polaris_security::automata::DfaRedactor::default_redactor()
        .map_err(|_| CliError::Unavailable("log redactor could not compile"))?;
    let mut log_config = polaris_log::LogConfig::new(args.state_dir.join("logs"));
    log_config.tui_mode = true;
    log_config.level = match loaded.config.log.level {
        polaris_config::LogLevel::Error => "error",
        polaris_config::LogLevel::Warn => "warn",
        polaris_config::LogLevel::Info => "info",
        polaris_config::LogLevel::Debug => "debug",
        polaris_config::LogLevel::Trace => "trace",
    }
    .to_owned();
    let guard = Arc::new(polaris_log::init(log_config, redactor)?);
    let weak_log = Arc::downgrade(&guard);
    // Process-owned cold-path hook. The default hook may expose secrets in panic payloads.
    std::panic::set_hook(Box::new(move |_| {
        tracing::error!("panic triggered; local owners stop through unwind");
        if let Some(log) = weak_log.upgrade() {
            let _flushed = log.flush(Duration::from_secs(2));
        }
    }));
    let result = match args.command {
        Action::Sandbox(sandbox) => {
            crate::sandbox::run(sandbox, &args.state_dir, &args.workspace, args.session).await
        }
        Action::Rtk(mut rtk) => {
            rtk.state_dir = args.state_dir;
            rtk.workspace = args.workspace;
            tokio::task::spawn_blocking(move || crate::rtk::run(rtk))
                .await
                .map_err(|_| CliError::Unavailable("RTK owner failed"))?
                .map_err(CliError::from)
        }
        Action::Doctor { json } | Action::Onboarding { json } => {
            doctor(&args.state_dir, &args.workspace, &loaded.config, json).await
        }
        Action::Run { task } => {
            headless(&args.state_dir, &args.workspace, args.session, &task).await
        }
    };
    let log = Arc::clone(&guard);
    let flushed = tokio::task::spawn_blocking(move || log.flush(Duration::from_secs(2)))
        .await
        .map_err(|_| CliError::Unavailable("log flush owner failed"))?;
    if !flushed {
        return Err(CliError::Unavailable("log flush deadline exceeded"));
    }
    if guard.stats().sink_errors != 0 {
        return Err(CliError::Unavailable("log sink reported an I/O failure"));
    }
    result
}
async fn doctor(
    root: &Path,
    workspace: &Path,
    config: &polaris_config::PolarisConfig,
    json: bool,
) -> Result<(), CliError> {
    let run = root.join("run");
    let leaks = tokio::task::spawn_blocking(move || {
        polaris_kernel::reaper::Reaper::new(run).map(|reaper| reaper.leak_report())
    })
    .await
    .map_err(|_| CliError::Unavailable("journal inspection owner failed"))?
    .map_err(|_| CliError::Unavailable("recovery journal cannot be inspected"))?;
    let unresolved: Vec<_> = leaks.entries.iter().map(|entry| serde_json::json!({"kind":format!("{:?}",entry.kind),"locator":format!("{:?}",entry.locator)})).collect();
    let setup = crate::doctor::collect(workspace, config).await?;
    let credential_presence = setup["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == "win-keyring"))
        .map(|item| item["observation"].clone());
    let report = serde_json::json!({"crypto_provider":"aws-lc-rs","configuration":"PASS","state_root":root,"recovery_orphans":unresolved,"credential_presence":credential_presence,"model_execution":"P3 execution profile required","setup":setup});
    let mut out = std::io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &report)?;
        writeln!(out)?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
    }
    out.flush()?;
    if !leaks.entries.is_empty() {
        return Err(CliError::Unavailable(
            "recovery journal contains unresolved resources",
        ));
    }
    Ok(())
}
async fn headless(
    root: &Path,
    workspace: &Path,
    session: Option<SessionId>,
    task: &str,
) -> Result<(), CliError> {
    if !task.trim_start().starts_with('$') {
        return Err(CliError::Unavailable(
            "P1 run accepts one local $directive; model task execution requires the P3 execution profile",
        ));
    }
    let session = match session {
        Some(id) => id,
        None => SessionId::new_v7(),
    };
    let handle = Kernel::start(LocalKernelConfig::new(
        root,
        workspace,
        session,
        FrameId::new_v7(),
    ))
    .await?;
    let result = tokio::select! {
        result = handle.apply_directive(task) => result.map(|_| ()).map_err(CliError::from),
        signal = shutdown_signal() => match signal { Ok(()) => Err(CliError::Unavailable("interrupted")), Err(error) => Err(CliError::Io(error)) },
    };
    let report = handle.shutdown().await?;
    if report.deadline_exceeded
        || report.journal_error.is_some()
        || report.entries.iter().any(|entry| {
            !matches!(
                entry.outcome,
                polaris_kernel::reaper::Outcome::Released | polaris_kernel::reaper::Outcome::Forced
            )
        })
    {
        return Err(CliError::Unavailable("shutdown left unresolved resources"));
    }
    result?;
    let mut events = handle.subscribe(Seq::new(1)).await?;
    while let Some(record) = events.next_event().await {
        if record.session() != session {
            continue;
        }
        let mut out = std::io::stdout().lock();
        serde_json::to_writer(&mut out, &record)?;
        writeln!(out)?;
    }
    Ok(())
}
#[cfg(windows)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    let mut close = tokio::signal::windows::ctrl_close()?;
    let mut logoff = tokio::signal::windows::ctrl_logoff()?;
    let mut shutdown = tokio::signal::windows::ctrl_shutdown()?;
    tokio::select! { _ = ctrl_c.recv() => {}, _ = ctrl_break.recv() => {}, _ = close.recv() => {}, _ = logoff.recv() => {}, _ = shutdown.recv() => {} }
    Ok(())
}
#[cfg(unix)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = terminate.recv() => {} }
    Ok(())
}
