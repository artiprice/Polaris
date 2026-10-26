//! `polarisd` command-line entry point.
use clap::Parser;
use polaris_daemon::{
    DaemonConfig, DaemonSettings, EndpointSettings, read_token_public_key, serve_daemon,
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
    #[arg(long)]
    token_root_public_key: PathBuf,
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
    let token_root = read_token_public_key(&args.token_root_public_key)?;
    let settings = DaemonSettings {
        event_buffer: args.event_buffer,
        backpressure_timeout: Duration::from_millis(args.backpressure_timeout_ms),
        max_connections: args.max_connections,
        streams_per_connection: args.streams_per_connection,
        shutdown_grace: Duration::from_secs(args.shutdown_grace_secs),
    }
    .validate()?;
    let config = DaemonConfig {
        kernel: LocalKernelConfig::new(
            &args.state_dir,
            &args.workspace,
            SessionId::from_u128(args.session),
            FrameId::from_u128(args.frame),
        ),
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
        serve_daemon(config, shutdown).await
    });
    let flushed = log_guard.flush(Duration::from_secs(2));
    match (result, flushed) {
        (Err(error), _) => Err(error),
        (Ok(()), true) => Ok(()),
        (Ok(()), false) => Err(polaris_daemon::DaemonError::LogFlush),
    }
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
