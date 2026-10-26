use crate::model::{NamespacedToolId, TrustLevel};

/// Sanitized failures from MCP configuration, transport, policy, or remote calls.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum McpError {
    /// Sandbox process creation or cleanup failed.
    #[error("MCP sandbox process operation failed")]
    Spawn,
    /// MCP initialization did not complete successfully.
    #[error("MCP initialization failed")]
    Handshake,
    /// The negotiated protocol version is unsupported by this client.
    #[error("MCP protocol version mismatch")]
    VersionMismatch,
    /// The requested namespaced tool is absent or no longer available.
    #[error("MCP tool not found: {0}")]
    ToolNotFound(NamespacedToolId),
    /// A remote tool input schema is malformed or outside supported limits.
    #[error("MCP tool schema is invalid or unsupported")]
    SchemaInvalid,
    /// An MCP request exceeded its configured deadline.
    #[error("MCP request timed out")]
    Timeout,
    /// An MCP request was cancelled by its caller.
    #[error("MCP request cancelled")]
    Cancelled,
    /// The remote tool response exceeded the configured result limit.
    #[error("MCP tool result exceeds the configured limit")]
    ResultTooLarge,
    /// The configured server does not have sufficient trust to connect.
    #[error("MCP server denied by trust policy: {0:?}")]
    Denied(TrustLevel),
    /// Caller authorization did not permit the conservative effects of this tool.
    #[error("MCP tool effects were not authorized")]
    UnauthorizedEffects,
    /// The configured secret reference could not be resolved.
    #[error("MCP transport credential unavailable")]
    SecretUnavailable,
    /// A configured endpoint, tool, or server name is invalid.
    #[error("MCP configuration is invalid")]
    InvalidSpec,
    /// The requested server is not connected.
    #[error("MCP server is not connected")]
    NotConnected,
    /// Transport failed; underlying diagnostics are intentionally omitted.
    #[error("MCP transport failed")]
    Transport,
}
