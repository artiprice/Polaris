#![forbid(unsafe_code)]

use std::{
    convert::Infallible,
    io,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use bytes::Bytes;
use polaris_ext_mcp::{
    CancelToken, EffectAuthorization, EffectAuthorizer, LaunchFuture, LaunchedStdio, McpHub,
    McpServerSpec, McpTransport, ProcessLauncher, ServerName, ToolContent, ToolSpec, TrustLevel,
};
use polaris_sdk::{
    interactive::{
        InteractiveSession, StdioChunk, StdioControl, StdioFuture, StdioInput, StdioOutput,
    },
    registry::{SandboxError, SandboxHandle, SecretSource, SecretSourceError},
    secrets::SecretLocation,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use secrecy::SecretString;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, mpsc},
    task::JoinHandle,
};

const FIXTURE_CHILD_ENV: &str = "POLARIS_MCP_STDIO_FIXTURE_CHILD";

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EchoRequest {
    message: String,
}

#[derive(Debug, Clone)]
struct FixtureServer {
    tool_router: ToolRouter<Self>,
}

impl FixtureServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl FixtureServer {
    #[tool(description = "Returns the supplied fixture text")]
    fn echo(&self, Parameters(EchoRequest { message }): Parameters<EchoRequest>) -> String {
        message
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Child mode is activated only by the stdio integration test launcher.
#[tokio::test]
async fn mcp_stdio_fixture_server_process() -> Result<(), io::Error> {
    if std::env::var_os(FIXTURE_CHILD_ENV).is_none() {
        return Ok(());
    }
    let server = FixtureServer::new();
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|_| io::Error::other("fixture server failed to initialize"))?;
    service
        .waiting()
        .await
        .map_err(|_| io::Error::other("fixture server process failed"))?;
    Ok(())
}

#[tokio::test]
async fn connect_stdio_fixture_lists_and_calls_tool_then_reaps_child()
-> Result<(), Box<dyn std::error::Error>> {
    let launched_child = Arc::new(StdMutex::new(None));
    let launcher = Arc::new(LocalChildLauncher {
        launched_child: Arc::clone(&launched_child),
    });
    let hub = McpHub::new(launcher, Arc::new(NoSecrets));
    let server = server_name("stdio-fixture")?;
    hub.connect(server_spec(
        server.clone(),
        McpTransport::Stdio {
            command: vec!["fixture".to_owned()],
            cwd: None,
            env: Vec::new(),
        },
    ))
    .await?;
    let tools = hub.tools().await;
    assert_eq!(tools.len(), 1);
    if let Some(tool) = tools.first() {
        assert_eq!(tool.effects, polaris_ext_mcp::ToolEffect::CONSERVATIVE);
    }
    let tool_id = tools
        .first()
        .ok_or_else(|| io::Error::other("fixture tool should be listed"))?
        .id
        .clone();
    let denied = hub
        .call(
            &tool_id,
            serde_json::json!({"message": "must not execute"}),
            &CancelToken::new(),
            &DenyEffects,
        )
        .await;
    assert!(matches!(
        denied,
        Err(polaris_ext_mcp::McpError::UnauthorizedEffects)
    ));
    let rejected_arguments = hub
        .call(
            &tool_id,
            serde_json::json!({}),
            &CancelToken::new(),
            &AllowEffects,
        )
        .await;
    assert!(
        rejected_arguments.is_err(),
        "invalid tool arguments must be rejected before transport"
    );
    let outcome = hub
        .call(
            &tool_id,
            serde_json::json!({"message": "hello from stdio"}),
            &CancelToken::new(),
            &AllowEffects,
        )
        .await?;
    assert_eq!(
        outcome.content,
        vec![ToolContent::Text("hello from stdio".to_owned())]
    );
    hub.disconnect(&server).await?;
    let guard = launched_child
        .lock()
        .map_err(|_| io::Error::other("fixture child tracker poisoned"))?;
    let child = guard
        .as_ref()
        .ok_or_else(|| io::Error::other("fixture child should have launched"))?;
    let mut child = child
        .try_lock()
        .map_err(|_| io::Error::other("fixture child lifecycle lock unavailable"))?;
    let status = child.try_wait()?;
    if status.is_none() {
        return Err(io::Error::other("fixture child was not reaped").into());
    }
    Ok(())
}

#[tokio::test]
async fn connect_repo_unapproved_is_denied_before_launcher()
-> Result<(), Box<dyn std::error::Error>> {
    let hub = McpHub::new(Arc::new(UnexpectedLauncher), Arc::new(NoSecrets));
    let server = server_name("unapproved")?;
    let mut spec = server_spec(
        server,
        McpTransport::Stdio {
            command: vec!["fixture".to_owned()],
            cwd: None,
            env: Vec::new(),
        },
    );
    spec.trust = TrustLevel::RepoUnapproved;
    let result = hub.connect(spec).await;
    if !matches!(
        result,
        Err(polaris_ext_mcp::McpError::Denied(
            TrustLevel::RepoUnapproved
        ))
    ) {
        return Err(io::Error::other("unapproved repository server was not denied").into());
    }
    Ok(())
}

#[tokio::test]
async fn connect_http_fixture_lists_and_calls_tool() -> Result<(), Box<dyn std::error::Error>> {
    let (url, server_task) = start_http_fixture().await?;
    let launcher = Arc::new(UnexpectedLauncher);
    let hub = McpHub::new(launcher, Arc::new(NoSecrets));
    let server = server_name("http-fixture")?;
    hub.connect(server_spec(
        server.clone(),
        McpTransport::StreamableHttp {
            url,
            headers: Vec::new(),
        },
    ))
    .await?;
    let tools = hub.tools().await;
    assert_eq!(tools.len(), 1);
    let tool_id = tools
        .first()
        .ok_or_else(|| io::Error::other("fixture tool should be listed"))?
        .id
        .clone();
    let outcome = hub
        .call(
            &tool_id,
            serde_json::json!({"message": "hello from http"}),
            &CancelToken::new(),
            &AllowEffects,
        )
        .await?;
    assert_eq!(
        outcome.content,
        vec![ToolContent::Text("hello from http".to_owned())]
    );
    hub.disconnect(&server).await?;
    server_task.abort();
    let _server_result = server_task.await;
    Ok(())
}

async fn start_http_fixture() -> io::Result<(url::Url, JoinHandle<()>)> {
    use hyper::{Request, body::Incoming, service::service_fn};
    use hyper_util::rt::TokioIo;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let service = StreamableHttpService::new(
        || Ok(FixtureServer::new()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let current_service = service.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let http_service = service_fn(move |request: Request<Incoming>| {
                    let cloned_service = current_service.clone();
                    async move {
                        let response = cloned_service.handle(request).await;
                        Ok::<_, Infallible>(response)
                    }
                });
                let connection = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, http_service)
                    .await;
                match connection {
                    Ok(()) | Err(_) => {}
                }
            });
        }
    });
    let url = url::Url::parse(&format!("http://{address}/mcp"))
        .map_err(|_| io::Error::other("invalid test fixture URL"))?;
    Ok((url, task))
}

