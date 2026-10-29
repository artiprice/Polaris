use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use polaris_sdk::registry::SecretSource;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, PaginatedRequestParams, Tool},
    service::{RunningService, RunningServiceCancellationToken},
    transport::{
        StreamableHttpClientTransport, async_rw::AsyncRwTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use secrecy::ExposeSecret;
use tokio::{io::DuplexStream, sync::RwLock};
use tokio_util::sync::CancellationToken;

use crate::{
    call::bounded_outcome,
    error::McpError,
    frame::{BoundedFrameReader, MAX_JSONRPC_FRAME_BYTES},
    http_client::{BoundedHttpClient, is_reserved_header},
    launcher::{ProcessLauncher, StdioLifecycle},
    model::{
        EffectAuthorization, EffectAuthorizer, McpServerSpec, McpTransport, NamespacedToolId,
        ServerName, ToolEffect, ToolOutcome, ToolSpec, TrustLevel,
    },
    normalize_schema, prune_summary,
};

const MAX_TOOL_LIST_ENTRIES: usize = 512;
const MAX_TOOL_LIST_PAGES: usize = 32;
const MAX_SUMMARY_CHARS: usize = 256;
const MAX_HTTP_REQUESTS: usize = 8;

type StdioService = RunningService<RoleClient, ()>;

/// Owns configured MCP servers and the normalized registry view of their tools.
pub struct McpHub {
    launcher: Arc<dyn ProcessLauncher>,
    secret_source: Arc<dyn SecretSource>,
    servers: RwLock<HashMap<ServerName, Arc<ServerSession>>>,
    tools: RwLock<BTreeMap<NamespacedToolId, ToolSpec>>,
}

impl std::fmt::Debug for McpHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("McpHub").finish_non_exhaustive()
    }
}

impl McpHub {
    /// Creates a hub with an explicit supervised process launcher and secret source.
    #[must_use]
    pub fn new(launcher: Arc<dyn ProcessLauncher>, secret_source: Arc<dyn SecretSource>) -> Self {
        Self {
            launcher,
            secret_source,
            servers: RwLock::new(HashMap::new()),
            tools: RwLock::new(BTreeMap::new()),
        }
    }

    /// Connects a trusted server, negotiates MCP, and indexes its bounded tool list.
    ///
    /// Repository-provided unapproved specs are refused before secret lookup, process creation,
    /// or network access. Stdio processes launch only through the injected sandbox launcher.
    ///
    /// # Errors
    /// Returns a sanitized trust, startup, schema, transport, or configuration error.
    pub async fn connect(&self, spec: McpServerSpec) -> Result<(), McpError> {
        if spec.trust == TrustLevel::RepoUnapproved {
            return Err(McpError::Denied(spec.trust));
        }
        validate_spec(&spec)?;
        if self.servers.read().await.contains_key(&spec.name) {
            return Err(McpError::InvalidSpec);
        }
        let server_name = spec.name.clone();
        let call_timeout = spec.call_timeout;
        let max_result_bytes = spec.max_result_bytes;
        let (mut service, mut lifecycle) = self.connect_service(&spec).await?;
        let listed_tools =
            match timeout_result(spec.startup_timeout, list_server_tools(&service, &spec)).await {
                Ok(result) => result,
                Err(error) => {
                    cleanup_failed_service(&mut service, &mut lifecycle).await?;
                    return Err(error);
                }
            };
        let listed_tools = match listed_tools {
            Ok(tools) => tools,
            Err(error) => {
                cleanup_failed_service(&mut service, &mut lifecycle).await?;
                return Err(error);
            }
        };
        let service = Arc::new(service);
        let cancellation = service.cancellation_token();
        let session = Arc::new(ServerSession {
            service,
            call_timeout,
            max_result_bytes,
            session_cancel: CancellationToken::new(),
            service_cancel: Mutex::new(Some(cancellation)),
            lifecycle: tokio::sync::Mutex::new(lifecycle),
            spec,
        });
        let mut server_map = self.servers.write().await;
        if server_map.contains_key(&server_name) {
            session.shutdown().await?;
            return Err(McpError::InvalidSpec);
        }
        let mut tool_map = self.tools.write().await;
        for tool in &listed_tools {
            if tool_map.contains_key(&tool.id) {
                session.shutdown().await?;
                return Err(McpError::InvalidSpec);
            }
        }
        for tool in listed_tools {
            tool_map.insert(tool.id.clone(), tool);
        }
        server_map.insert(server_name, session);
        Ok(())
    }

