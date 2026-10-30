//! # polaris-desktop
//!
//! Native desktop GUI built with Slint: the welcome screen and the session view of
//! [`polaris_ui_gui`], backed by an embedded kernel session ([`bridge`]). Sending records the
//! message (or `$directive`) in the local session store shared with the terminal app; typed
//! messages are answered by the same provider chain and agent as the terminal, and file changes
//! wait for a decision in the review bar
//! (provider keys from `--env-file` or `~/.polaris/.env`). With `--connect <pipe> --session <id>
//! --token-file <file>` the window attaches to a `polarisd` session instead (P9): the daemon
//! records each message and the view follows its event stream.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L7 — application |
//! | Kind | app |
//! | Phase | P9 (welcome shell in progress) |
//!
//! Usage: `polaris-desktop [--static] [--workspace <dir>] [--state-dir <dir>] [--log-dir <dir>]
//! [--env-file <file>] [--connect <pipe> --session <id> --token-file <file>]`.
//! `--static`, or `POLARIS_REDUCED_MOTION` set to anything but `0`, draws a still aurora frame.
//! The session store defaults to `<workspace>/.polaris/state`, like the terminal app.
//!
//! Slint is used under its royalty-free licence (PLAN ADR-014); the attribution is the
//! `AboutSlint` widget in the Settings panel's About section.
//!
//! Test-first descriptors: `specs/descriptors/ui_desktop_main.json`.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod models;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use polaris_ui_gui::{AppOptions, AppReport};

/// Why the desktop app stopped.
#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    /// A command-line argument was not understood.
    #[error("{0}")]
    Usage(String),
    /// The log pipeline could not start.
    #[error("logging failed: {0}")]
    Log(#[from] polaris_log::LogError),
    /// The redactor that runs before every log sink could not compile.
    #[error("log redactor could not compile")]
    Redactor,
    /// The window failed.
    #[error(transparent)]
    Gui(#[from] polaris_ui_gui::GuiError),
    /// The configuration (model list, defaults) could not be loaded.
    #[error("configuration: {0}")]
    Config(#[from] polaris_config::ConfigError),
    /// The kernel bridge thread could not start or stopped abnormally.
    #[error("session bridge: {0}")]
    Bridge(&'static str),
    /// The `polarisd` attach target could not be read.
    #[error("--connect: {0}")]
    Attach(String),
}

/// A `polarisd` session to attach to (P9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectArgs {
    /// Pipe name (`polaris-…`) or socket path the daemon listens on.
    pub endpoint: String,
    /// The daemon's session id.
    pub session: String,
    /// Credential file written by `polarisd --client-token-out`.
    pub token_file: PathBuf,
}

/// Parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopArgs {
    /// Draw a still aurora frame.
    pub reduced_motion: bool,
    /// Workspace directory recorded with new sessions.
    pub workspace: PathBuf,
    /// Session store root; `None` means `<workspace>/.polaris/state`.
    pub state_dir: Option<PathBuf>,
    /// Directory of the rotating log file.
    pub log_dir: PathBuf,
    /// Dotenv file with provider keys; `None` means `~/.polaris/.env` when it exists.
    pub env_file: Option<PathBuf>,
    /// Attach to a `polarisd` session instead of the local store.
    pub connect: Option<ConnectArgs>,
}

const USAGE: &str = "usage: polaris-desktop [--static] [--workspace <dir>] [--state-dir <dir>] [--log-dir <dir>] [--env-file <file>] [--connect <pipe> --session <id> --token-file <file>]";

/// Parses the arguments after the program name.
///
/// `reduced_motion_env` is the value of `POLARIS_REDUCED_MOTION`; `cwd` and `local_app_data`
/// supply the defaults for `--workspace` and `--log-dir`.
///
/// # Errors
/// [`DesktopError::Usage`] for unknown flags, missing values, and `--connect` without
/// `--session` and `--token-file` (or either of those without `--connect`).
pub fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    reduced_motion_env: Option<OsString>,
    cwd: PathBuf,
    local_app_data: Option<PathBuf>,
) -> Result<DesktopArgs, DesktopError> {
    let mut reduced_motion = match reduced_motion_env {
        Some(value) => !value.is_empty() && value != "0",
        None => false,
    };
    let mut workspace = cwd;
    let mut log_dir = match local_app_data {
        Some(base) => base.join("Polaris").join("logs"),
        None => workspace.join(".polaris").join("logs"),
    };
    let mut state_dir = None;
    let mut env_file = None;
    let mut endpoint = None;
    let mut session = None;
    let mut token_file = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--static") => reduced_motion = true,
            Some("--workspace") => {
                let Some(value) = args.next() else {
                    return Err(DesktopError::Usage(format!(
                        "--workspace needs a directory\n{USAGE}"
                    )));
                };
                workspace = PathBuf::from(value);
            }
            Some("--state-dir") => {
                let Some(value) = args.next() else {
                    return Err(DesktopError::Usage(format!(
                        "--state-dir needs a directory
{USAGE}"
                    )));
                };
                state_dir = Some(PathBuf::from(value));
            }
            Some("--log-dir") => {
                let Some(value) = args.next() else {
                    return Err(DesktopError::Usage(format!(
                        "--log-dir needs a directory\n{USAGE}"
                    )));
                };
                log_dir = PathBuf::from(value);
            }
            Some("--env-file") => {
                let Some(value) = args.next() else {
                    return Err(DesktopError::Usage(format!(
                        "--env-file needs a file\n{USAGE}"
                    )));
                };
                env_file = Some(PathBuf::from(value));
            }
            Some("--connect") => {
                let Some(value) = args.next().and_then(|value| value.into_string().ok()) else {
                    return Err(DesktopError::Usage(format!(
                        "--connect needs the daemon's pipe name or socket path\n{USAGE}"
                    )));
                };
                endpoint = Some(value);
            }
            Some("--session") => {
                let Some(value) = args.next().and_then(|value| value.into_string().ok()) else {
                    return Err(DesktopError::Usage(format!(
                        "--session needs the daemon's session id\n{USAGE}"
                    )));
                };
                session = Some(value);
            }
            Some("--token-file") => {
                let Some(value) = args.next() else {
                    return Err(DesktopError::Usage(format!(
                        "--token-file needs a file\n{USAGE}"
                    )));
                };
                token_file = Some(PathBuf::from(value));
            }
            Some("--help" | "-h") => return Err(DesktopError::Usage(String::from(USAGE))),
            Some(other) => {
                return Err(DesktopError::Usage(format!(
                    "unknown argument `{other}`\n{USAGE}"
                )));
            }
            None => {
                return Err(DesktopError::Usage(format!(
                    "argument is not valid Unicode\n{USAGE}"
                )));
            }
        }
    }
    let connect = match (endpoint, session, token_file) {
        (None, None, None) => None,
        (Some(endpoint), Some(session), Some(token_file)) => Some(ConnectArgs {
            endpoint,
            session,
            token_file,
        }),
        _ => {
            return Err(DesktopError::Usage(format!(
                "--connect, --session and --token-file go together\n{USAGE}"
            )));
        }
    };
    Ok(DesktopArgs {
        reduced_motion,
        workspace,
        state_dir,
        log_dir,
        env_file,
        connect,
    })
}

