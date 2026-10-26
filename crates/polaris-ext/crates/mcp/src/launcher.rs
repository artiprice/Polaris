use std::{future::Future, path::Path, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use polaris_config::SecretRef;
use polaris_sdk::{
    interactive::{
        InteractiveSandbox, InteractiveSession, StdioChunk, StdioControl, StdioInput, StdioOutput,
        StdioRequest,
    },
    registry::{SandboxSpec, SecretSource},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf},
    task::JoinHandle,
};

use crate::{
    error::McpError,
    frame::{BoundedFrameReader, MAX_JSONRPC_FRAME_BYTES},
    model::{McpServerSpec, McpTransport},
};

const BRIDGE_BUFFER_BYTES: usize = 65_536;
const PROCESS_CHUNK_BYTES: usize = 8_192;
const PROCESS_STOP_GRACE: Duration = Duration::from_secs(3);
const PUMP_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Future returned by a process launcher.
pub type LaunchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LaunchedStdio, McpError>> + Send + 'a>>;

/// Production/test seam for sandbox-supervised stdio MCP processes.
pub trait ProcessLauncher: Send + Sync {
    /// Starts the configured server through an isolated process launcher.
    ///
    /// # Errors
    /// Returns a sanitized error when configuration, credentials, sandbox startup, or bridging
    /// fails. Implementations must not start a process directly on the host.
    fn launch<'a>(&'a self, spec: &'a McpServerSpec) -> LaunchFuture<'a>;
}

/// Stdio process launcher backed by the SDK's dedicated interactive sandbox contract.
pub struct SandboxProcessLauncher<S: InteractiveSandbox> {
    sandbox_backend: Arc<S>,
    sandbox_spec: SandboxSpec,
    secret_source: Arc<dyn SecretSource>,
}

impl<S: InteractiveSandbox> std::fmt::Debug for SandboxProcessLauncher<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxProcessLauncher")
            .field("sandbox_spec", &self.sandbox_spec)
            .finish_non_exhaustive()
    }
}

impl<S: InteractiveSandbox> SandboxProcessLauncher<S> {
    /// Creates a launcher using a caller-supplied pinned image and policy-bearing sandbox spec.
    ///
    /// # Errors
    /// Returns [`McpError::InvalidSpec`] for missing or empty image/policy references.
    pub fn new(
        sandbox_backend: Arc<S>,
        sandbox_spec: SandboxSpec,
        secret_source: Arc<dyn SecretSource>,
    ) -> Result<Self, McpError> {
        if sandbox_spec.image.as_deref().is_none_or(str::is_empty) || sandbox_spec.policy.is_none()
        {
            return Err(McpError::InvalidSpec);
        }
        Ok(Self {
            sandbox_backend,
            sandbox_spec,
            secret_source,
        })
    }
}

impl<S: InteractiveSandbox + 'static> ProcessLauncher for SandboxProcessLauncher<S> {
    fn launch<'a>(&'a self, spec: &'a McpServerSpec) -> LaunchFuture<'a> {
        Box::pin(async move {
            let McpTransport::Stdio { command, cwd, env } = &spec.transport else {
                return Err(McpError::InvalidSpec);
            };
            if command.is_empty() {
                return Err(McpError::InvalidSpec);
            }
            let workdir = match cwd {
                Some(path) => Some(path_to_sandbox_string(path)?),
                None => None,
            };
            let secret_env = resolve_env(env, self.secret_source.as_ref())?;
            let request = StdioRequest {
                argv: command.clone(),
                workdir,
                env: secret_env,
                timeout: None,
                output_limit_bytes: Some(MAX_JSONRPC_FRAME_BYTES),
            };
            let session = self
                .sandbox_backend
                .launch_stdio(&self.sandbox_spec, &request)
                .await
                .map_err(|_| McpError::Spawn)?;
            bridge_session(session, MAX_JSONRPC_FRAME_BYTES)
        })
    }
}

fn path_to_sandbox_string(path: &Path) -> Result<String, McpError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(McpError::InvalidSpec)
}

fn resolve_env(
    env: &[(String, SecretRef)],
    secret_source: &dyn SecretSource,
) -> Result<Vec<(String, secrecy::SecretString)>, McpError> {
    let mut resolved = Vec::with_capacity(env.len());
    for (name, secret_ref) in env {
        let location = secret_ref.location().map_err(|_| McpError::InvalidSpec)?;
        let secret = secret_source
            .load(&location)
            .map_err(|_| McpError::SecretUnavailable)?
            .ok_or(McpError::SecretUnavailable)?;
        resolved.push((name.clone(), secret));
    }
    Ok(resolved)
}

