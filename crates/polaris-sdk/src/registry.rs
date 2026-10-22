//! Internal API registry (prompt.md §6.2): one trait per external concern. Business logic depends on
//! these traits only; concrete adapters live in L2 crates and are wired at the composition root.
//!
//! Async methods use return-position `impl Future + Send` (no `dyn`, no boxing). Every error enum
//! implements [`HasKind`].

use core::future::Future;
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::contracts::{Command, CommandId, EventRecord, Unverified};
use crate::effort::Effort;
use crate::error::{ErrorKind, HasKind, ParseError};
use crate::ids::{Cid, FrameId, ModelId, Seq, SessionId, TokenCount};
use crate::system_one::{Decision, DecisionInput, Question};

// ----------------------------------------------------------------------------------------- tools

/// Side effects a tool may have; used for permission prompts, sandbox routing and tool pruning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Effects {
    /// Reads workspace files.
    pub reads_workspace: bool,
    /// Proposes workspace changes (always through `ProposeDiff`, never a direct write).
    pub writes_workspace: bool,
    /// Uses the network.
    pub network: bool,
    /// Executes processes (always inside a sandbox backend).
    pub executes: bool,
}

impl Effects {
    /// No side effects at all (pure computation).
    pub const PURE: Self = Self {
        reads_workspace: false,
        writes_workspace: false,
        network: false,
        executes: false,
    };
    /// Reads the workspace only.
    pub const READ_ONLY: Self = Self {
        reads_workspace: true,
        writes_workspace: false,
        network: false,
        executes: false,
    };
}

/// What the model sees about a tool. Part of the cached prompt prefix, so it must be stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique tool name (`snake_case`).
    pub name: Box<str>,
    /// One-paragraph description for the model.
    pub description: Box<str>,
    /// JSON Schema of the input, as JSON text.
    pub input_schema_json: Box<str>,
    /// Declared side effects.
    pub effects: Effects,
}

/// Per-call context handed to a tool.
#[derive(Debug, Clone, Copy)]
pub struct ToolContext<'a> {
    /// Calling session.
    pub session: SessionId,
    /// Calling frame.
    pub frame: FrameId,
    /// Workspace root every path is resolved against.
    pub workspace_root: &'a Path,
    /// Remaining time budget, if any.
    pub deadline: Option<Duration>,
}

/// A typed tool. Registries dispatch by [`Tool::NAME`]; inputs arrive as JSON.
pub trait Tool: Send + Sync {
    /// Unique tool name.
    const NAME: &'static str;
    /// Declared side effects.
    const EFFECTS: Effects;
    /// Parsed input.
    type Input: DeserializeOwned + Send;
    /// Result returned to the model.
    type Output: Serialize + Send;

    /// The model-facing description of this tool.
    fn spec(&self) -> ToolSpec;

    /// Runs the tool.
    fn call(
        &self,
        input: Self::Input,
        ctx: &ToolContext<'_>,
    ) -> impl Future<Output = Result<Self::Output, ToolError>> + Send;
}

/// Tool failures.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Input did not match the schema or failed validation.
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    /// Refused by a guard (path outside the workspace, denied effect, missing capability).
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// A referenced file, symbol or tool does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The deadline passed.
    #[error("tool timed out")]
    Timeout,
    /// Local I/O failed.
    #[error("tool I/O error: {0}")]
    Io(String),
    /// The sandbox backend failed.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
}

impl HasKind for ToolError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidInput(_) | Self::NotFound(_) => ErrorKind::Permanent,
            Self::Forbidden(_) => ErrorKind::Policy,
            Self::Timeout | Self::Io(_) => ErrorKind::Transient,
            Self::Sandbox(e) => e.kind(),
        }
    }
}

// ------------------------------------------------------------------------------------------ chat

/// Author of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    /// System prompt.
    System,
    /// User turn.
    User,
    /// Model turn.
    Assistant,
    /// Tool result.
    Tool,
}

/// One actual model-issued function call, replayed in assistant history before its tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatToolCall {
    /// Provider call id used by exactly one corresponding tool-result message.
    pub id: String,
    /// Registered tool name.
    pub name: String,
    /// Complete JSON argument text; the provider adapter validates it before transmission.
    pub arguments_json: String,
}

/// One chat message. Structured tool metadata prevents tool results from being replayed as
/// unaffiliated text. Older serialized text-only messages deserialize with empty metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Author.
    pub role: ChatRole,
    /// Text content.
    pub content: String,
    /// Assistant function calls; empty for other roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatToolCall>,
    /// Identity of the preceding assistant function call for a tool-role result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Provider reasoning history, isolated from user-visible content. Adapters send it only
    /// when that provider's explicit replay policy supports the reasoning field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

