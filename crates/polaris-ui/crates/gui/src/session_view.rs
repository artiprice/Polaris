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

/// Local note added to every opened session. It is shown, never recorded, and never implies a
/// reply is coming.
pub const OFFLINE_NOTE: &str = "Offline session · messages are recorded locally; model execution is unavailable until the P3 model path exists, so nothing will reply.";

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
                self.view.conversation.clear();
                if let Err(error) = self.view.push_system(OFFLINE_NOTE) {
                    self.notice = format!("Conversation view is full: {error}.");
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
                Vec::new()
            }
        }
    }

    /// "New session": close the open one and return to the welcome screen.
    pub fn new_session(&mut self) -> Vec<BackendRequest> {
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