struct LocalChildLauncher {
    launched_child: Arc<StdMutex<Option<Arc<Mutex<Child>>>>>,
}

impl ProcessLauncher for LocalChildLauncher {
    fn launch<'a>(&'a self, _spec: &'a McpServerSpec) -> LaunchFuture<'a> {
        Box::pin(async move {
            let executable =
                std::env::current_exe().map_err(|_| polaris_ext_mcp::McpError::Spawn)?;
            let mut command = Command::new(executable);
            command
                .args(["--exact", "mcp_stdio_fixture_server_process", "--quiet"])
                .env(FIXTURE_CHILD_ENV, "enabled")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = command
                .spawn()
                .map_err(|_| polaris_ext_mcp::McpError::Spawn)?;
            let stdin = child.stdin.take().ok_or(polaris_ext_mcp::McpError::Spawn)?;
            let stdout = child
                .stdout
                .take()
                .ok_or(polaris_ext_mcp::McpError::Spawn)?;
            let child = Arc::new(Mutex::new(child));
            match self.launched_child.lock() {
                Ok(mut guard) => {
                    if guard.replace(Arc::clone(&child)).is_some() {
                        return Err(polaris_ext_mcp::McpError::Spawn);
                    }
                }
                Err(_) => return Err(polaris_ext_mcp::McpError::Spawn),
            };
            let (stop_sender, mut stop_receiver) = mpsc::unbounded_channel::<()>();
            let reaper_child = Arc::clone(&child);
            tokio::spawn(async move {
                if stop_receiver.recv().await.is_some() {
                    let mut child = reaper_child.lock().await;
                    match child.try_wait() {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            let kill_result = child.start_kill();
                            if kill_result.is_err() {
                                let wait_result = child.wait().await;
                                match wait_result {
                                    Ok(_) | Err(_) => {}
                                }
                            }
                        }
                        Err(_) => {}
                    }
                }
            });
            let control = Arc::new(LocalControl { child, stop_sender });
            let sandbox =
                SandboxHandle::new("mcp-fixture").map_err(|_| polaris_ext_mcp::McpError::Spawn)?;
            let session = InteractiveSession {
                sandbox,
                input: Box::new(LocalInput { stdin: Some(stdin) }),
                output: Box::new(LocalOutput { stdout }),
                control,
            };
            LaunchedStdio::from_interactive(session, polaris_ext_mcp::MAX_JSONRPC_FRAME_BYTES)
        })
    }
}