impl ChatMessage {
    /// Construct a text-only message with no tool or reasoning metadata.
    #[must_use]
    pub fn text(role: ChatRole, content: String) -> Self {
        Self {
            role,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning: None,
        }
    }

    /// Construct one explicit tool-result message. The adapter validates history linkage.
    #[must_use]
    pub fn tool_result(call_id: String, content: String) -> Self {
        Self {
            role: ChatRole::Tool,
            content,
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id),
            reasoning: None,
        }
    }

    /// Construct an assistant tool-call turn with the exact complete calls and optional isolated
    /// reasoning channel. This stores data and does not authorize or execute any call.
    #[must_use]
    pub fn assistant_calls(
        content: String,
        calls: Vec<ChatToolCall>,
        reasoning: Option<String>,
    ) -> Self {
        Self {
            role: ChatRole::Assistant,
            content,
            tool_calls: calls,
            tool_call_id: None,
            reasoning,
        }
    }
}

/// A chat completion request (OpenAI-compatible semantics).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Model to call.
    pub model: ModelId,
    /// Conversation, stable prefix first.
    pub messages: Vec<ChatMessage>,
    /// Tools offered to the model (stable order).
    pub tools: Vec<ToolSpec>,
    /// Output token cap.
    pub max_output_tokens: Option<TokenCount>,
    /// Effort level (maps to reasoning controls where the model supports them).
    pub effort: Effort,
}

/// Token usage reported by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens.
    pub prompt_tokens: TokenCount,
    /// Completion tokens.
    pub completion_tokens: TokenCount,
    /// Prompt tokens served from the provider's prefix cache (0 if not reported).
    pub cached_tokens: TokenCount,
}

/// Why generation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Natural end.
    Stop,
    /// Output cap reached.
    Length,
    /// The model requested tool calls.
    ToolCalls,
    /// Provider-side filtering.
    ContentFilter,
    /// Any other provider reason.
    Other,
}

/// One streamed piece of a completion. Adjacently tagged (`{"type":…,"data":…}`): internally
/// tagged enums cannot carry newtype variants such as `Text(String)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ChatChunk {
    /// Assistant text delta.
    Text(String),
    /// Provider-emitted reasoning text, kept separate from user-visible assistant output.
    /// This is provider data, never an instruction or evidence that an action is authorized.
    Reasoning(String),
    /// Tool-call delta; fragments with the same `index` concatenate.
    ToolCallDelta {
        /// Position of the call in this response.
        index: u32,
        /// Call id (first fragment only).
        id: Option<String>,
        /// Tool name (first fragment only).
        name: Option<String>,
        /// JSON arguments fragment.
        arguments_fragment: String,
    },
    /// Final usage report.
    Usage(Usage),
    /// End of the completion.
    Finished(FinishReason),
}

/// A pull-based stream of chat chunks (`None` = end of stream).
pub trait ChatChunkSource: Send {
    /// The next chunk, an error, or `None` when the stream ended.
    fn next_chunk(&mut self) -> impl Future<Output = Option<Result<ChatChunk, ModelError>>> + Send;
}

/// A chat model (Token Factory, a local OpenAI-compatible server, …).
pub trait ChatModel: Send + Sync {
    /// The chunk stream this model returns.
    type Stream: ChatChunkSource;

    /// Starts a streaming completion.
    fn stream(
        &self,
        req: ChatRequest,
    ) -> impl Future<Output = Result<Self::Stream, ModelError>> + Send;
}

/// Model-provider failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// Connection-level failure.
    #[error("transport error: {0}")]
    Transport(String),
    /// The request timed out.
    #[error("model request timed out")]
    Timeout,
    /// HTTP 429; honour `retry_after_ms` when present.
    #[error("rate limited (retry after {retry_after_ms:?} ms)")]
    RateLimited {
        /// Server-provided retry delay.
        retry_after_ms: Option<u64>,
    },
    /// The provider rejected the request as malformed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Missing or invalid credentials.
    #[error("unauthorized")]
    Unauthorized,
    /// Any other HTTP status.
    #[error("upstream status {status}")]
    Upstream {
        /// HTTP status code.
        status: u16,
    },
    /// The stream or body did not follow the protocol.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl HasKind for ModelError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Transport(_) | Self::Timeout | Self::RateLimited { .. } => ErrorKind::Transient,
            Self::Upstream { status } if *status == 429 || *status >= 500 => ErrorKind::Transient,
            Self::Upstream { .. }
            | Self::BadRequest(_)
            | Self::Unauthorized
            | Self::Protocol(_) => ErrorKind::Permanent,
        }
    }
}

// ------------------------------------------------------------------------------------ embeddings

