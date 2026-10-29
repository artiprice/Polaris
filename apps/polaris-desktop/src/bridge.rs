//! Kernel bridge: implements the window's [`polaris_ui_gui::backend`] contract over an offline
//! kernel session ([`polaris_ui_session::OfflineSession`]) and, when chat is enabled, answers typed
//! messages through the shared [`polaris_ui_session::ConversationHost`] (the same provider chain
//! and agent the terminal uses; file changes wait for the review bar).
//!
//! One background thread owns a tokio runtime. Requests arrive on a std channel (forwarded into
//! the runtime by a blocking task); one loop selects over requests and the open session's record
//! stream, and reports [`BackendEvent`]s back on another std channel that the UI thread drains.
//! The kernel starts lazily on the first `Open`, so the welcome screen never waits for it.
//!
//! [`spawn_attached`] runs the same loop over a `polarisd` session instead
//! ([`polaris_ui_session::RemoteSession`], P9): messages are recorded by the daemon and shown
//! from its event stream; replies, search and activity stay with local sessions.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use polaris_sdk::ids::SessionId;
use polaris_ui_gui::backend::{
    BackendEvent, BackendLink, BackendRequest, MessageKind, SearchHit, SessionSummary,
};
use polaris_ui_session::{
    AttachTarget, ChangeReview, ChatProgress, ConversationHost, HostEvent, OfflineSession,
    RecordTextKind, RemoteSession, SessionHostError, SessionMeta, answer_review,
    is_single_directive, list_sessions, records_until_own_input, search_history,
};

use crate::DesktopError;

/// Rows shown in the sidebar (newest first).
const LIST_LIMIT: u32 = 100;
/// Search hits returned per query.
const SEARCH_LIMIT: u32 = 50;
/// Why replies are absent while attached.
const ATTACHED: &str = "Attached to polarisd: messages are recorded in the daemon's session; model replies are not served in attached mode yet.";
/// Why search and activity are absent while attached.
const LOCAL_ONLY: &str = "Search and activity cover the local session store; they are not available while attached to polarisd.";

/// Handle to the bridge thread.
#[derive(Debug)]
pub struct Bridge {
    thread: Option<JoinHandle<()>>,
}

impl Bridge {
    /// Waits for the bridge thread to finish (after the window dropped its request sender).
    ///
    /// # Errors
    /// [`DesktopError::Bridge`] when the thread panicked.
    pub fn join(mut self) -> Result<(), DesktopError> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| DesktopError::Bridge("bridge thread panicked")),
            None => Ok(()),
        }
    }
}

/// Model replies for typed messages: the layered configuration and the dotenv file to read
/// provider keys from.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    /// Loaded configuration (provider order and models).
    pub config: polaris_config::PolarisConfig,
    /// Dotenv file with provider keys, if any.
    pub env_file: Option<PathBuf>,
}

/// Starts the bridge thread for the session store at `state_dir` and new sessions in
/// `workspace`, recording messages without replies.
///
/// # Errors
/// [`DesktopError::Bridge`] when the thread or its runtime cannot start.
pub fn spawn(
    state_dir: PathBuf,
    workspace: PathBuf,
) -> Result<(BackendLink, Bridge), DesktopError> {
    spawn_with_chat(state_dir, workspace, None)
}

/// As [`spawn`], answering typed messages through the provider chain when `chat` is set.
///
/// # Errors
/// [`DesktopError::Bridge`] when the thread or its runtime cannot start.
pub fn spawn_with_chat(
    state_dir: PathBuf,
    workspace: PathBuf,
    chat: Option<ChatOptions>,
) -> Result<(BackendLink, Bridge), DesktopError> {
    spawn_inner(state_dir, workspace, chat, None)
}

/// Starts the bridge thread over the `polarisd` session `target` names (P9): `Open` attaches,
/// typed lines are recorded by the daemon, and the view follows its event stream.
///
/// # Errors
/// [`DesktopError::Bridge`] when the thread or its runtime cannot start.
pub fn spawn_attached(
    state_dir: PathBuf,
    workspace: PathBuf,
    target: AttachTarget,
) -> Result<(BackendLink, Bridge), DesktopError> {
    spawn_inner(state_dir, workspace, None, Some(target))
}

