//! `polaris-web`: the Leptos web UI over a local session store, or attached to a `polarisd`
//! session with `--connect`, on loopback only.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Serve the Leptos web UI on this machine.
#[derive(Debug, Parser)]
#[command(name = "polaris-web", version, about = "Polaris web UI (Leptos)")]
struct Args {
    /// Loopback address to listen on.
    #[arg(long, default_value = "127.0.0.1:8788")]
    bind: SocketAddr,
    /// Durable state root.
    #[arg(long, default_value = ".polaris/state")]
    state_dir: PathBuf,
    /// Workspace of new sessions.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// Dotenv file with provider keys (default: `~/.polaris/.env` when present).
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Site built by `cargo leptos build` (holds `pkg/`).
    #[arg(long, default_value = "target/site")]
    site_root: String,
    /// Attach to the `polarisd` listening here (a `polaris-*` pipe name, a socket path, or
    /// `tcp://host:port` with mutual TLS files in `POLARIS_TLS_CA/CERT/KEY`).
    #[arg(long, requires_all = ["session", "token_file"])]
    connect: Option<String>,
    /// The daemon's session id (as passed to `polarisd --session`).
    #[arg(long, requires = "connect")]
    session: Option<String>,
    /// Credential file written by `polarisd --client-token-out`.
    #[arg(long, requires = "connect")]
    token_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
enum WebAppError {
    #[error(transparent)]
    Config(#[from] polaris_config::ConfigError),
    #[error(transparent)]
    Web(#[from] polaris_ui_web::WebError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("--connect: {0}")]
    Attach(String),
}

fn attach_target(args: &Args) -> Result<Option<polaris_ui_web::AttachTarget>, WebAppError> {
    let (Some(endpoint), Some(session), Some(token_file)) =
        (&args.connect, &args.session, &args.token_file)
    else {
        return Ok(None);
    };
    let credential = std::fs::read_to_string(token_file)?;
    // A `tcp://` daemon off loopback takes its mutual TLS files from POLARIS_TLS_*.
    polaris_ui_web::AttachTarget::new(endpoint, session, &credential)
        .map(|target| Some(target.with_env_tls()))
        .map_err(|error| WebAppError::Attach(error.to_string()))
}

#[tokio::main]
async fn main() -> Result<(), WebAppError> {
    let args = Args::parse();
    let attach = attach_target(&args)?;
    let dirs = polaris_config::discover(&args.workspace)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )?;
    let loaded = polaris_config::load(&layers, &polaris_config::TrustGrant::none())?;
    // A repository `.env` is never read implicitly: only an explicit file or the global one.
    let env_file = args.env_file.clone().or_else(|| {
        let global = dirs.global.join(".env");
        global.is_file().then_some(global)
    });
    let pages =
        polaris_web_app::server::pages(polaris_web_app::server::site_options(&args.site_root));
    let attached = attach.as_ref().map(|target| target.endpoint.clone());
    let server = polaris_ui_web::WebServer::bind_with(
        polaris_ui_web::WebOptions {
            bind: args.bind,
            state_dir: args.state_dir,
            workspace: args.workspace,
            config: loaded.config,
            env_file,
        },
        Some(pages),
        attach,
    )
    .await?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "Polaris web UI (Leptos): {}", server.url())?;
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
            let _signal = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
