//! Contract between the window and the session backend (the desktop app's kernel bridge).
//!
//! The GUI never touches the kernel, tokio or storage: it sends [`BackendRequest`]s down one
//! channel and drains [`BackendEvent`]s from another on the UI thread. The bridge in
//! `apps/polaris-desktop` owns the runtime and the offline kernel session.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use polaris_sdk::effort::Effort;

/// What the window asks the backend to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendRequest {
    /// Start a new session (`resume: None`) or reopen an existing one by id, replaying its
    /// committed messages.
    Open {
        /// Session id to resume, as listed in [`SessionSummary::id`].
        resume: Option<String>,
    },
    /// Record one message in the open session: a single `$directive` or plain user input.
    Submit {
        /// Exact composer text.
        text: String,
    },
    /// Refresh the session list.
    List,
    /// Full-text search over session history.
    Search {
        /// Non-blank query.
        query: String,
    },
    /// Close the open session (stop admissions, persist pause).
    Close,
    /// Interrupt the running reply.
    Interrupt,
    /// Continue the conversation on the provider's model for this effort.
    SetEffort(Effort),
    /// Activity across every local session (the Usage panel).
    Stats,
    /// Plan mode (the model plans changes before writing code) on or off.
    Mode {
        /// Plan mode.
        plan: bool,
    },
    /// Apply or decline the change the reply waits on.
    Review {
        /// Request id from [`BackendEvent::Review`].
        request_id: String,
        /// Write the change.
        apply: bool,
        /// Note for the model, sent with a decline.
        note: Option<String>,
    },
}

/// How a committed message entered the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Plain user input (`SubmitUserInput`).
    UserInput,
    /// A `$directive` applied to the session.
    Directive,
}

/// One session as listed by the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Stable session id.
    pub id: String,
    /// Session name; empty when the registry has none.
    pub title: String,
    /// Workspace the session belongs to.
    pub workspace: PathBuf,
    /// Branch stored with the session, if any.
    pub branch: Option<String>,
    /// Last update, Unix milliseconds.
    pub updated_ms: i64,
}

/// One full-text search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Session the hit belongs to.
    pub session_id: String,
    /// Session title (may be empty).
    pub title: String,
    /// Matching text excerpt.
    pub snippet: String,
}

/// What the backend reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    /// The kernel is starting for an `Open` request.
    Starting,
    /// A session is open and live.
    Opened {
        /// Session id.
        id: String,
        /// Session name (may be empty).
        title: String,
        /// Workspace stored with the session.
        workspace: PathBuf,
        /// Branch stored with the session, if any.
        branch: Option<String>,
    },
    /// A committed message of the open session (live or replayed).
    Message {
        /// Directive or user input.
        kind: MessageKind,
        /// Committed text, read back from the content store.
        text: String,
    },
    /// The pending submit was committed by the kernel.
    Committed,
    /// The pending submit was rejected; the draft must be kept.
    Rejected(String),
    /// Current session list, newest first.
    Sessions(Vec<SessionSummary>),
    /// Search results for the latest query.
    Hits(Vec<SearchHit>),
    /// A backend failure that is not tied to a submit (open, list, search, store in use).
    Failed(String),
    /// The open session was closed.
    Closed,
    /// A model provider was selected; replies are possible.
    ProviderReady {
        /// Provider name (`token_factory`, `openrouter`, `local`).
        provider: String,
        /// Selected model id.
        model: String,
        /// Effort the model serves.
        effort: Effort,
        /// The provider's model for every effort.
        models: Vec<(Effort, String)>,
    },
    /// The conversation continues on another model.
    ModelSwitched {
        /// New model id.
        model: String,
        /// Its effort.
        effort: Effort,
    },
    /// Token use of the open session, from its committed model requests.
    Usage {
        /// Model requests.
        requests: u64,
        /// Prompt tokens.
        prompt_tokens: u64,
        /// Completion tokens.
        completion_tokens: u64,
    },
    /// Activity lines across every local session (`/stats` in the terminal).
    Stats(Vec<String>),
    /// No model provider is usable; messages are recorded without replies.
    ProviderUnavailable(String),
    /// The model started answering the last message.
    ReplyStarted,
    /// Streamed assistant text.
    ReplyText(String),
    /// The model began another request after a tool round.
    ReplyStep,
    /// The reply finished; `answer` is the final text when nothing was streamed.
    ReplyFinished {
        /// Final assistant text, if any.
        answer: Option<String>,
        /// Work summary ("Worked for 3s · 2 requests").
        summary: String,
    },
    /// The reply failed or was interrupted.
    ReplyFailed(String),
    /// The reply waits until the person applies or declines this change.
    Review {
        /// Request id to answer with [`BackendRequest::Review`].
        request_id: String,
        /// Markdown: the target, why the person decides, and the diff.
        prompt: String,
    },
    /// The review was answered or withdrawn.
    ReviewClosed(String),
    /// A reviewed change was written to this workspace path.
    Applied(String),
}

/// The two channel ends the window uses.
#[derive(Debug)]
pub struct BackendLink {
    /// Requests to the backend.
    pub requests: Sender<BackendRequest>,
    /// Events from the backend, drained on the UI thread.
    pub events: Receiver<BackendEvent>,
}
