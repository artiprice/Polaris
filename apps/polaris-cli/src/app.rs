//! CLI dispatch, preserving headless controls alongside the interactive UI.
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
    pub command: Option<Action>,
    /// Render a deterministic plain-text screen without starting services.
    #[arg(long)]
    pub dump: Option<crate::tui::DumpSize>,
    /// Explicit fixture for --dump, or an interactive scenario in --demo.
    #[arg(long)]
    pub scenario: Option<String>,
    /// Run the isolated interactive UI demo without a kernel or model provider.
    #[arg(long, conflicts_with = "session")]
    pub demo: bool,
    /// Initial unsent draft.
    pub prompt: Option<String>,
    /// Color policy.
    #[arg(long, value_enum)]
    pub color: Option<crate::tui::ColorPolicy>,
    /// Color depth; terminal support must be verified independently.
    #[arg(long, value_enum)]
    pub color_depth: Option<crate::tui::ColorDepth>,
    /// Use inherited terminal colors rather than the dark application palette.
    #[arg(long, value_enum)]
    pub theme: Option<crate::tui::ThemeMode>,
    /// Use ASCII application decoration; preserve user Unicode text.
    #[arg(long)]
    pub ascii: bool,
    /// Override a saved ASCII preference and use Unicode decoration.
    #[arg(long, conflicts_with = "ascii")]
    pub no_ascii: bool,
    /// Disable cosmetic motion.
    #[arg(long)]
    pub reduce_motion: bool,
    /// Override a saved reduced-motion preference for this invocation.
    #[arg(long, conflicts_with = "reduce_motion")]
    pub no_reduce_motion: bool,
    /// Create a new diagnostic trace containing categories only, never text.
    #[arg(long)]
    pub key_trace: Option<PathBuf>,
    /// Read `env:NAME` credential references from this `.env` file before the process environment.
    #[arg(long, global = true)]
    pub env_file: Option<PathBuf>,
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
    /// Inspect configured skills against the current local builtin tool catalog.
    Status {
        /// Emit compact machine-readable JSON.
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
    /// Execute one bounded model task. Without --profile the provider chain selects the model
    /// (Token Factory, OpenRouter, local server) and System One runs in degraded mode.
    Task {
        /// Explicit trusted local llama.cpp and System-One profile TOML.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// One bounded task goal.
        task: String,
    },
    /// Profile the workspace's NVIDIA stack, target architectures and local GPUs (`/nv-profile`).
    NvProfile,
    /// Run the CUDA advisor over the workspace's `.cu`/`.cuh` sources (`/nv-verify`).
    NvVerify,
    /// Serve the local web UI on loopback and print its address (with this run's access token).
    Web {
        /// Loopback port; 0 picks a free one.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Attach to the `polarisd` listening here (a `polaris-*` pipe name or a socket path);
        /// `--session` names the daemon's session.
        #[arg(long, requires = "token_file")]
        connect: Option<String>,
        /// Credential file written by `polarisd --client-token-out`.
        #[arg(long, requires = "connect")]
        token_file: Option<PathBuf>,
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
    /// A validated UI, rendering or adapter operation failed.
    #[error("{0}")]
    Ui(String),
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
    /// Local task entry failure with a sanitized static diagnostic.
    #[error(transparent)]
    Task(#[from] crate::task::TaskError),
    /// Execution ended, but independent current-tree proof was not established.
    #[error("task execution finished without independent completion proof")]
    TaskUnverified,
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
    crate::tui::validate_invocation(&args)?;
    if args.dump.is_some() {
        return crate::tui::dump(&args);
    }
    let dirs = polaris_config::discover(&args.workspace)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )?;
    let loaded = polaris_config::load(&layers, &polaris_config::TrustGrant::none())?;
    if args.demo {
        // No log sink or durable owner is initialized for the presentation demo.
        return crate::tui::run(&args, &loaded, &dirs).await;
    }
    if args.session.is_some() && matches!(args.command.as_ref(), Some(Action::Task { .. })) {
        return Err(CliError::Unavailable(
            "task creates a new local session and cannot resume an existing session",
        ));
    }
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
    let result = if args.command.is_none() {
        crate::tui::run(&args, &loaded, &dirs).await
    } else {
        match args.command {
            None => Err(CliError::Unavailable("missing command branch")),
            Some(Action::Sandbox(sandbox)) => {
                crate::sandbox::run(sandbox, &args.state_dir, &args.workspace, args.session).await
            }
            Some(Action::Rtk(mut rtk)) => {
                rtk.state_dir = args.state_dir;
                rtk.workspace = args.workspace;
                tokio::task::spawn_blocking(move || crate::rtk::run(rtk))
                    .await
                    .map_err(|_| CliError::Unavailable("RTK owner failed"))?
                    .map_err(CliError::from)
            }
            Some(Action::Doctor { json }) | Some(Action::Onboarding { json }) => {
                doctor(&args.state_dir, &args.workspace, &loaded.config, json).await
            }
            Some(Action::Status { json }) => status(&dirs, json),
            Some(Action::NvProfile) | Some(Action::NvVerify) => {
                let command = if matches!(args.command, Some(Action::NvProfile)) {
                    polaris_ui_session::nvidia::NvidiaCommand::Profile
                } else {
                    polaris_ui_session::nvidia::NvidiaCommand::Verify
                };
                let lines = command
                    .run_with(
                        polaris_kernel::directives::directive::NvidiaMode::Auto,
                        &args.workspace,
                    )
                    .await
                    .map_err(|error| CliError::Ui(error.to_string()))?;
                let mut out = std::io::stdout().lock();
                writeln!(out, "{}", command.title())?;
                for line in lines {
                    writeln!(out, "{line}")?;
                }
                out.flush()?;
                Ok(())
            }
            Some(Action::Web {
                port,
                connect,
                token_file,
            }) => {
                let attach = match (connect, token_file) {
                    (Some(endpoint), Some(token_file)) => {
                        let session = args.session.ok_or(CliError::Unavailable(
                            "--connect needs --session with the daemon's session id",
                        ))?;
                        let credential = std::fs::read_to_string(token_file)?;
                        Some(
                            polaris_ui_web::AttachTarget::new(
                                &endpoint,
                                &session.to_string(),
                                &credential,
                            )
                            .map_err(|error| CliError::Ui(error.to_string()))?,
                        )
                    }
                    _ => None,
                };
                let attached = attach.as_ref().map(|target| target.endpoint.clone());
                let server = polaris_ui_web::WebServer::bind_with(
                    polaris_ui_web::WebOptions {
                        bind: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                        state_dir: args.state_dir.clone(),
                        workspace: args.workspace.clone(),
                        config: loaded.config.clone(),
                        env_file: polaris_ui_session::default_env_file(
                            args.env_file.as_deref(),
                            &dirs.global,
                        ),
                    },
                    None,
                    attach,
                )
                .await
                .map_err(|error| CliError::Ui(error.to_string()))?;
                let mut out = std::io::stdout().lock();
                writeln!(out, "Polaris web UI: {}", server.url())?;
                if let Some(endpoint) = attached {
                    writeln!(out, "Attached to polarisd at {endpoint}.")?;
                }
                writeln!(
                    out,
                    "Open it in a browser on this machine; Ctrl+C stops the server."
                )?;
                out.flush()?;
                drop(out);
                server
                    .serve(async {
                        let _signal = crate::app::shutdown_signal().await;
                    })
                    .await
                    .map_err(|error| CliError::Ui(error.to_string()))
            }
            Some(Action::Run { task }) => {
                headless(&args.state_dir, &args.workspace, args.session, &task).await
            }
            Some(Action::Task { profile, task }) => {
                let summary = crate::task::run(
                    &args.state_dir,
                    &args.workspace,
                    profile.as_deref(),
                    &task,
                    &loaded.config,
                    args.env_file.as_deref(),
                )
                .await?;
                if let Some(answer) = summary.answer.as_deref() {
                    let mut err = std::io::stderr().lock();
                    writeln!(err, "{answer}")?;
                    err.flush()?;
                }
                let mut out = std::io::stdout().lock();
                serde_json::to_writer(&mut out, &summary)?;
                writeln!(out)?;
                out.flush()?;
                // Proof of Done covers changes; a read-only answer has nothing to verify.
                if summary.proof_status == "verified" || summary.rights == ["read"] {
                    Ok(())
                } else {
                    Err(CliError::TaskUnverified)
                }
            }
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
fn status(dirs: &polaris_config::Dirs, json: bool) -> Result<(), CliError> {
    let report = polaris_kernel::inspect_skill_status(dirs);
    let mut out = std::io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &report)?;
        writeln!(out)?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
    }
    out.flush()?;
    Ok(())
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
    let credential_presence = setup
        .get("items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("id").and_then(serde_json::Value::as_str) == Some("win-keyring")
            })
        })
        .and_then(|item| item.get("observation").cloned());
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
    if !crate::doctor::is_ready(&setup) {
        return Err(CliError::Unavailable(
            "required setup prerequisites are missing; follow the reported onboarding actions",
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
pub(crate) async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    let mut close = tokio::signal::windows::ctrl_close()?;
    let mut logoff = tokio::signal::windows::ctrl_logoff()?;
    let mut shutdown = tokio::signal::windows::ctrl_shutdown()?;
    tokio::select! { _ = ctrl_c.recv() => {}, _ = ctrl_break.recv() => {}, _ = close.recv() => {}, _ = logoff.recv() => {}, _ = shutdown.recv() => {} }
    Ok(())
}
#[cfg(unix)]
pub(crate) async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { result = tokio::signal::ctrl_c() => result?, _ = terminate.recv() => {} }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/apps/polaris-cli/src/app.rs"]
mod tests;
