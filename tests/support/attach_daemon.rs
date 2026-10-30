//! A real `polarisd` on a private local endpoint for the P9 attach tests: its own state and
//! workspace, and the hex credential `polarisd --client-token-out` would write. Included by the
//! session, web and desktop attach tests via `#[path]`.

#![allow(dead_code, reason = "each attach test uses a different subset")]

use std::error::Error;
use std::time::Duration;

use polaris_daemon::{DaemonConfig, DaemonError, DaemonSettings, EndpointSettings};
use polaris_kernel::LocalKernelConfig;
use polaris_sdk::ids::{FrameId, SessionId};
use polaris_ui_session::{AttachTarget, RemoteSession};

pub type AnyError = Box<dyn Error + Send + Sync>;

const FRAME: u128 = 0xF00D;

/// One running daemon; [`Daemon::stop`] shuts it down.
pub struct Daemon {
    /// Pipe name or socket path.
    pub endpoint: String,
    /// The daemon's session.
    pub session: SessionId,
    /// Hex credential the daemon trusts.
    pub credential: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), DaemonError>>,
    _state: tempfile::TempDir,
    _workspace: tempfile::TempDir,
}

#[cfg(windows)]
fn endpoint(_state: &std::path::Path) -> Result<(EndpointSettings, String), AnyError> {
    let name = format!("polaris-attach-{}", polaris_sdk::ids::CommandId::new_v7());
    Ok((EndpointSettings::NamedPipe(name.clone()), name))
}

#[cfg(unix)]
fn endpoint(state: &std::path::Path) -> Result<(EndpointSettings, String), AnyError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700))?;
    let path = state.join("daemon.sock");
    let shown = path.display().to_string();
    Ok((EndpointSettings::Unix(path), shown))
}

impl Daemon {
    /// Serve a fresh daemon session on the current tokio runtime.
    pub fn start() -> Result<Self, AnyError> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let (settings, endpoint) = endpoint(state.path())?;
        let session = SessionId::from_u128(0xA77A_C11E);
        let frame = FrameId::from_u128(FRAME);
        let (root, credential) =
            polaris_daemon::issue_local_client_token(session, frame, Duration::from_secs(600))?;
        let config = DaemonConfig {
            kernel: LocalKernelConfig::new(state.path(), workspace.path(), session, frame),
            endpoint: settings,
            token_root: root,
            settings: DaemonSettings::default(),
        };
        let (stop, signal) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(polaris_daemon::serve_daemon(config, async move {
            let _received = signal.await;
        }));
        Ok(Self {
            endpoint,
            session,
            credential: polaris_daemon::credential_hex(&credential),
            stop: Some(stop),
            task,
            _state: state,
            _workspace: workspace,
        })
    }

    /// Where a frontend attaches, with the trusted credential.
    pub fn target(&self) -> Result<AttachTarget, AnyError> {
        Ok(AttachTarget::new(
            &self.endpoint,
            &self.session.to_string(),
            &self.credential,
        )?)
    }

    /// A well-formed credential from an issuer this daemon does not trust.
    pub fn foreign_target(&self) -> Result<AttachTarget, AnyError> {
        let (_root, credential) = polaris_daemon::issue_local_client_token(
            self.session,
            FrameId::from_u128(FRAME),
            Duration::from_secs(600),
        )?;
        Ok(AttachTarget::new(
            &self.endpoint,
            &self.session.to_string(),
            &polaris_daemon::credential_hex(&credential),
        )?)
    }

    /// Wait until the endpoint accepts an authenticated client.
    pub async fn ready(&self) -> Result<(), AnyError> {
        let target = self.target()?;
        for _ in 0..200 {
            if RemoteSession::attach(&target).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err("polarisd did not become ready".into())
    }

    /// Signal shutdown and wait for the daemon to stop cleanly.
    pub async fn stop(mut self) -> Result<(), AnyError> {
        if let Some(stop) = self.stop.take() {
            let _sent = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(10), &mut self.task)
            .await
            .map_err(|_| "polarisd shutdown exceeded ten seconds")???;
        Ok(())
    }
}