    /// Returns a stable identifier-sorted snapshot of all available tools.
    pub async fn tools(&self) -> Vec<ToolSpec> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Invokes a remote tool after caller authorization and applies the configured result bound.
    ///
    /// # Errors
    /// Returns typed lookup, authorization, cancellation, timeout, or sanitized transport errors.
    pub async fn call<A: EffectAuthorizer + ?Sized>(
        &self,
        id: &NamespacedToolId,
        arguments: serde_json::Value,
        cancel: &CancellationToken,
        authorizer: &A,
    ) -> Result<ToolOutcome, McpError> {
        let tool = match self.tools.read().await.get(id).cloned() {
            Some(tool) => tool,
            None => return Err(McpError::ToolNotFound(id.clone())),
        };
        if authorizer.authorize(&tool) != EffectAuthorization::Allow {
            return Err(McpError::UnauthorizedEffects);
        }
        let arguments = arguments
            .as_object()
            .cloned()
            .ok_or(McpError::InvalidSpec)?;
        let validator =
            jsonschema::validator_for(&tool.input_schema).map_err(|_| McpError::SchemaInvalid)?;
        if !validator.is_valid(&serde_json::Value::Object(arguments.clone())) {
            return Err(McpError::InvalidSpec);
        }
        let session = match self.servers.read().await.get(&id.server).cloned() {
            Some(session) => session,
            None => return Err(McpError::ToolNotFound(id.clone())),
        };
        let request = CallToolRequestParams::new(std::borrow::Cow::Owned(id.tool.clone()))
            .with_arguments(arguments);
        let request = session.service.call_tool_with_mrtr_max_rounds(request, 1);
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(McpError::Cancelled),
            _ = session.session_cancel.cancelled() => return Err(McpError::Cancelled),
            response = tokio::time::timeout(session.call_timeout, request) => match response {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => return Err(McpError::Transport),
                Err(_) => return Err(McpError::Timeout),
            },
        };
        bounded_outcome(response, session.max_result_bytes)
    }

    /// Refreshes one server's tool list and atomically replaces its registry entries.
    ///
    /// # Errors
    /// Returns a sanitized lookup, schema, or transport error.
    pub async fn refresh(&self, server: &ServerName) -> Result<(), McpError> {
        let session = match self.servers.read().await.get(server).cloned() {
            Some(session) => session,
            None => return Err(McpError::NotConnected),
        };
        let replacement = timeout_result(
            session.spec.startup_timeout,
            list_server_tools(&session.service, &session.spec),
        )
        .await??;
        let servers = self.servers.read().await;
        if !servers
            .get(server)
            .is_some_and(|current| Arc::ptr_eq(current, &session))
            || session.session_cancel.is_cancelled()
        {
            return Err(McpError::NotConnected);
        }
        let mut tool_map = self.tools.write().await;
        let mut replacement_map = BTreeMap::new();
        for tool in replacement {
            if tool_map.contains_key(&tool.id) && tool.id.server != *server {
                return Err(McpError::InvalidSpec);
            }
            if replacement_map.insert(tool.id.clone(), tool).is_some() {
                return Err(McpError::InvalidSpec);
            }
        }
        tool_map.retain(|id, _tool| id.server != *server);
        tool_map.extend(replacement_map);
        Ok(())
    }

    /// Closes every connected service and confirms each stdio sandbox teardown.
    ///
    /// # Errors
    /// Returns the first sanitized cleanup error after attempting all services.
    pub async fn shutdown(&self) -> Result<(), McpError> {
        let server_names = self
            .servers
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for server in server_names {
            if let Err(error) = self.disconnect(&server).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Cancels requests, closes the rmcp service, and confirms stdio sandbox teardown.
    ///
    /// # Errors
    /// Returns a sanitized cleanup error if the sandbox or bridge cannot be confirmed stopped.
    pub async fn disconnect(&self, server: &ServerName) -> Result<(), McpError> {
        let session = self
            .servers
            .write()
            .await
            .remove(server)
            .ok_or(McpError::NotConnected)?;
        self.tools
            .write()
            .await
            .retain(|id, _tool| id.server != *server);
        session.shutdown().await
    }

    async fn connect_service(
        &self,
        spec: &McpServerSpec,
    ) -> Result<(StdioService, Option<StdioLifecycle>), McpError> {
        match &spec.transport {
            McpTransport::Stdio { .. } => {
                let launched = self.launcher.launch(spec).await?;
                let LaunchedStdioParts {
                    reader,
                    writer,
                    lifecycle,
                } = LaunchedStdioParts::from(launched);
                let transport = AsyncRwTransport::<RoleClient, _, _>::new(reader, writer);
                match timeout_result(spec.startup_timeout, ().serve(transport)).await {
                    Ok(Ok(service)) => Ok((service, Some(lifecycle))),
                    Ok(Err(_)) => {
                        shutdown_lifecycle(Some(lifecycle)).await?;
                        Err(McpError::Handshake)
                    }
                    Err(error) => {
                        shutdown_lifecycle(Some(lifecycle)).await?;
                        Err(error)
                    }
                }
            }
            McpTransport::StreamableHttp { .. } => {
                let transport = self.http_transport(spec)?;
                match timeout_result(spec.startup_timeout, ().serve(transport)).await {
                    Ok(Ok(service)) => Ok((service, None)),
                    Ok(Err(_error)) => Err(McpError::Handshake),
                    Err(error) => Err(error),
                }
            }
        }
    }

    fn http_transport(
        &self,
        spec: &McpServerSpec,
    ) -> Result<StreamableHttpClientTransport<BoundedHttpClient>, McpError> {
        let McpTransport::StreamableHttp { url, headers } = &spec.transport else {
            return Err(McpError::InvalidSpec);
        };
        let mut custom_headers = HeaderMap::new();
        for (name, secret_ref) in headers {
            let header_name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| McpError::InvalidSpec)?;
            if custom_headers.contains_key(&header_name) {
                return Err(McpError::InvalidSpec);
            }
            let location = secret_ref.location().map_err(|_| McpError::InvalidSpec)?;
            let secret = self
                .secret_source
                .load(&location)
                .map_err(|_| McpError::SecretUnavailable)?
                .ok_or(McpError::SecretUnavailable)?;
            let header_value =
                HeaderValue::from_str(secret.expose_secret()).map_err(|_| McpError::InvalidSpec)?;
            custom_headers.insert(header_name, header_value);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(spec.call_timeout)
            .build()
            .map_err(|_| McpError::Transport)?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.as_str().to_owned());
        config.custom_headers = custom_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        config.max_sse_event_size = MAX_JSONRPC_FRAME_BYTES;
        config.max_concurrent_requests = MAX_HTTP_REQUESTS;
        Ok(StreamableHttpClientTransport::with_client(
            BoundedHttpClient::new(client),
            config,
        ))
    }
}