/// Text embedding provider.
pub trait EmbeddingProvider: Send + Sync {
    /// Dimension of every returned vector.
    fn dimension(&self) -> usize;

    /// Embeds a batch (inputs must already be PII-masked).
    fn embed(
        &self,
        batch: &[&str],
    ) -> impl Future<Output = Result<Vec<Vec<f32>>, ModelError>> + Send;
}

// -------------------------------------------------------------------------------------- decision

/// System-One decision engine (GLiNER2 worker).
pub trait DecisionEngine: Send + Sync {
    /// Answers every question of `schema` about `input` in one batched call.
    fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> impl Future<Output = Result<Decision, DecisionError>> + Send;
}

/// Decision-engine failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecisionError {
    /// The worker is down or restarting.
    #[error("decision engine unavailable: {0}")]
    Unavailable(String),
    /// The call exceeded its latency budget.
    #[error("decision timed out")]
    Timeout,
    /// The schema is invalid for this engine.
    #[error("invalid schema: {0}")]
    InvalidSchema(String),
    /// The model failed on this input.
    #[error("model failure: {0}")]
    Model(String),
}

impl HasKind for DecisionError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable(_) | Self::Timeout => ErrorKind::Transient,
            Self::InvalidSchema(_) | Self::Model(_) => ErrorKind::Permanent,
        }
    }
}

// --------------------------------------------------------------------------------------- sandbox

/// What sandbox to create.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SandboxSpec {
    /// Requested name (backend-generated when `None`).
    pub name: Option<String>,
    /// Image reference (backend default when `None`).
    pub image: Option<String>,
    /// CPU limit in millicores.
    pub cpu_millis: Option<u32>,
    /// Memory limit in MiB.
    pub memory_mib: Option<u32>,
    /// Policy document in CAS (backend default when `None`).
    pub policy: Option<Cid>,
}

/// Backend-assigned sandbox name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SandboxHandle(Box<str>);

impl SandboxHandle {
    /// Validates a sandbox name: non-empty, at most 128 bytes, ASCII alphanumerics and `-_.`.
    ///
    /// # Errors
    /// [`ParseError::InvalidValue`] when a rule is violated.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        let what = "sandbox handle";
        if value.is_empty() {
            return Err(ParseError::InvalidValue {
                what,
                reason: "empty",
            });
        }
        if value.len() > 128 {
            return Err(ParseError::InvalidValue {
                what,
                reason: "too long",
            });
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(ParseError::InvalidValue {
                what,
                reason: "contains a disallowed character",
            });
        }
        Ok(Self(value.into()))
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SandboxHandle {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<SandboxHandle> for String {
    fn from(value: SandboxHandle) -> Self {
        value.0.into_string()
    }
}

/// A command to run inside a sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecRequest {
    /// Program and arguments (no shell unless argv starts one).
    pub argv: Vec<String>,
    /// Working directory inside the sandbox.
    pub workdir: Option<String>,
    /// Bytes piped to stdin (keep each call below the backend's request limit).
    pub stdin: Option<Bytes>,
    /// Extra environment variables (never secrets: credentials go through provider profiles).
    pub env: Vec<(String, String)>,
    /// Timeout for the command.
    pub timeout: Option<Duration>,
}

/// Result of a sandboxed command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecOutput {
    /// Exit code (`-1` when killed by a signal).
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: Bytes,
    /// Captured stderr.
    pub stderr: Bytes,
    /// Wall time.
    pub duration: Duration,
}

