//! `polarisd` command-line entry point.
use clap::Parser;
use polaris_daemon::{
    DaemonConfig, DaemonSettings, EndpointSettings, ReplyConfig, read_token_public_key,
    serve_daemon_with_replies,
};
use polaris_kernel::LocalKernelConfig;
use polaris_sdk::ids::{FrameId, SessionId};
use std::{
    future::Future, io::Write, net::SocketAddr, path::PathBuf, process::ExitCode, time::Duration,
};

#[derive(Debug, Parser)]
#[command(
    name = "polarisd",
    version,
    about = "Authenticated headless Polaris kernel daemon"
)]
struct Args {
    /// Durable local kernel state directory.
    #[arg(long)]
    state_dir: PathBuf,
    /// Existing workspace directory exposed to the local kernel.
    #[arg(long)]
    workspace: PathBuf,
    /// Stable decimal session identity used for durable recovery.
    #[arg(long)]
    session: u128,
    /// Stable decimal root frame identity.
    #[arg(long)]
    frame: u128,
    /// File containing the trusted issuer's 32-byte public key or 64 hexadecimal digits.
    #[arg(
        long,
        required_unless_present = "client_token_out",
        conflicts_with = "client_token_out"
    )]
    token_root_public_key: Option<PathBuf>,
    /// Create a local issuer and write one client credential (hex) to this new file, for
    /// frontends attaching with `--connect` on this machine.
    #[arg(long)]
    client_token_out: Option<PathBuf>,
    /// Lifetime of the `--client-token-out` credential, in hours (1..=168).
    #[arg(long, default_value_t = 24)]
    client_token_hours: u64,
    /// Local Windows named-pipe basename (`polaris-*`).
    #[arg(long, conflicts_with_all = ["uds", "listen"])]
    pipe: Option<String>,
    /// Local Unix-domain socket path in a private directory.
    #[arg(long, conflicts_with_all = ["pipe", "listen"])]
    uds: Option<String>,
    /// TCP listener. Non-loopback addresses require all three TLS files.
    #[arg(long, conflicts_with_all = ["pipe", "uds"])]
    listen: Option<SocketAddr>,
    /// TLS server certificate PEM file.
    #[arg(long)]
    tls_cert: Option<String>,
    /// TLS server private key PEM file. The key is read into a redacted secret wrapper.
    #[arg(long)]
    tls_key: Option<String>,
    /// Client CA PEM file required for mutual TLS.
    #[arg(long)]
    client_ca: Option<String>,
    /// Maximum queued events for each subscriber (1..=1024).
    #[arg(long, default_value_t = 128)]
    event_buffer: usize,
    /// Per-subscriber backpressure deadline in milliseconds (1..=30000).
    #[arg(long, default_value_t = 2000)]
    backpressure_timeout_ms: u64,
    /// Maximum simultaneously accepted client connections (1..=1024).
    #[arg(long, default_value_t = 64)]
    max_connections: usize,
    /// Maximum HTTP/2 streams per connection (1..=128).
    #[arg(long, default_value_t = 16)]
    streams_per_connection: u32,
    /// Graceful transport drain deadline in seconds (1..=120).
    #[arg(long, default_value_t = 10)]
    shutdown_grace_secs: u64,
    /// Dotenv file with provider keys for model replies (default: `~/.polaris/.env` when
    /// present; a repository `.env` is never read implicitly).
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Record messages without answering them.
    #[arg(long)]
    no_replies: bool,
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let success = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if error.print().is_err() {
                return ExitCode::FAILURE;
            }
            return if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _reported = writeln!(std::io::stderr().lock(), "polarisd: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), polaris_daemon::DaemonError> {
    let state_dir = args.state_dir.clone();
    let endpoint = endpoint_from_args(&args)?;
    let session = SessionId::from_u128(args.session);
    let frame = FrameId::from_u128(args.frame);
    let token_root = match (&args.token_root_public_key, &args.client_token_out) {
        (Some(path), None) => read_token_public_key(path)?,
        (None, Some(out)) => {
            let (root, token) = polaris_daemon::issue_local_client_token(
                session,
                frame,
                Duration::from_secs(args.client_token_hours.saturating_mul(3600)),
            )?;
            write_private(out, &polaris_daemon::credential_hex(&token))?;
            let _printed = writeln!(
                std::io::stdout().lock(),
                "polarisd: client credential written to {}",
                out.display()
            );
            root
        }
        _ => {
            return Err(polaris_daemon::DaemonError::Configuration(
                "use exactly one of --token-root-public-key or --client-token-out",
            ));
        }
    };
    let settings = DaemonSettings {
        event_buffer: args.event_buffer,
        backpressure_timeout: Duration::from_millis(args.backpressure_timeout_ms),
        max_connections: args.max_connections,
        streams_per_connection: args.streams_per_connection,
        shutdown_grace: Duration::from_secs(args.shutdown_grace_secs),
    }
    .validate()?;
    let replies = if args.no_replies {
        None
    } else {
        Some(reply_config(&args)?)
    };
    let mut kernel = LocalKernelConfig::new(&args.state_dir, &args.workspace, session, frame);
    // The daemon's kernel starts the local System One worker the user's configuration names.
    kernel.s1 = polaris_ui_session::system_one::local_s1(&args.state_dir);
    let config = DaemonConfig {
        kernel,
        endpoint,
        token_root,
        settings,
    }
    .validate()?;

    let redactor = polaris_security_automata::DfaRedactor::default_redactor().map_err(|_| {
        polaris_daemon::DaemonError::Configuration("default log redactor unavailable")
    })?;
    let log_guard = polaris_log::init(
        polaris_log::LogConfig::new(state_dir.join("logs")),
        redactor,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(polaris_daemon::DaemonError::Runtime)?;
    let result = runtime.block_on(async {
        let shutdown = shutdown_signal()?;
        serve_daemon_with_replies(config, replies, shutdown).await
    });
    let flushed = log_guard.flush(Duration::from_secs(2));
    match (result, flushed) {
        (Err(error), _) => Err(error),
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => Err(polaris_daemon::DaemonError::LogFlush),
    }
}

/// Provider order and models from the configuration layers of the workspace, keys from an
/// explicit dotenv file or the global one.
fn reply_config(args: &Args) -> Result<ReplyConfig, polaris_daemon::DaemonError> {
    let invalid = |_| polaris_daemon::DaemonError::Configuration("configuration layers rejected");
    let dirs = polaris_config::discover(&args.workspace).map_err(invalid)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )
    .map_err(invalid)?;
    let loaded =
        polaris_config::load(&layers, &polaris_config::TrustGrant::none()).map_err(invalid)?;
    let system_one = polaris_ui_session::system_one::configure(&loaded.config, &dirs.global);
    let _reported = writeln!(std::io::stderr().lock(), "polarisd: {system_one}");
    let sandbox = polaris_ui_session::sandbox::configure(&dirs.global);
    let _reported = writeln!(std::io::stderr().lock(), "polarisd: {sandbox}");
    Ok(ReplyConfig {
        config: loaded.config,
        env_file: polaris_ui_session::default_env_file(args.env_file.as_deref(), &dirs.global),
    })
}