fn spawn_inner(
    state_dir: PathBuf,
    workspace: PathBuf,
    chat: Option<ChatOptions>,
    attach: Option<AttachTarget>,
) -> Result<(BackendLink, Bridge), DesktopError> {
    let (request_tx, request_rx) = mpsc::channel::<BackendRequest>();
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("polaris-bridge")
        .build()
        .map_err(|_| DesktopError::Bridge("tokio runtime failed to start"))?;
    let thread = std::thread::Builder::new()
        .name(String::from("polaris-bridge"))
        .spawn(move || {
            runtime.block_on(run(
                state_dir, workspace, chat, attach, request_rx, event_tx,
            ));
        })
        .map_err(|_| DesktopError::Bridge("bridge thread failed to start"))?;
    Ok((
        BackendLink {
            requests: request_tx,
            events: event_rx,
        },
        Bridge {
            thread: Some(thread),
        },
    ))
}

fn summary(meta: &SessionMeta) -> SessionSummary {
    SessionSummary {
        id: meta.id.to_string(),
        title: meta.name.clone(),
        workspace: meta.workspace.clone(),
        branch: branch_of(&meta.branch),
        updated_ms: meta.updated_at.as_millisecond(),
    }
}

fn branch_of(branch: &str) -> Option<String> {
    if branch.trim().is_empty() {
        None
    } else {
        Some(branch.to_owned())
    }
}

/// Human wording for a host error shown in the window.
fn describe(context: &str, error: &SessionHostError) -> String {
    format!("{context}: {error}")
}

struct Bridgework {
    state_dir: PathBuf,
    workspace: PathBuf,
    events: Sender<BackendEvent>,
    session: Option<OfflineSession>,
    chat: Option<ConversationHost>,
    /// Model requests and tokens of the open session, from its committed records.
    usage: (u64, u64, u64),
    /// Plan mode of the composer (the window starts in plan mode).
    plan: bool,
    /// Changes the running reply waits on, by request id.
    reviews: std::collections::BTreeMap<String, ChangeReview>,
    /// The `polarisd` session `Open` attaches to, in attached mode.
    attach: Option<AttachTarget>,
    /// The attached daemon session.
    remote: Option<RemoteSession>,
}

impl Bridgework {
    async fn open_remote(&mut self, target: AttachTarget) {
        let _sent = self.emit(BackendEvent::Starting);
        self.close_session().await;
        match RemoteSession::attach(&target).await {
            Ok(remote) => {
                let _sent = self.emit(BackendEvent::Opened {
                    id: remote.id.to_string(),
                    title: String::from("polarisd"),
                    workspace: PathBuf::from(remote.endpoint()),
                    branch: None,
                });
                self.remote = Some(remote);
            }
            Err(error) => {
                let _sent = self.emit(BackendEvent::Failed(describe(
                    "Could not attach to polarisd",
                    &error,
                )));
            }
        }
    }

    async fn submit_remote(&mut self, text: &str) {
        if polaris_ui_session::commands::LocalCommand::parse(text).is_some() {
            let _sent = self.emit(BackendEvent::Rejected(String::from(
                "/nv-profile, /nv-verify and /hack run on local sessions; this window is attached to polarisd",
            )));
            return;
        }
        let Some(remote) = self.remote.as_ref() else {
            let _sent = self.emit(BackendEvent::Rejected(String::from("no session is open")));
            return;
        };
        let event = match remote.submit(text).await {
            Ok(_) => BackendEvent::Committed,
            Err(error) => BackendEvent::Rejected(error.to_string()),
        };
        let _sent = self.emit(event);
    }