struct LaunchedStdioParts {
    reader: BoundedFrameReader<tokio::io::ReadHalf<DuplexStream>>,
    writer: tokio::io::WriteHalf<DuplexStream>,
    lifecycle: StdioLifecycle,
}

impl From<crate::launcher::LaunchedStdio> for LaunchedStdioParts {
    fn from(launched: crate::launcher::LaunchedStdio) -> Self {
        Self {
            reader: launched.reader,
            writer: launched.writer,
            lifecycle: launched.lifecycle,
        }
    }
}

struct ServerSession {
    service: Arc<StdioService>,
    call_timeout: Duration,
    max_result_bytes: usize,
    session_cancel: CancellationToken,
    service_cancel: Mutex<Option<RunningServiceCancellationToken>>,
    lifecycle: tokio::sync::Mutex<Option<StdioLifecycle>>,
    spec: McpServerSpec,
}

impl std::fmt::Debug for ServerSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerSession")
            .field("call_timeout", &self.call_timeout)
            .field("max_result_bytes", &self.max_result_bytes)
            .finish_non_exhaustive()
    }
}

impl ServerSession {
    async fn shutdown(&self) -> Result<(), McpError> {
        self.session_cancel.cancel();
        let service_cancel = match self.service_cancel.lock() {
            Ok(mut guard) => guard.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(token) = service_cancel {
            token.cancel();
        }
        let lifecycle = self.lifecycle.lock().await.take();
        shutdown_lifecycle(lifecycle).await
    }
}

async fn shutdown_lifecycle(lifecycle: Option<StdioLifecycle>) -> Result<(), McpError> {
    if let Some(mut lifecycle) = lifecycle {
        lifecycle.shutdown().await
    } else {
        Ok(())
    }
}

async fn cleanup_failed_service(
    service: &mut StdioService,
    lifecycle: &mut Option<StdioLifecycle>,
) -> Result<(), McpError> {
    let close_result = service.close().await;
    let lifecycle_result = shutdown_lifecycle(lifecycle.take()).await;
    match (close_result, lifecycle_result) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(_), Ok(())) | (Ok(_), Err(_)) | (Err(_), Err(_)) => Err(McpError::Spawn),
    }
}