struct LocalInput {
    stdin: Option<ChildStdin>,
}

impl StdioInput for LocalInput {
    fn send(&mut self, bytes: Bytes) -> StdioFuture<'_, ()> {
        Box::pin(async move {
            let stdin = match self.stdin.as_mut() {
                Some(stdin) => stdin,
                None => {
                    return Err(SandboxError::PolicyDenied(
                        "fixture stdin closed".to_owned(),
                    ));
                }
            };
            stdin
                .write_all(&bytes)
                .await
                .map_err(|_| SandboxError::Unavailable("fixture stdin failed".to_owned()))
        })
    }

    fn close(&mut self) -> StdioFuture<'_, ()> {
        if let Some(stdin) = self.stdin.take() {
            drop(stdin);
        }
        Box::pin(async { Ok(()) })
    }
}

struct LocalOutput {
    stdout: ChildStdout,
}

impl StdioOutput for LocalOutput {
    fn next(&mut self, max_bytes: usize) -> StdioFuture<'_, Option<StdioChunk>> {
        Box::pin(async move {
            if max_bytes == 0 {
                return Err(SandboxError::LimitExceeded(
                    "fixture output bound".to_owned(),
                ));
            }
            let mut buffer = vec![0_u8; max_bytes];
            let read = self
                .stdout
                .read(&mut buffer)
                .await
                .map_err(|_| SandboxError::Unavailable("fixture stdout failed".to_owned()))?;
            if read == 0 {
                return Ok(None);
            }
            let bytes = buffer.get(..read).ok_or_else(|| {
                SandboxError::Protocol("fixture output bound mismatch".to_owned())
            })?;
            Ok(Some(StdioChunk::Stdout(Bytes::copy_from_slice(bytes))))
        })
    }
}

struct LocalControl {
    child: Arc<Mutex<Child>>,
    stop_sender: mpsc::UnboundedSender<()>,
}

impl StdioControl for LocalControl {
    fn wait(&self) -> StdioFuture<'_, i32> {
        Box::pin(async move {
            let mut child = self.child.lock().await;
            let status = child
                .wait()
                .await
                .map_err(|_| SandboxError::Unavailable("fixture wait failed".to_owned()))?;
            match status.code() {
                Some(code) => Ok(code),
                None => Ok(-1),
            }
        })
    }

    fn stop(&self, _grace: Duration) -> StdioFuture<'_, ()> {
        Box::pin(async move {
            let mut child = self.child.lock().await;
            match child
                .try_wait()
                .map_err(|_| SandboxError::Unavailable("fixture wait failed".to_owned()))?
            {
                Some(_) => {}
                None => {
                    let kill_result = child.start_kill();
                    if kill_result.is_err() {
                        return Err(SandboxError::Unavailable("fixture kill failed".to_owned()));
                    }
                }
            }
            child
                .wait()
                .await
                .map_err(|_| SandboxError::Unavailable("fixture wait failed".to_owned()))?;
            Ok(())
        })
    }

    fn request_stop(&self) {
        match self.stop_sender.send(()) {
            Ok(()) | Err(_) => {}
        }
    }
}

struct NoSecrets;

impl SecretSource for NoSecrets {
    fn load(&self, _location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        Ok(None)
    }

    fn store(
        &self,
        _location: &SecretLocation,
        _value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }

    fn delete(&self, _location: &SecretLocation) -> Result<bool, SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }
}

struct UnexpectedLauncher;

impl ProcessLauncher for UnexpectedLauncher {
    fn launch<'a>(&'a self, _spec: &'a McpServerSpec) -> LaunchFuture<'a> {
        Box::pin(async { Err(polaris_ext_mcp::McpError::Spawn) })
    }
}

struct AllowEffects;

impl EffectAuthorizer for AllowEffects {
    fn authorize(&self, _tool: &ToolSpec) -> EffectAuthorization {
        EffectAuthorization::Allow
    }
}

struct DenyEffects;

impl EffectAuthorizer for DenyEffects {
    fn authorize(&self, _tool: &ToolSpec) -> EffectAuthorization {
        EffectAuthorization::Deny
    }
}

fn server_name(name: &str) -> Result<ServerName, polaris_ext_mcp::McpError> {
    ServerName::new(name)
}

fn server_spec(name: ServerName, transport: McpTransport) -> McpServerSpec {
    McpServerSpec {
        name,
        transport,
        trust: TrustLevel::UserGlobal,
        startup_timeout: Duration::from_secs(8),
        call_timeout: Duration::from_secs(8),
        tool_allowlist: None,
        max_result_bytes: 256,
    }
}