    fn remote_record(
        &mut self,
        next: Result<Option<polaris_ui_session::ResolvedRecord>, SessionHostError>,
    ) {
        match next {
            Ok(Some(resolved)) => {
                if let (Some(text), Some(kind)) = (resolved.text, resolved.text_kind) {
                    let kind = match kind {
                        RecordTextKind::Directive => MessageKind::Directive,
                        RecordTextKind::UserInput => MessageKind::UserInput,
                    };
                    let _sent = self.emit(BackendEvent::Message { kind, text });
                }
            }
            Ok(None) | Err(_) => {
                self.remote = None;
                let _sent = self.emit(BackendEvent::Failed(String::from(
                    "The connection to polarisd ended; open the session again.",
                )));
                let _closed = self.emit(BackendEvent::Closed);
            }
        }
    }

    fn remote_list(&self) {
        let rows = self
            .remote
            .iter()
            .map(|remote| SessionSummary {
                id: remote.id.to_string(),
                title: String::from("polarisd"),
                workspace: PathBuf::from(remote.endpoint()),
                branch: None,
                updated_ms: 0,
            })
            .collect();
        let _sent = self.emit(BackendEvent::Sessions(rows));
    }

    /// `false` once the window is gone (nobody to report to).
    fn emit(&self, event: BackendEvent) -> bool {
        self.events.send(event).is_ok()
    }

    async fn close_session(&mut self) {
        if let Some(chat) = self.chat.as_mut() {
            chat.reset_conversation();
        }
        if let Some(mut session) = self.session.take() {
            if let Err(error) = session.close().await {
                tracing::warn!(%error, "session close reported a failure");
                let _sent = self.emit(BackendEvent::Failed(describe(
                    "Closing the session failed",
                    &error,
                )));
            }
            let _sent = self.emit(BackendEvent::Closed);
        }
        if self.remote.take().is_some() {
            let _sent = self.emit(BackendEvent::Closed);
        }
    }

    async fn meta_of(&self, id: SessionId) -> Option<SessionMeta> {
        match list_sessions(&self.state_dir, LIST_LIMIT).await {
            Ok(rows) => rows.into_iter().find(|row| row.id == id),
            Err(error) => {
                tracing::warn!(%error, "session metadata lookup failed");
                None
            }
        }
    }

    async fn open(&mut self, resume: Option<String>) {
        let _sent = self.emit(BackendEvent::Starting);
        self.close_session().await;
        let id = match resume {
            Some(text) => match text.parse::<SessionId>() {
                Ok(id) => id,
                Err(_) => {
                    let _sent =
                        self.emit(BackendEvent::Failed(format!("Unknown session id `{text}`")));
                    return;
                }
            },
            None => SessionId::new_v7(),
        };
        let started = std::time::Instant::now();
        self.usage = (0, 0, 0);
        match OfflineSession::open(&self.state_dir, &self.workspace, id).await {
            Ok(session) => {
                tracing::info!(
                    open_ms = started.elapsed().as_millis(),
                    "offline session opened"
                );
                let workspace = session.workspace().to_path_buf();
                self.session = Some(session);
                let (title, branch) = match self.meta_of(id).await {
                    Some(meta) => (meta.name.clone(), branch_of(&meta.branch)),
                    None => (String::new(), None),
                };
                let _sent = self.emit(BackendEvent::Opened {
                    id: id.to_string(),
                    title,
                    workspace,
                    branch,
                });
            }
            Err(error) => {
                let _sent = self.emit(BackendEvent::Failed(describe(
                    "Could not open the local session (is another Polaris process using this session store?)",
                    &error,
                )));
            }
        }
    }