/// An isolated execution environment (OpenShell, Contree, remote gateway).
///
/// Adapters must bound download bytes and list entries while receiving the transport response,
/// before allocating an unbounded result. Transport-specific limits must be documented.
pub trait SandboxBackend: Send + Sync {
    /// Creates a sandbox and waits until it is ready.
    fn create(
        &self,
        spec: &SandboxSpec,
    ) -> impl Future<Output = Result<SandboxHandle, SandboxError>> + Send;
    /// Runs a command.
    fn exec(
        &self,
        sandbox: &SandboxHandle,
        req: &ExecRequest,
    ) -> impl Future<Output = Result<ExecOutput, SandboxError>> + Send;
    /// Writes a file inside the sandbox.
    fn upload(
        &self,
        sandbox: &SandboxHandle,
        path: &str,
        bytes: Bytes,
    ) -> impl Future<Output = Result<(), SandboxError>> + Send;
    /// Reads a file from the sandbox.
    fn download(
        &self,
        sandbox: &SandboxHandle,
        path: &str,
    ) -> impl Future<Output = Result<Bytes, SandboxError>> + Send;
    /// Lists file paths under a directory inside the sandbox.
    fn list_files(
        &self,
        sandbox: &SandboxHandle,
        dir: &str,
    ) -> impl Future<Output = Result<Vec<String>, SandboxError>> + Send;
    /// Reads one page from a bounded immutable remote manifest snapshot. When `generation` is
    /// `None`, the backend creates a snapshot and requires `offset == 0`; later pages must pass
    /// the returned generation token. Implementations fail closed when the generation expires.
    fn snapshot_page(
        &self,
        _sandbox: &SandboxHandle,
        _root: &str,
        _generation: Option<&str>,
        _offset: u64,
        _limit: u32,
    ) -> impl Future<Output = Result<SandboxSnapshotPage, SandboxError>> + Send {
        async {
            Err(SandboxError::PolicyDenied(
                "backend does not support immutable sync snapshots".into(),
            ))
        }
    }
    /// Deletes the sandbox; idempotent.
    fn destroy(
        &self,
        sandbox: &SandboxHandle,
    ) -> impl Future<Output = Result<(), SandboxError>> + Send;
    /// Conditionally commit a staged mutation in the same process that revalidates its expected
    /// state. Implementations serialize cooperating writers using a workspace lock, reject
    /// symlink parents and atomically install on the destination filesystem. No general-purpose
    /// filesystem API can exclude an uncooperative writer between compare and rename; callers
    /// also retain their workspace lease. Unsupported backends fail closed.
    fn conditional_commit(
        &self,
        _sandbox: &SandboxHandle,
        _request: &SandboxCommitRequest,
    ) -> impl Future<Output = Result<CommitOutcome, SandboxError>> + Send {
        async {
            Err(SandboxError::PolicyDenied(
                "backend does not support conditional sync commits".into(),
            ))
        }
    }
    /// Prepare and validate a private sync namespace under the canonical remote workspace.
    /// Existing symlinked, shared or incorrectly owned private directories must be rejected,
    /// never silently chmod-ed. This precedes upload so staged bytes are private from creation.
    fn prepare_sync(
        &self,
        _sandbox: &SandboxHandle,
        _root: &str,
    ) -> impl Future<Output = Result<(), SandboxError>> + Send {
        async {
            Err(SandboxError::PolicyDenied(
                "backend does not support private sync preparation".into(),
            ))
        }
    }
}

/// One exact remote file or internal symbolic link in a captured sync snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSnapshotEntry {
    /// Root-relative slash-separated path.
    pub relative_path: String,
    /// File byte count or link-target byte count.
    pub size: u64,
    /// BLAKE3 hash of exact file bytes or link-target UTF-8 bytes.
    pub hash: Cid,
    /// Executable mode for regular files; false for links.
    pub executable: bool,
    /// Exact internal relative link target, absent for regular files.
    pub link_target: Option<String>,
}

/// One page from an immutable, expiring remote manifest snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSnapshotPage {
    /// Opaque token binding every page to one cached snapshot and root.
    pub generation: String,
    /// Canonical content id of the complete manifest.
    pub manifest_id: Cid,
    /// Total number of entries in the complete manifest.
    pub total_entries: u64,
    /// First entry offset in this page.
    pub offset: u64,
    /// Entries, in lexical path order.
    pub entries: Vec<SandboxSnapshotEntry>,
    /// Next offset, or `None` after the final page.
    pub next_offset: Option<u64>,
}

/// One path mutation carried in a bounded tar sync batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxBatchMutation {
    /// Root-relative path.
    pub relative_path: String,
    /// Exact remote state observed by the planner; `None` means absent.
    pub expected: Option<SandboxFileState>,
    /// Requested file/link write or delete.
    pub action: SandboxBatchAction,
}

/// Action encoded in a sync batch manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxBatchAction {
    /// Conditionally delete the observed file or link.
    Delete,
    /// Install exact regular-file bytes from the named synthetic tar payload.
    File {
        /// Synthetic archive name, never used as a filesystem path.
        payload: String,
        /// BLAKE3 of the exact file bytes.
        hash: Cid,
        /// Whether to install executable mode.
        executable: bool,
    },
    /// Install a non-escaping relative symlink.
    Link {
        /// Exact link target.
        target: String,
        /// BLAKE3 of the target UTF-8 bytes.
        hash: Cid,
    },
}

/// Manifest stored as the first entry in one bounded tar sync batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxSyncBatch {
    /// Batch format version.
    pub version: u32,
    /// Canonical sandbox workspace root.
    pub root: String,
    /// Non-empty, unique path operations.
    pub mutations: Vec<SandboxBatchMutation>,
}

