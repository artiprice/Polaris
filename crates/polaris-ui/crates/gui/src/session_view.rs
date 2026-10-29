//! Window-side state of the session flow: welcome → session screen, the composer's two-phase
//! submit, the conversation of the open session and the sidebar list.
//!
//! Pure logic over [`BackendEvent`]s; it returns the [`BackendRequest`]s to send. The Slint
//! wiring in `app.rs` only copies its snapshots into window properties.

use std::path::Path;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use polaris_ui_model::composer::{Composer, PreparedSubmission};
use polaris_ui_model::view::{Entry, ViewState};

use crate::backend::{BackendEvent, BackendRequest, SearchHit, SessionSummary};

/// Local note added to an opened session when no model provider is usable. It is shown, never
/// recorded, and never implies a reply is coming.
pub const OFFLINE_NOTE: &str = "Offline session · messages are recorded locally. No model provider is configured, so nothing will reply; see Settings for setup.";

/// Model connection as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Provider selection is still running.
    Connecting,
    /// Replies come from this provider and model.
    Ready {
        /// Provider name.
        provider: String,
        /// Model id.
        model: String,
    },
    /// No provider is usable.
    Offline(String),
}

/// The reply being streamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reply {
    started: std::time::Instant,
    /// Conversation entry receiving streamed text.
    entry: Option<usize>,
    /// Separate the next text from earlier text (a tool round happened).
    break_pending: bool,
    /// A review ended the streamed text; a later answer goes below it.
    after_review: bool,
}

/// A change the reply waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingReview {
    id: String,
    /// First line of the prompt ("Change `src/lib.rs`?").
    title: String,
}

/// Which screen the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Welcome page with the ASCII aurora.
    Welcome,
    /// Session view (sidebar, conversation, docked composer).
    Session,
}

/// The session currently open in the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSession {
    /// Session id.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Workspace stored with the session.
    pub workspace: String,
    /// Branch stored with the session, if any.
    pub branch: Option<String>,
}

/// One sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// Session id.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Relative update time ("now", "5m", "3h", "2d").
    pub when: String,
    /// Whether this is the open session.
    pub active: bool,
}

/// Sessions of one workspace folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGroup {
    /// Folder name of the workspace.
    pub name: String,
    /// Full workspace path.
    pub path: String,
    /// Rows, newest first.
    pub rows: Vec<SessionRow>,
}

/// Title shown for a session the registry has no name for.
pub const UNTITLED: &str = "Untitled session";

fn title_or_untitled(title: &str) -> String {
    if title.trim().is_empty() {
        String::from(UNTITLED)
    } else {
        title.to_owned()
    }
}

fn folder_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// Compact relative time of `then_ms` seen from `now_ms` (both Unix milliseconds).
#[must_use]
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    let elapsed = now_ms.saturating_sub(then_ms);
    let minutes = match elapsed.checked_div(60_000) {
        Some(minutes) if minutes > 0 => minutes,
        Some(_) | None => return String::from("now"),
    };
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = match minutes.checked_div(60) {
        Some(hours) => hours,
        None => return String::from("now"),
    };
    if hours < 24 {
        return format!("{hours}h");
    }
    match hours.checked_div(24) {
        Some(days) => format!("{days}d"),
        None => String::from("now"),
    }
}

/// State behind the welcome and session screens.
#[derive(Debug)]
pub struct SessionController {
    view: ViewState,
    composer: Composer,
    draft: String,
    pending: Option<PreparedSubmission>,
    screen: Screen,
    starting: bool,
    open: Option<OpenSession>,
    sessions: Vec<SessionSummary>,
    hits: Vec<SearchHit>,
    notice: String,
    connection: Connection,
    reply: Option<Reply>,
    models: Vec<(Effort, String)>,
    effort: Effort,
    usage: (u64, u64, u64),
    stats: Vec<String>,
    reviews: std::collections::VecDeque<PendingReview>,
}