    async fn submit(&mut self, text: &str) {
        let Some(session) = self.session.as_mut() else {
            let _sent = self.emit(BackendEvent::Rejected(String::from("no session is open")));
            return;
        };
        if let Some(command) = polaris_ui_session::commands::LocalCommand::parse(text) {
            // Answered off the bridge loop: a CI run can take minutes.
            let kernel = session.kernel().clone();
            let workspace = session.workspace().to_path_buf();
            let _committed = self.emit(BackendEvent::Committed);
            let events = self.events.clone();
            tokio::spawn(async move {
                let event = match command.run(&kernel, &workspace).await {
                    Ok(lines) => BackendEvent::Report {
                        title: command.title().to_owned(),
                        lines,
                    },
                    Err(error) => BackendEvent::Failed(format!("{}: {error}", command.title())),
                };
                let _sent = events.send(event);
            });
            return;
        }
        let answer = !is_single_directive(text)
            && self.chat.as_ref().is_some_and(ConversationHost::is_ready);
        if answer && self.chat.as_ref().is_some_and(ConversationHost::is_running) {
            let _sent = self.emit(BackendEvent::Rejected(String::from(
                "a reply is still running; stop it first",
            )));
            return;
        }
        if let Err(error) = session.submit(text).await {
            let _sent = self.emit(BackendEvent::Rejected(error.to_string()));
            return;
        }
        if !answer {
            let _sent = self.emit(BackendEvent::Committed);
            return;
        }
        // Show the recorded prompt before the reply starts streaming.
        let records = records_until_own_input(session).await;
        match records {
            Ok(records) => {
                for record in records {
                    self.record(Ok(Some(record)));
                }
            }
            Err(error) => self.record(Err(error)),
        }
        let _sent = self.emit(BackendEvent::Committed);
        let (Some(session), Some(chat)) = (self.session.as_ref(), self.chat.as_mut()) else {
            return;
        };
        let event = match chat.start_turn(session, text.to_owned(), self.plan).await {
            Ok(()) => BackendEvent::ReplyStarted,
            Err(error) => BackendEvent::ReplyFailed(error.to_string()),
        };
        let _sent = self.emit(event);
    }

    fn set_effort(&mut self, effort: polaris_sdk::effort::Effort) {
        let event = match self.chat.as_mut() {
            Some(chat) => match chat.set_effort(effort) {
                Ok(model) => BackendEvent::ModelSwitched { model, effort },
                Err(reason) => BackendEvent::Failed(reason),
            },
            None => BackendEvent::Failed(String::from("model replies are not enabled")),
        };
        let _sent = self.emit(event);
    }

    async fn stats(&self) {
        let event = match list_sessions(&self.state_dir, 10_000).await {
            Ok(rows) => {
                let now = jiff::Zoned::now();
                BackendEvent::Stats(polaris_ui_session::stats::stats_lines(
                    &rows,
                    now.date(),
                    now.time_zone(),
                ))
            }
            Err(error) => BackendEvent::Failed(describe("Reading activity failed", &error)),
        };
        let _sent = self.emit(event);
    }

    async fn review(&mut self, request_id: &str, apply: bool, note: Option<String>) {
        let (Some(review), Some(session)) = (self.reviews.get(request_id), self.session.as_ref())
        else {
            let _sent = self.emit(BackendEvent::Failed(
                "That change is no longer waiting for a decision.".to_owned(),
            ));
            return;
        };
        if let Err(error) = answer_review(session.kernel(), review, apply, note).await {
            let _sent = self.emit(BackendEvent::Failed(error.to_string()));
        }
    }

    fn host_event(&mut self, event: HostEvent) {
        let event = match event {
            HostEvent::ProviderReady {
                provider,
                model,
                effort,
                models,
                ..
            } => BackendEvent::ProviderReady {
                provider: provider.to_owned(),
                model,
                effort,
                models,
            },
            HostEvent::ProviderUnavailable(reason) => BackendEvent::ProviderUnavailable(reason),
            HostEvent::Progress(ChatProgress::Text(text)) => BackendEvent::ReplyText(text),
            HostEvent::Progress(ChatProgress::Request) => BackendEvent::ReplyStep,
            HostEvent::Progress(ChatProgress::Reasoning(_)) => return,
            HostEvent::Progress(ChatProgress::Review(review)) => {
                let request_id = review.request_id.to_string();
                let prompt = review.prompt.clone();
                self.reviews.insert(request_id.clone(), review);
                BackendEvent::Review { request_id, prompt }
            }
            HostEvent::Progress(ChatProgress::ReviewClosed(id)) => {
                let request_id = id.to_string();
                self.reviews.remove(&request_id);
                BackendEvent::ReviewClosed(request_id)
            }
            HostEvent::Progress(ChatProgress::Applied(path)) => BackendEvent::Applied(path),
            HostEvent::Progress(ChatProgress::Deleted(path)) => BackendEvent::Deleted(path),
            HostEvent::Finished(report) => {
                self.reviews.clear();
                BackendEvent::ReplyFinished {
                    summary: report.summary(),
                    answer: report.answer,
                }
            }
            HostEvent::Failed(reason) => {
                self.reviews.clear();
                BackendEvent::ReplyFailed(reason)
            }
        };
        let _sent = self.emit(event);
    }

