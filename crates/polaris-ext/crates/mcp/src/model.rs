use std::{fmt, path::PathBuf, time::Duration};

use polaris_config::SecretRef;
use serde_json::Value;

use crate::error::McpError;

/// Validated MCP server namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerName(String);

impl ServerName {
    /// Creates a bounded namespace using ASCII letters, digits, dot, underscore, and hyphen.
    ///
    /// # Errors
    /// Returns [`McpError::InvalidSpec`] for an empty, oversized, or malformed name.
    pub fn new(value: impl Into<String>) -> Result<Self, McpError> {
        let name = value.into();
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(McpError::InvalidSpec);
        }
        Ok(Self(name))
    }

    /// Returns the stable display form of this namespace.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServerName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Collision-safe identifier for a remote tool.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamespacedToolId {
    /// Owning server namespace.
    pub server: ServerName,
    /// Original remote tool name.
    pub tool: String,
}

impl NamespacedToolId {
    /// Creates an identifier from a validated server and bounded tool name.
    ///
    /// # Errors
    /// Returns [`McpError::InvalidSpec`] for an empty, oversized, or control-containing tool name.
    pub fn new(server: ServerName, tool: impl Into<String>) -> Result<Self, McpError> {
        let tool_name = tool.into();
        if tool_name.is_empty() || tool_name.len() > 256 || tool_name.chars().any(char::is_control)
        {
            return Err(McpError::InvalidSpec);
        }
        Ok(Self {
            server,
            tool: tool_name,
        })
    }
}

impl fmt::Display for NamespacedToolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}::{}", self.server, self.tool)
    }
}

/// Trust category of an MCP server specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustLevel {
    /// Explicitly configured by the user outside a repository.
    UserGlobal,
    /// Repository configuration that the user explicitly approved.
    RepoApproved,
    /// Repository configuration that has not been approved.
    RepoUnapproved,
}

/// Supported MCP transport.
#[derive(Clone, PartialEq, Eq)]
pub enum McpTransport {
    /// MCP over a sandbox-supervised stdio child.
    Stdio {
        /// Executable and arguments. The command is passed only to the injected sandbox launcher.
        command: Vec<String>,
        /// Optional working directory inside the sandbox.
        cwd: Option<PathBuf>,
        /// Explicit secret references mapped to child environment variable names.
        env: Vec<(String, SecretRef)>,
    },
    /// MCP streamable HTTP transport.
    StreamableHttp {
        /// HTTP or HTTPS endpoint URL.
        url: url::Url,
        /// Secret references mapped to HTTP header names.
        headers: Vec<(String, SecretRef)>,
    },
}

impl fmt::Debug for McpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stdio { command, cwd, env } => formatter
                .debug_struct("Stdio")
                .field("argument_count", &command.len())
                .field("cwd", cwd)
                .field(
                    "environment_names",
                    &env.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                )
                .finish(),
            Self::StreamableHttp { url, headers } => formatter
                .debug_struct("StreamableHttp")
                .field("scheme", &url.scheme())
                .field("host", &url.host_str())
                .field(
                    "header_names",
                    &headers.iter().map(|(name, _)| name).collect::<Vec<_>>(),
                )
                .finish(),
        }
    }
}

/// One configured MCP server and its resource limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    /// Stable server namespace.
    pub name: ServerName,
    /// Transport configuration.
    pub transport: McpTransport,
    /// User or repository trust classification.
    pub trust: TrustLevel,
    /// Maximum time allowed for MCP initialization.
    pub startup_timeout: Duration,
    /// Maximum time allowed for each remote request.
    pub call_timeout: Duration,
    /// Optional allowlist of exact remote tool names.
    pub tool_allowlist: Option<Vec<String>>,
    /// Maximum UTF-8 bytes returned from a tool.
    pub max_result_bytes: usize,
}

/// Conservative declared effects for every untrusted remote tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToolEffect {
    /// The tool may read local or remote data.
    pub may_read: bool,
    /// The tool may mutate state.
    pub may_write: bool,
    /// The tool may execute code or commands.
    pub may_execute: bool,
    /// The tool may contact network services.
    pub may_contact_network: bool,
    /// The tool's actual effects are unverified.
    pub unknown: bool,
}

impl ToolEffect {
    /// Effects assigned to untrusted MCP tools regardless of remote annotations.
    pub const CONSERVATIVE: Self = Self {
        may_read: true,
        may_write: true,
        may_execute: true,
        may_contact_network: true,
        unknown: true,
    };
}

/// Metadata registered for one namespaced remote tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Collision-safe tool identifier.
    pub id: NamespacedToolId,
    /// Untrusted remote description.
    pub description: String,
    /// Validated, normalized input schema.
    pub input_schema: Value,
    /// Sanitized bounded text intended for S1 pruning.
    pub prune_summary: String,
    /// Conservative effects; remote annotations cannot lower these effects.
    pub effects: ToolEffect,
}

/// Explicit caller authorization seam invoked before a remote effect is requested.
pub trait EffectAuthorizer: Send + Sync {
    /// Decides whether the caller permits this tool's conservative effects.
    fn authorize(&self, tool: &ToolSpec) -> EffectAuthorization;
}

/// Result of the caller's effect authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectAuthorization {
    /// Caller permits the declared conservative effects for this invocation.
    Allow,
    /// Caller refuses the invocation.
    Deny,
}

/// A content item returned by an MCP tool.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolContent {
    /// Text supplied by the remote server.
    Text(String),
    /// Structured JSON supplied by the remote server.
    Structured(Value),
}

/// Bounded result returned from an MCP tool invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    /// Remote result content after byte bounding.
    pub content: Vec<ToolContent>,
    /// True if the MCP server marked the result as an error.
    pub is_error: bool,
    /// True if any content was truncated to enforce the configured byte limit.
    pub truncated: bool,
}