/// Starts logging and the kernel bridge, then runs the window until it is closed.
///
/// # Errors
/// [`DesktopError`] when logging, the bridge or the window cannot start.
pub fn run(args: &DesktopArgs) -> Result<AppReport, DesktopError> {
    let redactor = polaris_security::automata::DfaRedactor::default_redactor()
        .map_err(|_| DesktopError::Redactor)?;
    let mut log_config = polaris_log::LogConfig::new(args.log_dir.clone());
    log_config.tui_mode = true;
    let log = polaris_log::init(log_config, redactor)?;
    // Same TLS provider setup as the terminal app; the offline session itself opens no
    // connections.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        tracing::warn!("TLS crypto provider was already installed");
    }
    let state_dir = match &args.state_dir {
        Some(dir) => dir.clone(),
        None => bridge::default_state_dir(&args.workspace),
    };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        reduced_motion = args.reduced_motion,
        "desktop starting (kernel session with provider-chain replies)"
    );
    // Same layered configuration as the terminal app; the model list comes from it.
    let dirs = polaris_config::discover(&args.workspace)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )?;
    let loaded = polaris_config::load(&layers, &polaris_config::TrustGrant::none())?;
    // `--static` or POLARIS_REDUCED_MOTION, else the preference the Settings page saved (shared
    // with the terminal UI); an unreadable preference file keeps full motion.
    let reduced_motion = args.reduced_motion
        || polaris_config::read_global_cli_preferences(&dirs)
            .map(|preferences| preferences.reduce_motion)
            .unwrap_or(false);
    let (models, default_model) = models::model_options(&loaded.config.nebius.effort);
    let chat = bridge::ChatOptions {
        env_file: polaris_ui_session::default_env_file(args.env_file.as_deref(), &dirs.global),
        config: loaded.config.clone(),
    };
    let (link, bridge) = match &args.connect {
        Some(connect) => {
            let credential = std::fs::read_to_string(&connect.token_file).map_err(|error| {
                DesktopError::Attach(format!("cannot read the credential file: {error}"))
            })?;
            let target = polaris_ui_session::AttachTarget::new(
                &connect.endpoint,
                &connect.session,
                &credential,
            )
            .map_err(|error| DesktopError::Attach(error.to_string()))?
            // A `tcp://` daemon off loopback takes its mutual TLS files from POLARIS_TLS_*.
            .with_env_tls();
            tracing::info!(endpoint = %connect.endpoint, "desktop attaching to polarisd");
            bridge::spawn_attached(state_dir.clone(), args.workspace.clone(), target)?
        }
        None => bridge::spawn_with_chat(state_dir.clone(), args.workspace.clone(), Some(chat))?,
    };
    let options = AppOptions {
        version: String::from(env!("CARGO_PKG_VERSION")),
        workspace: args.workspace.display().to_string(),
        state_dir: state_dir.display().to_string(),
        reduced_motion,
        models,
        default_model,
    };
    let result = polaris_ui_gui::run_app(&options, link);
    match &result {
        Ok(report) => tracing::info!(histogram = %report.histogram.summary(), "desktop closed"),
        Err(error) => tracing::error!(%error, "desktop failed"),
    }
    // The window dropped its request sender: the bridge closes the session and exits.
    let joined = bridge.join();
    if let Err(error) = &joined {
        tracing::error!(%error, "bridge did not stop cleanly");
    }
    let _flushed = log.flush(Duration::from_secs(2));
    let report = result?;
    joined?;
    Ok(report)
}