/// Outcome returned by the guest batch commit helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxBatchResult {
    /// Paths atomically installed or deleted.
    pub applied: Vec<String>,
    /// Paths that failed expected-state comparison; the whole batch was rejected before changes.
    pub conflicts: Vec<String>,
    /// Paths not applied because a previous operation in the batch failed.
    pub failed: Vec<String>,
}

/// Exact remote entry identity expected by a conditional sync commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxFileState {
    /// BLAKE3 of file bytes, or of the exact UTF-8 link target.
    pub hash: Cid,
    /// Executable mode for a regular file; false for a link.
    pub executable: bool,
    /// Exact relative target for a link; absent for a regular file.
    pub link_target: Option<String>,
}
/// New state requested by the trusted sync planner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxCommitAction {
    /// Delete only the observed destination.
    Delete,
    /// Verify and install a staged regular file.
    File {
        /// File under `<root>/.polaris/sync/staging/` on the same filesystem.
        staged_path: String,
        /// Expected BLAKE3 of all staged bytes.
        hash: Cid,
        /// Executable mode to install.
        executable: bool,
    },
    /// Install a non-escaping relative symbolic link.
    Link {
        /// Exact link target, never expanded through a shell.
        target: String,
        /// BLAKE3 of the target's UTF-8 bytes.
        hash: Cid,
    },
}
/// Bounded fixed-protocol request for the in-sandbox commit helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxCommitRequest {
    /// Absolute canonical workspace root inside the sandbox.
    pub root: String,
    /// Validated workspace-relative destination; `.git` and `.polaris` are forbidden.
    pub relative_path: String,
    /// Observed destination, or absence.
    pub expected: Option<SandboxFileState>,
    /// Requested atomic change.
    pub action: SandboxCommitAction,
}
/// Conditional commit result. A conflict preserves the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// Expected state matched and the change was committed.
    Applied,
    /// Current state diverged; no destination change was performed.
    Conflict,
}

/// Sandbox failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    /// Gateway or backend unreachable.
    #[error("sandbox backend unavailable: {0}")]
    Unavailable(String),
    /// The operation timed out.
    #[error("sandbox operation timed out")]
    Timeout,
    /// Denied by the sandbox policy (filesystem, process or network rule).
    #[error("denied by sandbox policy: {0}")]
    PolicyDenied(String),
    /// Unknown sandbox or path.
    #[error("not found: {0}")]
    NotFound(String),
    /// A sandbox with the requested name already exists; callers must not delete it as cleanup.
    #[error("sandbox already exists: {0}")]
    AlreadyExists(String),
    /// Quota or size limit exceeded.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// Unexpected backend response.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl HasKind for SandboxError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable(_) | Self::Timeout => ErrorKind::Transient,
            Self::PolicyDenied(_) => ErrorKind::Policy,
            Self::NotFound(_)
            | Self::AlreadyExists(_)
            | Self::LimitExceeded(_)
            | Self::Protocol(_) => ErrorKind::Permanent,
        }
    }
}

// ---------------------------------------------------------------------------------------- search

/// Web search provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    /// Tavily API.
    Tavily,
    /// Local SearXNG instance.
    Searxng,
}

/// A web search query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Query text (PII-masked).
    pub text: String,
    /// Maximum hits.
    pub max_results: u16,
    /// Restrict to a site (`site:` filter).
    pub site: Option<String>,
    /// Only results newer than this many days.
    pub recency_days: Option<u16>,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Result URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Snippet.
    pub snippet: String,
    /// Provider that returned it.
    pub provider: SearchProvider,
}

/// Web search.
pub trait WebSearch: Send + Sync {
    /// Runs a query.
    fn search(
        &self,
        query: &SearchQuery,
    ) -> impl Future<Output = Result<Vec<SearchHit>, SearchError>> + Send;
}

/// Search failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// Provider unreachable.
    #[error("search provider unavailable: {0}")]
    Unavailable(String),
    /// HTTP 429.
    #[error("rate limited (retry after {retry_after_ms:?} ms)")]
    RateLimited {
        /// Server-provided retry delay.
        retry_after_ms: Option<u64>,
    },
    /// Missing or invalid API key.
    #[error("unauthorized")]
    Unauthorized,
    /// The query was rejected.
    #[error("bad query: {0}")]
    BadQuery(String),
    /// Unexpected response.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl HasKind for SearchError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable(_) | Self::RateLimited { .. } => ErrorKind::Transient,
            Self::Unauthorized | Self::BadQuery(_) | Self::Protocol(_) => ErrorKind::Permanent,
        }
    }
}

// --------------------------------------------------------------------------------------- ingest

/// Input to a document parser.
#[derive(Debug, Clone, Copy)]
pub struct DocSource<'a> {
    /// File name (for type detection and provenance).
    pub name: &'a str,
    /// File content.
    pub bytes: &'a [u8],
    /// MIME type, if known.
    pub mime: Option<&'a str>,
}

