//! `polaris-web`: the Leptos web UI over a local session store, on loopback only.

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
}

#[derive(Debug, thiserror::Error)]
enum WebAppError {
    #[error(transparent)]
    Config(#[from] polaris_config::ConfigError),
    #[error(transparent)]
    Web(#[from] polaris_ui_web::WebError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), WebAppError> {
    let args = Args::parse();
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
    let server = polaris_ui_web::WebServer::bind_with_pages(
        polaris_ui_web::WebOptions {
            bind: args.bind,
            state_dir: args.state_dir,
            workspace: args.workspace,
            config: loaded.config,
            env_file,
        },
        Some(pages),
    )
    .await?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "Polaris web UI (Leptos): {}", server.url())?;
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