async fn timeout_result<T>(
    duration: Duration,
    future: impl std::future::Future<Output = T>,
) -> Result<T, McpError> {
    match tokio::time::timeout(duration, future).await {
        Ok(value) => Ok(value),
        Err(_) => Err(McpError::Timeout),
    }
}

fn validate_spec(spec: &McpServerSpec) -> Result<(), McpError> {
    if spec.startup_timeout.is_zero()
        || spec.startup_timeout > Duration::from_secs(120)
        || spec.call_timeout.is_zero()
        || spec.call_timeout > Duration::from_secs(120)
        || spec.max_result_bytes == 0
        || spec.max_result_bytes > MAX_JSONRPC_FRAME_BYTES
    {
        return Err(McpError::InvalidSpec);
    }
    match &spec.transport {
        McpTransport::Stdio { command, env, .. } => {
            if command.is_empty()
                || command
                    .iter()
                    .any(|part| part.chars().any(char::is_control))
                || env.iter().any(|(name, _)| {
                    name.is_empty()
                        || name.len() > 64
                        || !name.bytes().all(|byte| {
                            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                })
            {
                return Err(McpError::InvalidSpec);
            }
        }
        McpTransport::StreamableHttp { url, .. } => {
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(McpError::InvalidSpec);
            }
            if let McpTransport::StreamableHttp { headers, .. } = &spec.transport {
                if url.scheme() == "http" && !is_loopback_host(url.host_str()) {
                    return Err(McpError::InvalidSpec);
                }
                let mut seen = HashSet::new();
                for (name, _secret_ref) in headers {
                    let header_name = HeaderName::from_bytes(name.as_bytes())
                        .map_err(|_| McpError::InvalidSpec)?;
                    if is_reserved_header(&header_name) || !seen.insert(header_name) {
                        return Err(McpError::InvalidSpec);
                    }
                }
            }
        }
    }
    Ok(())
}

fn is_loopback_host(host: Option<&str>) -> bool {
    match host {
        Some(value) if value.eq_ignore_ascii_case("localhost") => true,
        Some(value) => match value.parse::<std::net::IpAddr>() {
            Ok(address) => address.is_loopback(),
            Err(_) => false,
        },
        None => false,
    }
}

async fn list_server_tools(
    service: &StdioService,
    spec: &McpServerSpec,
) -> Result<Vec<ToolSpec>, McpError> {
    let mut tools = Vec::new();
    let mut cursor = None;
    let mut complete = false;
    for _page_index in 0..MAX_TOOL_LIST_PAGES {
        let page = service
            .peer()
            .list_tools(Some(PaginatedRequestParams::default().with_cursor(cursor)))
            .await
            .map_err(|_| McpError::Transport)?;
        for tool in page.tools {
            if spec
                .tool_allowlist
                .as_ref()
                .is_some_and(|allowed| !allowed.iter().any(|name| name == tool.name.as_ref()))
            {
                continue;
            }
            if tools.len() >= MAX_TOOL_LIST_ENTRIES {
                return Err(McpError::InvalidSpec);
            }
            tools.push(normalize_tool(&spec.name, tool)?);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(McpError::InvalidSpec);
    }
    Ok(tools)
}

fn normalize_tool(server: &ServerName, tool: Tool) -> Result<ToolSpec, McpError> {
    let id = NamespacedToolId::new(server.clone(), tool.name.as_ref().to_owned())?;
    let schema = serde_json::Value::Object(tool.input_schema.as_ref().clone());
    let normalized = normalize_schema(&schema)?;
    let description = match tool.description {
        Some(description) => description.into_owned(),
        None => String::new(),
    };
    let prune_summary = prune_summary(&description, MAX_SUMMARY_CHARS);
    Ok(ToolSpec {
        id,
        description,
        input_schema: normalized.schema,
        prune_summary,
        effects: ToolEffect::CONSERVATIVE,
    })
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ext/crates/mcp/src/hub.rs"]
mod validation_tests;