/// A structural block of a parsed document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocBlock {
    /// Heading of `level` (1 = top).
    Heading {
        /// Heading level.
        level: u8,
        /// Heading text.
        text: String,
    },
    /// Paragraph text.
    Paragraph {
        /// The text.
        text: String,
    },
    /// Table rows (first row = header when present).
    Table {
        /// Cell text by row.
        rows: Vec<Vec<String>>,
    },
    /// Code or preformatted text.
    Code {
        /// Language hint.
        lang: Option<String>,
        /// The code.
        text: String,
    },
    /// An embedded image (bytes in CAS).
    Image {
        /// Alt text or OCR caption.
        alt: Option<String>,
        /// Image bytes.
        content: Cid,
    },
}

/// A parsed document.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Document {
    /// Title, if the format has one.
    pub title: Option<String>,
    /// Blocks in reading order.
    pub blocks: Vec<DocBlock>,
}

/// Document parser (docx, xlsx, pdf, pptx, html, md; OCR for scans).
pub trait DocParser: Send + Sync {
    /// Parses one document.
    ///
    /// # Errors
    /// [`IngestError`] when the format is unsupported, malformed or too large.
    fn parse(&self, src: DocSource<'_>) -> Result<Document, IngestError>;
}

/// Ingest failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngestError {
    /// No parser for this format.
    #[error("unsupported document type: {0}")]
    Unsupported(String),
    /// The document is damaged or hostile.
    #[error("malformed document: {0}")]
    Malformed(String),
    /// Over the configured size limit.
    #[error("document too large: {bytes} bytes (limit {limit})")]
    TooLarge {
        /// Actual size.
        bytes: u64,
        /// Configured limit.
        limit: u64,
    },
    /// OCR failed.
    #[error("ocr failed: {0}")]
    Ocr(String),
}

impl HasKind for IngestError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}

// --------------------------------------------------------------------------------------- vectors

/// A vector with its id and payload reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorChunk {
    /// Chunk id (CAS id of the chunk text).
    pub id: Cid,
    /// Embedding.
    pub vector: Vec<f32>,
    /// Payload record (source, offsets) in CAS.
    pub payload: Cid,
}

/// A nearest-neighbour hit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VectorHit {
    /// Chunk id.
    pub id: Cid,
    /// Similarity score (higher is closer).
    pub score: f32,
    /// Payload record in CAS.
    pub payload: Cid,
}

/// Embedded vector store (usearch + SQLite by default; S0-9).
pub trait VectorStore: Send + Sync {
    /// Inserts or replaces chunks; returns how many were written.
    ///
    /// # Errors
    /// [`VectorError`] on dimension mismatch or storage failure.
    fn upsert(&self, chunks: &[VectorChunk]) -> Result<usize, VectorError>;
    /// The `k` nearest chunks to `query`.
    ///
    /// # Errors
    /// [`VectorError`] on dimension mismatch or storage failure.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>, VectorError>;
    /// Deletes chunks; returns how many existed.
    ///
    /// # Errors
    /// [`VectorError`] on storage failure.
    fn delete(&self, ids: &[Cid]) -> Result<usize, VectorError>;
    /// Number of stored chunks.
    ///
    /// # Errors
    /// [`VectorError`] on storage failure.
    fn count(&self) -> Result<u64, VectorError>;
}

/// Vector store failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VectorError {
    /// Vector length differs from the index dimension.
    #[error("dimension mismatch: index has {expected}, got {found}")]
    DimensionMismatch {
        /// Index dimension.
        expected: usize,
        /// Offending vector length.
        found: usize,
    },
    /// Storage I/O failed.
    #[error("vector store I/O error: {0}")]
    Io(String),
    /// Index files are inconsistent.
    #[error("vector index corrupt: {0}")]
    Corrupt(String),
}

impl HasKind for VectorError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::DimensionMismatch { .. } => ErrorKind::Permanent,
            Self::Io(_) => ErrorKind::Transient,
            Self::Corrupt(_) => ErrorKind::Fatal,
        }
    }
}

// ---------------------------------------------------------------------------------------- events

/// Append-only event store (redb in `polaris-storage-eventlog`).
pub trait EventStore: Send + Sync {
    /// Appends a record; returns the sequence number the store assigned.
    ///
    /// # Errors
    /// [`EventStoreError`] when the store is unavailable, read-only or corrupt.
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError>;
    /// Records with `from <= seq <= to`, in order.
    ///
    /// # Errors
    /// [`EventStoreError`] when the store is unavailable or a record cannot be decoded.
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError>;
}

