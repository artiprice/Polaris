//! # polaris-ext-mcp
//!
//! Production MCP client adapters for supervised sandbox stdio processes and streamable HTTP.
//! Tool metadata is untrusted, namespaced by server, bounded, sanitized, and assigned conservative
//! effects. Every remote invocation passes through caller-supplied effect authorization.

#![forbid(unsafe_code)]
#![forbid(clippy::unwrap_used)]
#![forbid(clippy::expect_used)]
#![forbid(clippy::panic)]
#![forbid(clippy::indexing_slicing)]
#![forbid(clippy::as_conversions)]

mod call;
mod error;
mod frame;
mod http_client;
mod hub;
mod launcher;
mod model;
mod schema;
mod summary;

pub use error::McpError;
pub use frame::{BoundedFrameReader, MAX_JSONRPC_FRAME_BYTES};
pub use hub::McpHub;
pub use launcher::{LaunchFuture, LaunchedStdio, ProcessLauncher, SandboxProcessLauncher};
pub use model::{
    EffectAuthorization, EffectAuthorizer, McpServerSpec, McpTransport, NamespacedToolId,
    ServerName, ToolContent, ToolEffect, ToolOutcome, ToolSpec, TrustLevel,
};
pub use schema::{NormalizedSchema, normalize_schema};
pub use summary::prune_summary;
pub use tokio_util::sync::CancellationToken as CancelToken;