/// One row of the Models panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    /// Effort name.
    pub effort: String,
    /// Model id served for it.
    pub model: String,
    /// Whether the conversation uses it now.
    pub active: bool,
}

impl SessionController {
    /// Welcome screen, empty draft, no session. `model` labels the conversation view only;
    /// nothing executes.
    #[must_use]
    pub fn new(model: ModelId) -> Self {
        Self {
            view: ViewState::new(model, Effort::High),
            composer: Composer::new(),
            draft: String::new(),
            pending: None,
            screen: Screen::Welcome,
            starting: false,
            open: None,
            sessions: Vec::new(),
            hits: Vec::new(),
            notice: String::new(),
            connection: Connection::Connecting,
            reply: None,
            models: Vec::new(),
            effort: Effort::Medium,
            usage: (0, 0, 0),
            stats: Vec::new(),
            reviews: std::collections::VecDeque::new(),
        }
    }

    /// Models panel rows: the provider's model for each effort.
    #[must_use]
    pub fn model_rows(&self) -> Vec<ModelRow> {
        self.models
            .iter()
            .map(|(effort, model)| ModelRow {
                effort: effort.name().to_owned(),
                model: model.clone(),
                active: *effort == self.effort,
            })
            .collect()
    }

    /// Switch to the model of the Models panel row at `index`.
    pub fn choose_model(&mut self, index: usize) -> Vec<BackendRequest> {
        let Some((effort, _)) = self.models.get(index) else {
            return Vec::new();
        };
        if self.reply.is_some() {
            "Change the model after the reply finishes.".clone_into(&mut self.notice);
            return Vec::new();
        }
        if *effort == self.effort {
            return Vec::new();
        }
        vec![BackendRequest::SetEffort(*effort)]
    }

    /// Usage panel lines: the open session's tokens, then activity across sessions.
    #[must_use]
    pub fn usage_lines(&self) -> Vec<String> {
        let (requests, prompt, completion) = self.usage;
        let mut lines = vec![
            format!("This session    {requests} requests"),
            format!("Prompt          {prompt} tokens"),
            format!("Completion      {completion} tokens"),
            String::new(),
        ];
        if self.stats.is_empty() {
            lines.push(String::from("Loading activity…"));
        } else {
            lines.extend(self.stats.iter().cloned());
        }
        lines
    }

    /// Plan mode toggled in the composer (index 0 is plan).
    #[must_use]
    pub fn set_mode(&self, index: i32) -> Vec<BackendRequest> {
        vec![BackendRequest::Mode { plan: index == 0 }]
    }

    /// Effort picked in the composer menu (index into low…max).
    pub fn choose_effort(&mut self, index: usize) -> Vec<BackendRequest> {
        match Effort::ALL.get(index) {
            Some(effort) if self.models.iter().any(|(level, _)| level == effort) => {
                if self.reply.is_some() {
                    "Change the model after the reply finishes.".clone_into(&mut self.notice);
                    return Vec::new();
                }
                if *effort == self.effort {
                    Vec::new()
                } else {
                    vec![BackendRequest::SetEffort(*effort)]
                }
            }
            _ => Vec::new(),
        }
    }

    /// The provider's distinct models in effort order, and the index of the one in use.
    #[must_use]
    pub fn composer_models(&self) -> (Vec<String>, Option<usize>) {
        let mut names: Vec<String> = Vec::new();
        for (_, model) in &self.models {
            if !names.contains(model) {
                names.push(model.clone());
            }
        }
        let current = self
            .models
            .iter()
            .find(|(effort, _)| *effort == self.effort)
            .and_then(|(_, model)| names.iter().position(|name| name == model));
        (names, current)
    }

    /// Composer model menu pick: the lowest effort served by that model.
    pub fn choose_composer_model(&mut self, index: usize) -> Vec<BackendRequest> {
        let (names, _) = self.composer_models();
        let Some(name) = names.get(index) else {
            return Vec::new();
        };
        match self.models.iter().position(|(_, model)| model == name) {
            Some(row) => self.choose_model(row),
            None => Vec::new(),
        }
    }