/// Create `path` (never overwrite) readable by the owner only where the platform allows it.
fn write_private(path: &std::path::Path, text: &str) -> Result<(), polaris_daemon::DaemonError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(polaris_daemon::DaemonError::TlsMaterial)?;
    file.write_all(text.as_bytes())
        .map_err(polaris_daemon::DaemonError::TlsMaterial)
}

fn endpoint_from_args(args: &Args) -> Result<EndpointSettings, polaris_daemon::DaemonError> {
    if args.tls_cert.is_some() || args.tls_key.is_some() || args.client_ca.is_some() {
        let Some(address) = args.listen else {
            return Err(polaris_daemon::DaemonError::Configuration(
                "TLS files are only valid with --listen",
            ));
        };
        return EndpointSettings::tcp(
            address,
            args.tls_cert.as_deref(),
            args.tls_key.as_deref(),
            args.client_ca.as_deref(),
        );
    }
    EndpointSettings::select(args.pipe.as_deref(), args.uds.as_deref(), args.listen)
}

fn shutdown_signal() -> Result<impl Future<Output = ()>, polaris_daemon::DaemonError> {
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(polaris_daemon::DaemonError::Signal)?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(polaris_daemon::DaemonError::Signal)?;
    #[cfg(windows)]
    let mut ctrl_c =
        tokio::signal::windows::ctrl_c().map_err(polaris_daemon::DaemonError::Signal)?;
    #[cfg(windows)]
    let mut ctrl_break =
        tokio::signal::windows::ctrl_break().map_err(polaris_daemon::DaemonError::Signal)?;
    #[cfg(unix)]
    return Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    });
    #[cfg(windows)]
    return Ok(async move {
        tokio::select! {
            _ = ctrl_c.recv() => {}
            _ = ctrl_break.recv() => {}
        }
    });
    #[cfg(not(any(unix, windows)))]
    Err(polaris_daemon::DaemonError::Configuration(
        "shutdown signals are unsupported on this platform",
    ))
}
