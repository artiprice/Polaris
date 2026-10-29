//! The server's Server-Sent Events, as the page decodes them. The wire format is
//! `polaris_ui_web::WebEvent` (JSON, `type` in kebab-case); this mirror compiles to wasm.

use serde::Deserialize;

/// One event from `/api/events`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WebEvent {
    /// A provider was selected.
    Provider {
        /// Provider name.
        provider: String,
        /// Model id.
        model: String,
        /// Effort the model serves.
        effort: String,
        /// `(effort, model)` for every configured effort.
        models: Vec<(String, String)>,
    },
    /// No provider is usable.
    ProviderUnavailable {
        /// `provider: reason` list.
        reason: String,
    },
    /// Another model now answers.
    ModelSwitched {
        /// Model id.
        model: String,
        /// Effort name.
        effort: String,
    },
    /// The kernel is opening a session.
    Opening,
    /// A session is open.
    Opened {
        /// Session id.
        id: String,
        /// Title (may be empty).
        title: String,
        /// Workspace path.
        workspace: String,
    },
    /// The open session closed.
    Closed,
    /// A committed message.
    Message {
        /// `user`, `directive` or `system`.
        kind: String,
        /// Text.
        text: String,
    },
    /// The last submit was recorded.
    Committed,
    /// The last submit was rejected.
    Rejected {
        /// Why.
        reason: String,
    },
    /// A reply started.
    ReplyStarted,
    /// Streamed reply text.
    ReplyText {
        /// Delta.
        text: String,
    },
    /// Another model request began after a tool round.
    ReplyStep,
    /// The reply finished.
    ReplyFinished {
        /// Final text when nothing was streamed.
        answer: Option<String>,
        /// Work summary.
        summary: String,
    },
    /// The reply failed or was interrupted.
    ReplyFailed {
        /// `Interrupted` or the error.
        reason: String,
    },
    /// Token use of the open session.
    Usage {
        /// Model requests.
        requests: u64,
        /// Prompt tokens.
        prompt_tokens: u64,
        /// Completion tokens.
        completion_tokens: u64,
    },
    /// A notice to show.
    Notice {
        /// Text.
        text: String,
    },
    /// The reply waits for a decision on this change.
    Review {
        /// Request id to answer.
        request_id: String,
        /// Markdown prompt with the diff.
        prompt: String,
    },
    /// The review was answered or withdrawn.
    ReviewClosed {
        /// Request id.
        request_id: String,
    },
    /// A reviewed change was written.
    Applied {
        /// Workspace path.
        path: String,
    },
    /// A reviewed deletion removed a file.
    Deleted {
        /// Workspace path.
        path: String,
    },
    /// An event this page does not know yet.
    #[serde(other)]
    Unknown,
}

/// One sidebar row from `/api/sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionRow {
    /// Session id.
    pub id: String,
    /// Title (may be empty).
    pub title: String,
    /// Workspace path.
    pub workspace: String,
    /// Last update, Unix milliseconds.
    pub updated_ms: i64,
}