    async fn list(&self) {
        let event = match list_sessions(&self.state_dir, LIST_LIMIT).await {
            Ok(rows) => BackendEvent::Sessions(rows.iter().map(summary).collect()),
            Err(error) => BackendEvent::Failed(describe("Listing sessions failed", &error)),
        };
        let _sent = self.emit(event);
    }

    async fn search(&self, query: &str) {
        let titles = match list_sessions(&self.state_dir, LIST_LIMIT).await {
            Ok(rows) => rows,
            Err(_) => Vec::new(),
        };
        let event = match search_history(&self.state_dir, query, SEARCH_LIMIT).await {
            Ok(hits) => BackendEvent::Hits(
                hits.into_iter()
                    .map(|hit| SearchHit {
                        session_id: hit.session.to_string(),
                        title: match titles.iter().find(|row| row.id == hit.session) {
                            Some(row) => row.name.clone(),
                            None => String::new(),
                        },
                        snippet: hit.snippet,
                    })
                    .collect(),
            ),
            Err(error) => BackendEvent::Failed(describe("Search failed", &error)),
        };
        let _sent = self.emit(event);
    }

    /// Forwards one resolved record; `false` when the stream ended.
    fn record(
        &mut self,
        next: Result<Option<polaris_ui_session::ResolvedRecord>, SessionHostError>,
    ) {
        match next {
            Ok(Some(resolved)) => {
                let Some(open) = self.session.as_ref() else {
                    return;
                };
                if resolved.record.session() != open_id(open) {
                    return;
                }
                if let polaris_sdk::contracts::Event::ModelRequestCompleted {
                    prompt_tokens,
                    completion_tokens,
                    ..
                } = resolved.record.event()
                {
                    let (requests, prompt, completion) = self.usage;
                    self.usage = (
                        requests.saturating_add(1),
                        prompt.saturating_add(u64::from(prompt_tokens.get())),
                        completion.saturating_add(u64::from(completion_tokens.get())),
                    );
                    let _sent = self.emit(BackendEvent::Usage {
                        requests: self.usage.0,
                        prompt_tokens: self.usage.1,
                        completion_tokens: self.usage.2,
                    });
                }
                if let (Some(text), Some(kind)) = (resolved.text, resolved.text_kind) {
                    let kind = match kind {
                        RecordTextKind::Directive => MessageKind::Directive,
                        RecordTextKind::UserInput => MessageKind::UserInput,
                    };
                    let _sent = self.emit(BackendEvent::Message { kind, text });
                }
            }
            Ok(None) => {
                self.session = None;
                let _sent = self.emit(BackendEvent::Failed(String::from(
                    "The session's event stream ended; reopen the session.",
                )));
                let _closed = self.emit(BackendEvent::Closed);
            }
            Err(error) => {
                let _sent = self.emit(BackendEvent::Failed(describe(
                    "Reading the session failed",
                    &error,
                )));
            }
        }
    }
}

fn open_id(session: &OfflineSession) -> SessionId {
    session.id
}