/// Owned rmcp-side reader/writer and process cleanup handles.
pub struct LaunchedStdio {
    /// Bounded stdout reader consumed by rmcp.
    pub reader: BoundedFrameReader<ReadHalf<DuplexStream>>,
    /// Stdin writer consumed by rmcp.
    pub writer: WriteHalf<DuplexStream>,
    /// Process and bridge lifecycle owner.
    pub lifecycle: StdioLifecycle,
}

impl std::fmt::Debug for LaunchedStdio {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchedStdio")
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

impl LaunchedStdio {
    /// Bridges SDK-owned split process handles into bounded rmcp-compatible streams.
    ///
    /// # Errors
    /// Returns a sanitized configuration error for an invalid frame bound.
    pub fn from_interactive(
        session: InteractiveSession,
        max_frame_bytes: usize,
    ) -> Result<Self, McpError> {
        bridge_session(session, max_frame_bytes)
    }
}

/// Owns the process cleanup handle and both bounded stream bridge tasks.
pub struct StdioLifecycle {
    control: Arc<dyn StdioControl>,
    pumps: Vec<JoinHandle<Result<(), McpError>>>,
    stopped: bool,
}

impl std::fmt::Debug for StdioLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StdioLifecycle")
            .field("pump_count", &self.pumps.len())
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl StdioLifecycle {
    /// Confirms sandbox teardown and joins the I/O bridge tasks.
    ///
    /// # Errors
    /// Returns a sanitized transport error when cleanup or a bridge task fails.
    pub async fn shutdown(&mut self) -> Result<(), McpError> {
        self.control
            .stop(PROCESS_STOP_GRACE)
            .await
            .map_err(|_| McpError::Spawn)?;
        self.stopped = true;
        let mut first_error = None;
        for mut pump in self.pumps.drain(..) {
            match tokio::time::timeout(PUMP_JOIN_TIMEOUT, &mut pump).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Ok(Err(_)) => {
                    if first_error.is_none() {
                        first_error = Some(McpError::Transport);
                    }
                }
                Err(_) => {
                    pump.abort();
                    let _join_result = pump.await;
                    if first_error.is_none() {
                        first_error = Some(McpError::Transport);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for StdioLifecycle {
    fn drop(&mut self) {
        if !self.stopped {
            self.control.request_stop();
            for pump in &self.pumps {
                pump.abort();
            }
        }
    }
}

fn bridge_session(
    session: InteractiveSession,
    max_frame_bytes: usize,
) -> Result<LaunchedStdio, McpError> {
    let (client_stream, server_stream) = tokio::io::duplex(BRIDGE_BUFFER_BYTES);
    let (client_reader, client_writer) = tokio::io::split(client_stream);
    let (server_reader, server_writer) = tokio::io::split(server_stream);
    let frame_reader = match BoundedFrameReader::new(client_reader, max_frame_bytes) {
        Ok(reader) => reader,
        Err(_) => {
            session.control.request_stop();
            return Err(McpError::InvalidSpec);
        }
    };
    let input_pump = tokio::spawn(pump_input(server_reader, session.input));
    let output_pump = tokio::spawn(pump_output(server_writer, session.output, max_frame_bytes));
    let lifecycle = StdioLifecycle {
        control: session.control,
        pumps: vec![input_pump, output_pump],
        stopped: false,
    };
    Ok(LaunchedStdio {
        reader: frame_reader,
        writer: client_writer,
        lifecycle,
    })
}

async fn pump_input(
    mut reader: ReadHalf<DuplexStream>,
    mut input: Box<dyn StdioInput>,
) -> Result<(), McpError> {
    let mut buffer = [0_u8; PROCESS_CHUNK_BYTES];
    loop {
        let read_count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| McpError::Transport)?;
        if read_count == 0 {
            input.close().await.map_err(|_| McpError::Spawn)?;
            return Ok(());
        }
        let bytes = buffer.get(..read_count).ok_or(McpError::Transport)?;
        input
            .send(Bytes::copy_from_slice(bytes))
            .await
            .map_err(|_| McpError::Transport)?;
    }
}

async fn pump_output(
    mut writer: WriteHalf<DuplexStream>,
    mut output: Box<dyn StdioOutput>,
    max_frame_bytes: usize,
) -> Result<(), McpError> {
    loop {
        let chunk = output
            .next(PROCESS_CHUNK_BYTES.min(max_frame_bytes))
            .await
            .map_err(|_| McpError::Transport)?;
        match chunk {
            Some(StdioChunk::Stdout(bytes)) => writer
                .write_all(&bytes)
                .await
                .map_err(|_| McpError::Transport)?,
            Some(StdioChunk::Stderr(_)) => {}
            Some(StdioChunk::Exit(_)) => {}
            None => return Ok(()),
        }
    }
}
