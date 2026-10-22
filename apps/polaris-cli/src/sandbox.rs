//! Headless review/approve interface for the real P2 execution profile.
use crate::CliError;
use clap::{Args, Subcommand};
use polaris_kernel::{ExecutionRequest, Kernel, LocalKernelConfig};
use polaris_sdk::{
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{EventSource, KernelApi, SandboxSpec},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Explicit gateway and immutable sandbox policy configuration.
#[derive(Debug, Args)]
pub struct SandboxArgs {
    /// Immutable local Docker image ID or OCI reference with sha256 digest.
    #[arg(long)]
    pub image: String,
    /// OpenShell policy YAML. Secrets remain in provider profiles.
    #[arg(long)]
    pub policy: PathBuf,
    /// Gateway directory containing ca.crt, tls.crt and tls.key.
    #[arg(long)]
    pub mtls_dir: PathBuf,
    /// TLS gateway endpoint.
    #[arg(long, default_value = "https://localhost:17670")]
    pub endpoint: String,
    /// Expected certificate DNS name.
    #[arg(long, default_value = "localhost")]
    pub server_name: String,
    /// Distribution kept alive for a local Windows gateway connection.
    #[arg(long, default_value = "Ubuntu-24.04")]
    pub wsl_distro: String,
    /// Review or execute an explicitly approved immutable artifact.
    #[command(subcommand)]
    pub action: SandboxAction,
}
/// Headless human approval steps.
#[derive(Debug, Subcommand)]
pub enum SandboxAction {
    /// Create a review artifact; starts no sandbox.
    Review {
        /// JSON ExecutionRequest with ordinary and independent argv and bounded limits.
        #[arg(long)]
        request: PathBuf,
    },
    /// Execute a review; use the original global --session from Review.
    Execute {
        /// Exact stored review CID.
        #[arg(long)]
        review: Cid,
        /// Operator-accepted digest copied from the displayed review.
        #[arg(long)]
        approve: Cid,
        /// Original frame printed by Review.
        #[arg(long)]
        frame: FrameId,
    },
}
pub(crate) async fn run(
    args: SandboxArgs,
    root: &Path,
    workspace: &Path,
    session: Option<SessionId>,
) -> Result<(), CliError> {
    let local_gateway = args
        .endpoint
        .strip_prefix("https://")
        .is_some_and(|authority| {
            matches!(
                authority
                    .split('/')
                    .next()
                    .and_then(|host| host.split(':').next()),
                Some("localhost" | "127.0.0.1")
            )
        });
    let keepalive = if cfg!(windows) && local_gateway {
        let wsl = polaris_platform::wsl::Wsl::locate()
            .map_err(|_| CliError::Unavailable("local gateway requires a WSL owner"))?;
        let distro = polaris_platform::wsl::DistroName::new(&args.wsl_distro)
            .map_err(|_| CliError::Unavailable("invalid WSL distribution"))?;
        Some(
            wsl.keepalive(&distro)
                .await
                .map_err(|_| CliError::Unavailable("WSL keepalive failed"))?,
        )
    } else {
        None
    };
    let outcome = run_owned(args, root, workspace, session).await;
    let stopped = match keepalive {
        Some(owner) => owner
            .shutdown()
            .await
            .map_err(|_| CliError::Unavailable("WSL keepalive cleanup failed")),
        None => Ok(()),
    };
    stopped?;
    outcome
}
async fn run_owned(
    args: SandboxArgs,
    root: &Path,
    workspace: &Path,
    session: Option<SessionId>,
) -> Result<(), CliError> {
    let (session, frame) = match &args.action {
        SandboxAction::Review { .. } => (
            match session {
                Some(session) => session,
                None => SessionId::new_v7(),
            },
            FrameId::new_v7(),
        ),
        SandboxAction::Execute { frame, .. } => (
            session.ok_or(CliError::Unavailable(
                "execution requires the original --session from Review",
            ))?,
            *frame,
        ),
    };
    let tls = polaris_openshell::client::GatewayTls {
        ca_certificate: read_bounded(&args.mtls_dir.join("ca.crt"), 1024 * 1024)?,
        client_certificate: read_bounded(&args.mtls_dir.join("tls.crt"), 1024 * 1024)?,
        client_key: read_bounded(&args.mtls_dir.join("tls.key"), 1024 * 1024)?,
        server_name: args.server_name,
    };
    let mut config = polaris_openshell::client::OpenShellConfig::wsl2(tls);
    config.endpoint = args.endpoint;
    let backend = Arc::new(
        polaris_openshell::client::OpenShellBackend::connect(config)
            .await
            .map_err(polaris_kernel::KernelError::from)?,
    );
    let policy = String::from_utf8(read_bounded(&args.policy, 1024 * 1024)?)
        .map_err(|_| CliError::Unavailable("policy must be UTF-8"))?;
    let policy = polaris_openshell::policy::Policy::parse_yaml(&policy)
        .map_err(|_| CliError::Unavailable("sandbox policy invalid"))?;
    let cid = backend
        .register_policy(&policy)
        .map_err(polaris_kernel::KernelError::from)?;
    let spec = SandboxSpec {
        image: Some(args.image),
        policy: Some(cid),
        cpu_millis: Some(1000),
        memory_mib: Some(512),
        ..SandboxSpec::default()
    };
    let kernel = Kernel::start_sandbox(
        LocalKernelConfig::new(root, workspace, session, frame),
        backend,
        spec,
    )
    .await?;
    let outcome=async {
        match args.action {
            SandboxAction::Review{request}=>{
                let request:ExecutionRequest=serde_json::from_slice(&read_bounded(&request,65_536)?)?;
                let review=kernel.prepare_execution(request).await?;
                Ok(serde_json::json!({"session":session,"frame":frame,"review":review}))
            }
            SandboxAction::Execute{review,approve,..}=>{
                tokio::select! {
                    result=kernel.approve_execution(review,approve)=>{
                        let receipt=result?;
                        Ok(serde_json::json!({"session":session,"frame":frame,"command":receipt.command,"accepted_at":receipt.accepted_at,"review":review,"status":"proof_passed"}))
                    },
                    signal=tokio::signal::ctrl_c()=>{signal?;Err(CliError::Unavailable("execution interrupted; owned shutdown requested"))},
                }
            }
        }
    }.await;
    let report = kernel.shutdown().await?;
    let mut events = kernel.subscribe(Seq::new(1)).await?;
    let mut records = Vec::new();
    while let Some(record) = events.next_event().await {
        if record.session() == session {
            if records.len() >= 10_000 {
                return Err(CliError::Unavailable(
                    "execution transcript exceeds bound; inspect durable event log",
                ));
            }
            records.push(record);
        }
    }
    if report.deadline_exceeded
        || report.journal_error.is_some()
        || report.entries.iter().any(|entry| {
            !matches!(
                entry.outcome,
                polaris_kernel::reaper::Outcome::Released | polaris_kernel::reaper::Outcome::Forced
            )
        })
    {
        return Err(CliError::Unavailable(
            "sandbox shutdown left unresolved resources; inspect doctor",
        ));
    }
    match outcome {
        Ok(mut value) => {
            value["events"] = serde_json::to_value(records)?;
            value["cleanup"] =
                serde_json::json!({"deadline_exceeded":false,"unresolved_resources":0});
            write_json(&value)
        }
        Err(error) => {
            write_json(
                &serde_json::json!({"session":session,"frame":frame,"status":"blocked","events":records,"cleanup":{"deadline_exceeded":false,"unresolved_resources":0}}),
            )?;
            Err(error)
        }
    }
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, CliError> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_err(|_| CliError::Unavailable("input length overflow"))?
        > limit
    {
        return Err(CliError::Unavailable("input exceeds size bound"));
    }
    Ok(bytes)
}
fn write_json(value: &serde_json::Value) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