    /// Current effort as an index into low…max.
    #[must_use]
    pub fn effort_index(&self) -> usize {
        Effort::ALL
            .iter()
            .position(|effort| *effort == self.effort)
            .unwrap_or(0)
    }

    /// "ready", "working", "connecting" or "offline".
    #[must_use]
    pub fn status_state(&self) -> &'static str {
        if !self.reviews.is_empty() {
            return "waiting";
        }
        if self.reply.is_some() {
            return "working";
        }
        match self.connection {
            Connection::Ready { .. } => "ready",
            Connection::Connecting => "connecting",
            Connection::Offline(_) => "offline",
        }
    }

    /// Ask for fresh activity lines (the Usage panel opened).
    #[must_use]
    pub fn refresh_usage(&self) -> Vec<BackendRequest> {
        vec![BackendRequest::Stats]
    }

    /// Model connection state.
    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Whether a reply is being generated.
    #[must_use]
    pub const fn working(&self) -> bool {
        self.reply.is_some()
    }

    /// Status-bar text: the connection, or the running reply with its elapsed time.
    #[must_use]
    pub fn status_line(&self) -> String {
        if !self.reviews.is_empty() {
            return String::from("Waiting for your decision on a change");
        }
        if let Some(reply) = &self.reply {
            return format!(
                "Working {} · Esc to stop",
                polaris_ui_model::view::format_duration_ms(
                    u64::try_from(reply.started.elapsed().as_millis()).unwrap_or(u64::MAX)
                )
            );
        }
        match &self.connection {
            Connection::Connecting => String::from("Connecting to a model…"),
            Connection::Ready { provider, model } => format!("{model} · {provider}"),
            Connection::Offline(_) => String::from("Offline · no model replies"),
        }
    }

    /// Question of the change the reply waits on ("Change `notes.md`?"), if any.
    #[must_use]
    pub fn review(&self) -> Option<&str> {
        self.reviews.front().map(|review| review.title.as_str())
    }

    /// Apply or decline the oldest waiting change; `note` reaches the model with a decline.
    pub fn decide_review(&mut self, apply: bool, note: &str) -> Vec<BackendRequest> {
        let Some(review) = self.reviews.pop_front() else {
            return Vec::new();
        };
        let note = Some(note.trim().to_owned()).filter(|note| !note.is_empty());
        if !apply {
            let line = match &note {
                Some(note) => format!("Not applied · {note}"),
                None => String::from("Not applied"),
            };
            if let Err(error) = self.view.push_system(line) {
                self.notice = format!("Conversation view is full: {error}.");
            }
        }
        vec![BackendRequest::Review {
            request_id: review.id,
            apply,
            note,
        }]
    }

    /// Stop the running reply.
    pub fn interrupt(&mut self) -> Vec<BackendRequest> {
        if self.reply.is_some() {
            "Stopping…".clone_into(&mut self.notice);
            vec![BackendRequest::Interrupt]
        } else {
            Vec::new()
        }
    }

    fn stream(&mut self, delta: &str) {
        let Some(reply) = self.reply.as_mut() else {
            return;
        };
        let separator = if reply.break_pending && reply.entry.is_some() {
            "\n\n"
        } else {
            ""
        };
        reply.break_pending = false;
        let result = match reply.entry {
            Some(index) => self
                .view
                .append_to_entry(index, separator)
                .and_then(|()| self.view.append_to_entry(index, delta)),
            None => {
                let index = self.view.conversation.len();
                reply.entry = Some(index);
                self.view.push_assistant(delta.to_owned())
            }
        };
        if let Err(error) = result {
            self.notice = format!("Conversation view is full: {error}.");
        }
    }

    /// Current screen.
    #[must_use]
    pub const fn screen(&self) -> Screen {
        self.screen
    }
    /// Composer text as the controller knows it.
    #[must_use]
    pub fn draft(&self) -> &str {
        &self.draft
    }
    /// Notice under/above the composer (empty when none).
    #[must_use]
    pub fn notice(&self) -> &str {
        &self.notice
    }
    /// Whether a submit awaits the kernel's verdict.
    #[must_use]
    pub const fn pending(&self) -> bool {
        self.pending.is_some()
    }
    /// Whether the kernel is starting for an open request.
    #[must_use]
    pub const fn starting(&self) -> bool {
        self.starting
    }
    /// The open session, if any.
    #[must_use]
    pub const fn open_session(&self) -> Option<&OpenSession> {
        self.open.as_ref()
    }
    /// Conversation of the open session, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.view.conversation
    }
    /// Latest search results.
    #[must_use]
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }
    /// Title of the open session, or the new-session title.
    #[must_use]
    pub fn title(&self) -> String {
        match &self.open {
            Some(open) => open.title.clone(),
            None => String::from("New session"),
        }
    }

    /// Mirror an edit of the composer text; long or invalid drafts produce a notice.
    pub fn edit(&mut self, text: &str) {
        text.clone_into(&mut self.draft);
        let mut composer = Composer::new();
        self.notice = match composer.insert(text) {
            Ok(()) => String::new(),
            Err(error) => format!("This draft cannot be sent as is: {error}."),
        };
        self.composer = composer;
    }

    /// Send the draft: opens a session first when none is open.
    pub fn submit(&mut self) -> Vec<BackendRequest> {
        if self.pending.is_some() {
            "Waiting for the previous message to be recorded; draft kept."
                .clone_into(&mut self.notice);
            return Vec::new();
        }
        if self.reply.is_some() {
            "A reply is still running; press Esc to stop it. Draft kept."
                .clone_into(&mut self.notice);
            return Vec::new();
        }
        if self.draft.trim().is_empty() {
            return Vec::new();
        }
        let prepared = match self.composer.prepare_submit() {
            Ok(prepared) => prepared,
            Err(error) => {
                self.notice = format!("This draft cannot be sent as is: {error}.");
                return Vec::new();
            }
        };
        if !prepared.submission().images.is_empty() {
            self.composer.cancel_submit(prepared);
            "Attachment processing requires the model/ingest profile; draft kept."
                .clone_into(&mut self.notice);
            return Vec::new();
        }
        let text = prepared.submission().text.clone();
        self.pending = Some(prepared);
        self.notice.clear();
        self.screen = Screen::Session;
        let mut requests = Vec::with_capacity(2);
        if self.open.is_none() {
            self.starting = true;
            requests.push(BackendRequest::Open { resume: None });
        }
        requests.push(BackendRequest::Submit { text });
        requests
    }

    /// Apply one backend event; returns follow-up requests.
    pub fn apply(&mut self, event: BackendEvent) -> Vec<BackendRequest> {
        match event {
            BackendEvent::Starting => {
                self.starting = true;
                Vec::new()
            }
            BackendEvent::Opened {
                id,
                title,
                workspace,
                branch,
            } => {
                self.starting = false;
                self.reply = None;
                self.usage = (0, 0, 0);
                self.view.conversation.clear();
                if matches!(self.connection, Connection::Offline(_)) {
                    if let Err(error) = self.view.push_system(OFFLINE_NOTE) {
                        self.notice = format!("Conversation view is full: {error}.");
                    }
                }
                self.open = Some(OpenSession {
                    id,
                    title: title_or_untitled(&title),
                    workspace: workspace.display().to_string(),
                    branch,
                });
                self.screen = Screen::Session;
                vec![BackendRequest::List]
            }
            BackendEvent::Message { kind: _, text } => {
                if let Err(error) = self.view.push_user(text) {
                    self.notice = format!("Conversation view is full: {error}.");
                }
                Vec::new()
            }
            BackendEvent::Committed => {
                if let Some(prepared) = self.pending.take() {
                    // A stale submission means the user kept typing: their new text stays.
                    if self.composer.commit_submit(prepared).is_ok() {
                        self.draft.clear();
                        self.composer = Composer::new();
                    }
                }
                vec![BackendRequest::List]
            }
            BackendEvent::Rejected(reason) => {
                if let Some(prepared) = self.pending.take() {
                    self.composer.cancel_submit(prepared);
                }
                self.notice = format!("Not recorded: {reason}");
                Vec::new()
            }
            BackendEvent::Sessions(sessions) => {
                self.sessions = sessions;
                Vec::new()
            }
            BackendEvent::Hits(hits) => {
                self.hits = hits;
                Vec::new()
            }
            BackendEvent::Failed(reason) => {
                self.starting = false;
                if let Some(prepared) = self.pending.take() {
                    self.composer.cancel_submit(prepared);
                }
                self.notice = reason;
                Vec::new()
            }
            BackendEvent::Closed => {
                self.open = None;
                self.reply = None;
                Vec::new()
            }
            BackendEvent::ProviderReady {
                provider,
                model,
                effort,
                models,
            } => {
                if let Ok(id) = ModelId::new(&model) {
                    self.view.set_model(id);
                }
                self.models = models;
                self.effort = effort;
                self.connection = Connection::Ready { provider, model };
                Vec::new()
            }
            BackendEvent::ModelSwitched { model, effort } => {
                if let Ok(id) = ModelId::new(&model) {
                    self.view.set_model(id);
                }
                self.effort = effort;
                if let Connection::Ready { model: current, .. } = &mut self.connection {
                    current.clone_from(&model);
                }
                self.notice = format!("Model {model} · {} effort", effort.name());
                Vec::new()
            }
            BackendEvent::Usage {
                requests,
                prompt_tokens,
                completion_tokens,
            } => {
                self.usage = (requests, prompt_tokens, completion_tokens);
                Vec::new()
            }
            BackendEvent::Stats(lines) => {
                self.stats = lines;
                Vec::new()
            }
            BackendEvent::ProviderUnavailable(reason) => {
                self.connection = Connection::Offline(reason);
                Vec::new()
            }
            BackendEvent::ReplyStarted => {
                self.reply = Some(Reply {
                    started: std::time::Instant::now(),
                    entry: None,
                    break_pending: false,
                    after_review: false,
                });
                Vec::new()
            }
            BackendEvent::Review { request_id, prompt } => {
                if self.reviews.iter().any(|review| review.id == request_id) {
                    return Vec::new();
                }
                if let Some(reply) = self.reply.as_mut() {
                    reply.entry = None;
                    reply.break_pending = false;
                    reply.after_review = true;
                }
                let title = prompt
                    .lines()
                    .next()
                    .unwrap_or("Apply this change?")
                    .replace('`', "");
                if let Err(error) = self.view.push_assistant(prompt) {
                    self.notice = format!("Conversation view is full: {error}.");
                }
                self.reviews.push_back(PendingReview {
                    id: request_id,
                    title,
                });
                Vec::new()
            }
            BackendEvent::ReviewClosed(id) => {
                self.reviews.retain(|review| review.id != id);
                Vec::new()
            }
            BackendEvent::Applied(path) => {
                if let Err(error) = self.view.push_system(format!("✓ Edited {path}")) {
                    self.notice = format!("Conversation view is full: {error}.");
                }
                Vec::new()
            }
            BackendEvent::Deleted(path) => {
                if let Err(error) = self.view.push_system(format!("✓ Deleted {path}")) {
                    self.notice = format!("Conversation view is full: {error}.");
                }
                Vec::new()
            }
            BackendEvent::ReplyText(delta) => {
                self.stream(&delta);
                Vec::new()
            }
            BackendEvent::ReplyStep => {
                if let Some(reply) = self.reply.as_mut() {
                    reply.break_pending = reply.entry.is_some();
                }
                Vec::new()
            }
            BackendEvent::ReplyFinished { answer, summary } => {
                let (streamed, after_review) = self.reply.take().map_or((false, false), |reply| {
                    (reply.entry.is_some(), reply.after_review)
                });
                self.reviews.clear();
                let mut result = Ok(());
                if !streamed {
                    result = match answer.as_deref().map(str::trim) {
                        Some(text) if !text.is_empty() => self.view.push_assistant(text.to_owned()),
                        _ if after_review => Ok(()),
                        _ => self
                            .view
                            .push_warning("The model stopped without an answer."),
                    };
                }
                if let Err(error) = result.and_then(|()| self.view.push_system(summary)) {
                    self.notice = format!("Conversation view is full: {error}.");
                }
                if self.notice == "Stopping…" {
                    self.notice.clear();
                }
                vec![BackendRequest::List]
            }
            BackendEvent::ReplyFailed(reason) => {
                self.reply = None;
                self.reviews.clear();
                let shown = if reason == "Interrupted" {
                    self.view.push_warning("Stopped.")
                } else {
                    self.view.push_error(reason)
                };
                if let Err(error) = shown {
                    self.notice = format!("Conversation view is full: {error}.");
                } else if self.notice == "Stopping…" {
                    self.notice.clear();
                }
                Vec::new()
            }
        }
    }

    /// "New session": close the open one and return to the welcome screen.
    pub fn new_session(&mut self) -> Vec<BackendRequest> {
        if self.reply.is_some() {
            "A reply is still running; press Esc to stop it first.".clone_into(&mut self.notice);
            return Vec::new();
        }
        self.screen = Screen::Welcome;
        self.view.conversation.clear();
        self.notice.clear();
        if self.open.take().is_some() {
            vec![BackendRequest::Close]
        } else {
            Vec::new()
        }
    }

    /// Open (resume) a listed session.
    pub fn open_listed(&mut self, id: &str) -> Vec<BackendRequest> {
        if self.reply.is_some() {
            "A reply is still running; press Esc to stop it first.".clone_into(&mut self.notice);
            return Vec::new();
        }
        self.screen = Screen::Session;
        if self.open.as_ref().is_some_and(|open| open.id == id) {
            return Vec::new();
        }
        self.starting = true;
        self.notice.clear();
        vec![BackendRequest::Open {
            resume: Some(id.to_owned()),
        }]
    }

    /// Show the session screen (sidebar list) without opening a session.
    pub fn show_sessions(&mut self) -> Vec<BackendRequest> {
        self.screen = Screen::Session;
        vec![BackendRequest::List]
    }

    /// Search session history; a blank query clears the results.
    pub fn search(&mut self, query: &str) -> Vec<BackendRequest> {
        let query = query.trim();
        if query.is_empty() {
            self.hits.clear();
            return Vec::new();
        }
        vec![BackendRequest::Search {
            query: query.to_owned(),
        }]
    }

    /// Sidebar groups: by workspace folder, groups ordered by their newest session.
    #[must_use]
    pub fn groups(&self, now_ms: i64) -> Vec<SessionGroup> {
        let mut sorted: Vec<&SessionSummary> = self.sessions.iter().collect();
        sorted.sort_by_key(|session| core::cmp::Reverse(session.updated_ms));
        let active = self.open.as_ref().map(|open| open.id.as_str());
        let mut groups: Vec<SessionGroup> = Vec::new();
        for session in sorted {
            let path = session.workspace.display().to_string();
            let row = SessionRow {
                id: session.id.clone(),
                title: title_or_untitled(&session.title),
                when: relative_time(now_ms, session.updated_ms),
                active: active == Some(session.id.as_str()),
            };
            match groups.iter_mut().find(|group| group.path == path) {
                Some(group) => group.rows.push(row),
                None => groups.push(SessionGroup {
                    name: folder_name(&session.workspace),
                    path,
                    rows: vec![row],
                }),
            }
        }
        groups
    }
}