async fn next_record(
    session: &mut Option<OfflineSession>,
) -> Result<Option<polaris_ui_session::ResolvedRecord>, SessionHostError> {
    match session.as_mut() {
        Some(session) => session.next_record().await,
        // Never resolves: the select arm is disabled while no session is open.
        None => std::future::pending().await,
    }
}

async fn next_remote_record(
    remote: &mut Option<RemoteSession>,
) -> Result<Option<polaris_ui_session::ResolvedRecord>, SessionHostError> {
    match remote.as_mut() {
        Some(remote) => remote.next_record().await,
        // Never resolves: nothing is attached.
        None => std::future::pending().await,
    }
}

async fn next_host_event(chat: &mut Option<ConversationHost>) -> HostEvent {
    match chat.as_mut() {
        Some(chat) => chat.next_event().await,
        // Never resolves: chat is disabled.
        None => std::future::pending().await,
    }
}

async fn run(
    state_dir: PathBuf,
    workspace: PathBuf,
    chat: Option<ChatOptions>,
    attach: Option<AttachTarget>,
    requests: Receiver<BackendRequest>,
    events: Sender<BackendEvent>,
) {
    let (forward_tx, mut forward_rx) = tokio::sync::mpsc::unbounded_channel::<BackendRequest>();
    let forwarder = tokio::task::spawn_blocking(move || {
        while let Ok(request) = requests.recv() {
            if forward_tx.send(request).is_err() {
                break;
            }
        }
    });
    let mut work = Bridgework {
        state_dir,
        workspace,
        events,
        session: None,
        chat: chat.map(|options| ConversationHost::new(options.config, options.env_file)),
        usage: (0, 0, 0),
        plan: true,
        reviews: std::collections::BTreeMap::new(),
        attach,
        remote: None,
    };
    if work.attach.is_some() {
        let _sent = work.emit(BackendEvent::ProviderUnavailable(String::from(ATTACHED)));
    } else if let Some(chat) = work.chat.as_mut() {
        chat.start_pick();
    }
    loop {
        tokio::select! {
            request = forward_rx.recv() => {
                let Some(request) = request else {
                    break;
                };
                let attached = work.attach.clone();
                match request {
                    BackendRequest::Open { .. } if attached.is_some() => {
                        if let Some(target) = attached {
                            work.open_remote(target).await;
                        }
                    }
                    BackendRequest::Submit { text } if attached.is_some() => {
                        work.submit_remote(&text).await;
                    }
                    BackendRequest::List if attached.is_some() => work.remote_list(),
                    BackendRequest::Search { .. } | BackendRequest::Stats if attached.is_some() => {
                        let _sent = work.emit(BackendEvent::Failed(String::from(LOCAL_ONLY)));
                    }
                    BackendRequest::Open { resume } => work.open(resume).await,
                    BackendRequest::Submit { text } => work.submit(&text).await,
                    BackendRequest::List => work.list().await,
                    BackendRequest::Search { query } => work.search(&query).await,
                    BackendRequest::Close => work.close_session().await,
                    BackendRequest::Interrupt => {
                        if let Some(chat) = work.chat.as_ref() {
                            chat.cancel_turn();
                        }
                    }
                    BackendRequest::SetEffort(effort) => work.set_effort(effort),
                    BackendRequest::Stats => work.stats().await,
                    BackendRequest::Mode { plan } => work.plan = plan,
                    BackendRequest::Review { request_id, apply, note } => {
                        work.review(&request_id, apply, note).await;
                    }
                }
            }
            next = next_record(&mut work.session) => work.record(next),
            next = next_remote_record(&mut work.remote) => work.remote_record(next),
            event = next_host_event(&mut work.chat) => work.host_event(event),
        }
    }
    if let Some(chat) = work.chat.as_mut() {
        chat.shutdown().await;
    }
    work.close_session().await;
    if forwarder.await.is_err() {
        tracing::warn!("bridge request forwarder failed");
    }
}

/// Default session store for `workspace`: shared with the terminal app.
#[must_use]
pub fn default_state_dir(workspace: &Path) -> PathBuf {
    workspace.join(".polaris").join("state")
}