/// Event store failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventStoreError {
    /// Locked or temporarily unavailable.
    #[error("event store unavailable: {0}")]
    Unavailable(String),
    /// Opened read-only (replay, inspection).
    #[error("event store is read-only")]
    ReadOnly,
    /// A stored record failed its integrity check.
    #[error("event log corrupt at seq {seq:?}")]
    Corrupt {
        /// First bad sequence number.
        seq: Seq,
    },
    /// A stored record could not be decoded.
    #[error("event codec error: {0}")]
    Codec(String),
}

impl HasKind for EventStoreError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable(_) => ErrorKind::Transient,
            Self::ReadOnly => ErrorKind::Permanent,
            Self::Corrupt { .. } | Self::Codec(_) => ErrorKind::Fatal,
        }
    }
}

// ---------------------------------------------------------------------------------------- kernel

/// Receipt for an accepted command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
    /// The command.
    pub command: CommandId,
    /// Log position at acceptance.
    pub accepted_at: Seq,
}

/// A pull-based stream of event records (`None` = subscription closed).
pub trait EventSource: Send {
    /// The next record, or `None` when the subscription closed.
    fn next_event(&mut self) -> impl Future<Output = Option<EventRecord>> + Send;
}

/// The kernel as seen by front-ends (TUI, desktop, web) and remote clients. Commands enter
/// unverified; the kernel's verifier promotes them before any handler runs.
pub trait KernelApi: Send + Sync {
    /// Event subscription type.
    type Events: EventSource;

    /// Submits a command for verification and execution.
    fn submit(
        &self,
        cmd: Command<Unverified>,
    ) -> impl Future<Output = Result<CommandReceipt, KernelError>> + Send;
    /// Subscribes to events from `from` onwards (replay, then live).
    fn subscribe(
        &self,
        from: Seq,
    ) -> impl Future<Output = Result<Self::Events, KernelError>> + Send;
}

/// Kernel API failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KernelError {
    /// The verifier or a guard rejected the command.
    #[error("command rejected: {0}")]
    Rejected(String),
    /// Back-pressure: try again later.
    #[error("kernel busy")]
    Busy,
    /// The session or kernel shut down.
    #[error("kernel closed")]
    Closed,
    /// An internal invariant failed.
    #[error("kernel internal error: {0}")]
    Internal(String),
}

impl HasKind for KernelError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Rejected(_) => ErrorKind::Policy,
            Self::Busy => ErrorKind::Transient,
            Self::Closed => ErrorKind::Permanent,
            Self::Internal(_) => ErrorKind::Fatal,
        }
    }
}

// -------------------------------------------------------------------------------------- platform

/// A process tree to start under supervision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnSpec {
    /// Program path.
    pub program: PathBuf,
    /// Arguments.
    pub args: Vec<OsString>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables.
    pub env: Vec<(OsString, OsString)>,
}

/// A started process tree (root process id; on Windows the tree is bound to a Job Object).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessTree {
    /// Root process id.
    pub root_pid: u32,
    /// Opaque adapter-owned handle identifying this exact live process tree.
    ///
    /// This identifier is meaningful only to the `PlatformOps` instance that created it. It is
    /// not a durable process identity and must never be used to recover or signal a PID after a
    /// host restart.
    pub handle_id: u64,
}

/// Exit information for a process tree whose root has exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExit {
    /// Exit code when the operating system reports one.
    pub code: Option<i32>,
}

/// How a stop request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// Exited within the grace period.
    Exited {
        /// Exit code, when available.
        code: Option<i32>,
    },
    /// Killed after the grace period.
    Killed,
}

/// System memory state (drives model eviction before builds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryPressure {
    /// Plenty of free memory.
    Normal,
    /// Evict idle models.
    Elevated,
    /// Evict everything evictable; refuse new heavy work.
    Critical,
}

