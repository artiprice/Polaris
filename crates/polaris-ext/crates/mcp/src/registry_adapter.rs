//! Adapter from the bounded MCP hub to the SDK's generic MCP registry port.

use std::sync::Arc;

use polaris_sdk::registry::{McpToolClient, McpToolClientError, McpToolDefinition, McpToolResult};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{EffectAuthorizer, McpError, McpHub, ServerName, ToolContent};

const MAX_PORT_PAYLOAD_BYTES: usize = 1_048_576;

/// Presents one connected MCP server through the SDK port while preserving its effect authorizer.
pub struct McpHubToolClient<A: EffectAuthorizer + ?Sized> {
    hub: Arc<McpHub>,
    server: ServerName,
    authorizer: Arc<A>,
}

impl<A: EffectAuthorizer + ?Sized> std::fmt::Debug for McpHubToolClient<A> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpHubToolClient")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl<A: EffectAuthorizer + ?Sized> McpHubToolClient<A> {
    /// Creates an adapter for a configured MCP hub and its caller-supplied authorizer.
    #[must_use]
    pub fn new(hub: Arc<McpHub>, server: ServerName, authorizer: Arc<A>) -> Self {
        Self {
            hub,
            server,
            authorizer,
        }
    }

    /// Connects and owns one MCP server session in the supplied hub.
    ///
    /// Process-based servers launch only through the hub's injected SDK-backed `ProcessLauncher`.
    ///
    /// # Errors
    /// Returns a sanitized setup error from MCP transport, trust, schema, or lifecycle validation.
    pub async fn connect(
        hub: Arc<McpHub>,
        spec: crate::McpServerSpec,
        authorizer: Arc<A>,
    ) -> Result<Self, McpError> {
        let server = spec.name.clone();
        hub.connect(spec).await?;
        Ok(Self::new(hub, server, authorizer))
    }

    /// Disconnects the configured server and awaits owned transport/process cleanup.
    ///
    /// # Errors
    /// Returns a sanitized cleanup error when the server cannot be shut down cleanly.
    pub async fn shutdown(&self) -> Result<(), McpError> {
        self.hub.disconnect(&self.server).await
    }
}

impl<A: EffectAuthorizer + ?Sized> McpToolClient for McpHubToolClient<A> {
    async fn list_tools(&self, server: &str) -> Result<Vec<McpToolDefinition>, McpToolClientError> {
        if server != self.server.as_str() {
            return Err(McpToolClientError::Unavailable);
        }
        let tools = self.hub.tools().await;
        let mut definitions = Vec::new();
        for tool in tools
            .into_iter()
            .filter(|tool| tool.id.server == self.server)
        {
            let input_schema_json =
                serde_json::to_vec(&tool.input_schema).map_err(|_| McpToolClientError::Protocol)?;
            if input_schema_json.len() > MAX_PORT_PAYLOAD_BYTES {
                return Err(McpToolClientError::Protocol);
            }
            definitions.push(McpToolDefinition {
                name: tool.id.tool,
                input_schema_json,
            });
        }
        Ok(definitions)
    }

    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments_json: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<McpToolResult, McpToolClientError> {
        if server != self.server.as_str() || arguments_json.len() > MAX_PORT_PAYLOAD_BYTES {
            return Err(McpToolClientError::Protocol);
        }
        let arguments: Value =
            serde_json::from_slice(arguments_json).map_err(|_| McpToolClientError::Protocol)?;
        let id = crate::NamespacedToolId::new(self.server.clone(), tool)
            .map_err(|_| McpToolClientError::Unavailable)?;
        let outcome = self
            .hub
            .call(&id, arguments, cancellation, self.authorizer.as_ref())
            .await
            .map_err(map_error)?;
        let content: Vec<Value> = outcome
            .content
            .into_iter()
            .map(|item| match item {
                ToolContent::Text(text) => json!({"text": text}),
                ToolContent::Structured(value) => json!({"structured": value}),
            })
            .collect();
        let result_json = serde_json::to_vec(&json!({
            "content": content,
            "is_error": outcome.is_error,
            "truncated": outcome.truncated,
        }))
        .map_err(|_| McpToolClientError::Protocol)?;
        if result_json.len() > MAX_PORT_PAYLOAD_BYTES {
            return Err(McpToolClientError::Protocol);
        }
        Ok(McpToolResult {
            result_json,
            is_error: outcome.is_error,
        })
    }
}

fn map_error(error: McpError) -> McpToolClientError {
    match error {
        McpError::Timeout => McpToolClientError::Timeout,
        McpError::Cancelled => McpToolClientError::Cancelled,
        McpError::UnauthorizedEffects | McpError::Denied(_) => McpToolClientError::Denied,
        McpError::ResultTooLarge | McpError::SchemaInvalid | McpError::InvalidSpec => {
            McpToolClientError::Protocol
        }
        McpError::Spawn
        | McpError::Handshake
        | McpError::VersionMismatch
        | McpError::ToolNotFound(_)
        | McpError::SecretUnavailable
        | McpError::NotConnected
        | McpError::Transport => McpToolClientError::Unavailable,
    }
}