/// OS seam (keeps the later Linux/macOS port honest).
///
/// Windows semantics: the adapter may close the worker's stdin to allow cooperative exit, waits
/// for `grace`, then terminates the whole Job Object. Unix implementations send SIGTERM, wait,
/// then SIGKILL the process group. [`PlatformOps::kill_tree`] is the synchronous emergency path
/// used when an owning task is dropped before async teardown completes.
pub trait PlatformOps: Send + Sync {
    /// Starts a supervised process tree.
    ///
    /// # Errors
    /// [`PlatformError`] when the program cannot be started.
    fn spawn_tree(&self, spec: &SpawnSpec) -> Result<ProcessTree, PlatformError>;
    /// Stops a process tree gracefully, then forcibly.
    fn graceful_stop(
        &self,
        tree: &ProcessTree,
        grace: Duration,
    ) -> impl Future<Output = Result<StopOutcome, PlatformError>> + Send;
    /// Immediately sends the platform's force-stop operation to a retained live tree handle.
    ///
    /// This synchronous emergency path is used by Drop guards when a task is aborted or unwinds.
    /// It must never re-resolve the root PID.
    ///
    /// # Errors
    /// [`PlatformError`] when the handle cannot be found or the OS refuses the operation.
    fn kill_tree(&self, tree: &ProcessTree) -> Result<(), PlatformError>;
    /// Checks whether the root process has exited without blocking.
    ///
    /// Implementations must validate the opaque handle against their retained live-process
    /// record. A PID lookup alone is not a valid implementation.
    ///
    /// # Errors
    /// [`PlatformError`] when the tree handle is unknown or the OS query fails.
    fn try_wait_tree(
        &self,
        tree: &ProcessTree,
    ) -> impl Future<Output = Result<Option<ProcessExit>, PlatformError>> + Send;
    /// Current memory pressure.
    ///
    /// # Errors
    /// [`PlatformError`] when the OS query fails.
    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError>;
}

/// Platform failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// Program or process not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// The OS refused the operation.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// Other OS error.
    #[error("platform I/O error: {0}")]
    Io(String),
    /// Not supported on this OS.
    #[error("unsupported on this platform: {0}")]
    Unsupported(&'static str),
    /// The requested operation violates the platform adapter's process policy.
    #[error("platform policy rejected operation: {0}")]
    Policy(String),
}

impl HasKind for PlatformError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFound(_) | Self::Unsupported(_) => ErrorKind::Permanent,
            Self::PermissionDenied(_) | Self::Policy(_) => ErrorKind::Policy,
            Self::Io(_) => ErrorKind::Transient,
        }
    }
}

// -------------------------------------------------------------------------------------- redactor

/// Counters reported by a [`Redactor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RedactStats {
    /// Bytes read.
    pub bytes_in: usize,
    /// Secrets or PII spans replaced.
    pub replacements: u32,
}

/// Masks secrets and PII. Writes into a caller-owned buffer so the hot path does not allocate.
pub trait Redactor: Send + Sync {
    /// Appends the redacted form of `input` to `out`.
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats;
}

// ------------------------------------------------------------------------------------- operators

/// A pipeline stage of the kernel (sanitize, directives, triage, context, rate limit, …).
pub trait FunctionalOperator: Send + Sync {
    /// Stage input.
    type In: Send;
    /// Stage output.
    type Out: Send;
    /// Stage error.
    type Err: HasKind + Send;

    /// Runs the stage.
    fn apply(&self, input: Self::In) -> impl Future<Output = Result<Self::Out, Self::Err>> + Send;
}

// ------------------------------------------------------------------------------------ exact cache

/// Scope an exact-cache entry belongs to (for `/cache purge` and `$storage-level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheScope {
    /// Shared across workspaces.
    Global,
    /// One workspace (descriptor in CAS).
    Workspace {
        /// Workspace descriptor.
        workspace: Cid,
    },
    /// One session.
    Session {
        /// The session.
        session: SessionId,
    },
}

/// L2 exact cache backing store (lives in the event database; injected into the tiered cache).
pub trait ExactCacheStore: Send + Sync {
    /// The cached value for a 32-byte key.
    ///
    /// # Errors
    /// [`CacheStoreError`] on storage failure.
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError>;
    /// Stores a value under `key` in `scope`.
    ///
    /// # Errors
    /// [`CacheStoreError`] on storage failure or an oversized value.
    fn put(&self, key: [u8; 32], value: &[u8], scope: CacheScope) -> Result<(), CacheStoreError>;
    /// Removes every entry of `scope`; returns how many were removed.
    ///
    /// # Errors
    /// [`CacheStoreError`] on storage failure.
    fn clear_scope(&self, scope: &CacheScope) -> Result<u64, CacheStoreError>;
}

/// Exact-cache store failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheStoreError {
    /// Storage I/O failed.
    #[error("cache store I/O error: {0}")]
    Io(String),
    /// Stored data is inconsistent.
    #[error("cache store corrupt: {0}")]
    Corrupt(String),
    /// The value exceeds the per-entry limit.
    #[error("cache value too large: {bytes} bytes (limit {limit})")]
    TooLarge {
        /// Value size.
        bytes: u64,
        /// Configured limit.
        limit: u64,
    },
}

impl HasKind for CacheStoreError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io(_) => ErrorKind::Transient,
            Self::Corrupt(_) => ErrorKind::Fatal,
            Self::TooLarge { .. } => ErrorKind::Permanent,
        }
    }
}
