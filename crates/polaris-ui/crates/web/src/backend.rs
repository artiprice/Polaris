//! The web UI's session owner: one task holding the [`OfflineSession`] and the shared
//! [`ConversationHost`], fed by HTTP commands and publishing [`WebEvent`]s to every open page.
use polaris_sdk::{contracts::Event, effort::Effort, ids::SessionId};
use polaris_ui_session::{
    ChangeReview, ChatProgress, ConversationHost, HostEvent, OfflineSession, RecordTextKind,
    ResolvedRecord, SessionHostError, answer_review, is_single_directive, list_sessions,
    records_until_own_input,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Events replayed to a page that connects (or reloads) mid-session.
const REPLAY_LIMIT: usize = 4096;
/// Sessions listed in the sidebar.
const LIST_LIMIT: u32 = 100;

/// What a page asks the session owner to do.
#[derive(Debug)]
pub enum Command {
    /// Start a new session or reopen one by id.
    Open { resume: Option<String> },
    /// Record a message; typed text is answered when a provider is available.
    Submit { text: String, plan: bool },
    /// Stop the running reply.
    Interrupt,
    /// Continue on the provider's model for this effort.
    SetEffort(Effort),
    /// Close the open session.
    Close,
    /// Apply or decline the change a reply waits on; `note` reaches the model with a decline.
    Review {
        request_id: String,
        apply: bool,
        note: Option<String>,
    },
    /// Report the session list.
    List {
        reply: oneshot::Sender<Vec<SessionRow>>,
    },
}

/// One sidebar row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionRow {
    /// Session id.
    pub id: String,
    /// Display name (may be empty).
    pub title: String,
    /// Workspace path.
    pub workspace: String,
    /// Last update, Unix milliseconds.
    pub updated_ms: i64,
}

/// What every open page receives (Server-Sent Events, JSON payloads).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WebEvent {
    /// A provider was selected.
    Provider {
        provider: String,
        model: String,
        effort: String,
        models: Vec<(String, String)>,
    },
    /// No provider is usable.
    ProviderUnavailable { reason: String },
    /// A different model now answers.
    ModelSwitched { model: String, effort: String },
    /// The kernel is opening a session.
    Opening,
    /// A session is open.
    Opened {
        id: String,
        title: String,
        workspace: String,
    },
    /// The open session closed.
    Closed,
    /// A committed message of the open session.
    Message { kind: String, text: String },
    /// The last submit was recorded.
    Committed,
    /// The last submit was rejected; the draft must be kept.
    Rejected { reason: String },
    /// A reply started.
    ReplyStarted,
    /// Streamed reply text.
    ReplyText { text: String },
    /// Another model request began after a tool round.
    ReplyStep,
    /// The reply finished.
    ReplyFinished {
        answer: Option<String>,
        summary: String,
    },
    /// The reply failed or was interrupted.
    ReplyFailed { reason: String },
    /// The reply waits until the person applies or declines this change (Markdown prompt with
    /// a diff).
    Review { request_id: String, prompt: String },
    /// The review was answered or withdrawn.
    ReviewClosed { request_id: String },
    /// A reviewed change was written to this workspace path.
    Applied { path: String },
    /// Token use of the open session.
    Usage {
        requests: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
    },
    /// Something the page should show as a notice.
    Notice { text: String },
}

/// Events since the provider was chosen and the session opened, for pages that connect later.
#[derive(Debug, Default)]
pub struct Replay {
    provider: Option<WebEvent>,
    session: VecDeque<WebEvent>,
}

impl Replay {
    fn record(&mut self, event: &WebEvent) {
        match event {
            WebEvent::Provider { .. } | WebEvent::ProviderUnavailable { .. } => {
                self.provider = Some(event.clone());
            }
            WebEvent::ModelSwitched { model, effort } => {
                if let Some(WebEvent::Provider {
                    model: current,
                    effort: current_effort,
                    ..
                }) = self.provider.as_mut()
                {
                    current.clone_from(model);
                    current_effort.clone_from(effort);
                }
            }
            WebEvent::Opening | WebEvent::Closed => self.session.clear(),
            WebEvent::Notice { .. } | WebEvent::Rejected { .. } => {}
            _ => {
                if self.session.len() >= REPLAY_LIMIT {
                    let _oldest = self.session.pop_front();
                }
                self.session.push_back(event.clone());
            }
        }
    }

    /// Provider state first, then the open session's events in order.
    #[must_use]
    pub fn snapshot(&self) -> Vec<WebEvent> {
        self.provider
            .iter()
            .cloned()
            .chain(self.session.iter().cloned())
            .collect()
    }
}

/// Shared publication point: records for replay, then broadcasts.
#[derive(Debug, Clone)]
pub struct Publisher {
    events: broadcast::Sender<WebEvent>,
    replay: Arc<Mutex<Replay>>,
}

impl Publisher {
    /// A publisher with room for `capacity` undelivered events per page.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (events, _receiver) = broadcast::channel(capacity);
        Self {
            events,
            replay: Arc::new(Mutex::new(Replay::default())),
        }
    }

    /// Replay snapshot and a live receiver taken under one lock, so nothing is missed between.
    pub fn subscribe(&self) -> (Vec<WebEvent>, broadcast::Receiver<WebEvent>) {
        match self.replay.lock() {
            Ok(replay) => (replay.snapshot(), self.events.subscribe()),
            Err(poisoned) => (poisoned.into_inner().snapshot(), self.events.subscribe()),
        }
    }

    fn publish(&self, event: WebEvent) {
        match self.replay.lock() {
            Ok(mut replay) => {
                replay.record(&event);
                let _receivers = self.events.send(event);
            }
            Err(poisoned) => {
                poisoned.into_inner().record(&event);
                let _receivers = self.events.send(event);
            }
        }
    }
}

/// Session store, workspace and chat configuration of the web UI.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    /// Session store root.
    pub state_dir: PathBuf,
    /// Workspace of new sessions.
    pub workspace: PathBuf,
    /// Layered configuration (provider order and models).
    pub config: polaris_config::PolarisConfig,
    /// Dotenv file with provider keys.
    pub env_file: Option<PathBuf>,
}

struct Owner {
    config: BackendConfig,
    publisher: Publisher,
    session: Option<OfflineSession>,
    chat: ConversationHost,
    usage: (u64, u64, u64),
    /// Changes the running reply waits on, by request id.
    reviews: BTreeMap<String, ChangeReview>,
}

impl Owner {
    async fn review(&mut self, request_id: &str, apply: bool, note: Option<String>) {
        let (Some(review), Some(session)) = (self.reviews.get(request_id), self.session.as_ref())
        else {
            self.publisher.publish(WebEvent::Notice {
                text: "That change is no longer waiting for a decision.".to_owned(),
            });
            return;
        };
        if let Err(error) = answer_review(session.kernel(), review, apply, note).await {
            self.publisher.publish(WebEvent::Notice {
                text: error.to_string(),
            });
        }
    }

    async fn open(&mut self, resume: Option<String>) {
        if self.chat.is_running() {
            self.publisher.publish(WebEvent::Notice {
                text: "A reply is still running; stop it first.".to_owned(),
            });
            return;
        }
        self.publisher.publish(WebEvent::Opening);
        self.close().await;
        let id = match resume.as_deref().map(str::parse::<SessionId>) {
            None => SessionId::new_v7(),
            Some(Ok(id)) => id,
            Some(Err(_)) => {
                self.publisher.publish(WebEvent::Notice {
                    text: "Unknown session id.".to_owned(),
                });
                return;
            }
        };
        self.usage = (0, 0, 0);
        match OfflineSession::open(&self.config.state_dir, &self.config.workspace, id).await {
            Ok(session) => {
                let workspace = session.workspace().display().to_string();
                self.session = Some(session);
                let title = match list_sessions(&self.config.state_dir, LIST_LIMIT).await {
                    Ok(rows) => rows
                        .into_iter()
                        .find(|row| row.id == id)
                        .map(|row| row.name)
                        .unwrap_or_default(),
                    Err(_) => String::new(),
                };
                self.publisher.publish(WebEvent::Opened {
                    id: id.to_string(),
                    title,
                    workspace,
                });
            }
            Err(error) => self.publisher.publish(WebEvent::Notice {
                text: format!(
                    "Could not open the session (is another Polaris process using this store?): {error}"
                ),
            }),
        }
    }

    async fn close(&mut self) {
        self.chat.reset_conversation();
        if let Some(mut session) = self.session.take() {
            if let Err(error) = session.close().await {
                tracing::warn!(%error, "web session close failed");
            }
            self.publisher.publish(WebEvent::Closed);
        }
    }

    async fn submit(&mut self, text: String, plan: bool) {
        let Some(session) = self.session.as_mut() else {
            self.publisher.publish(WebEvent::Rejected {
                reason: "No session is open.".to_owned(),
            });
            return;
        };
        let answer = !is_single_directive(&text) && self.chat.is_ready();
        if answer && self.chat.is_running() {
            self.publisher.publish(WebEvent::Rejected {
                reason: "A reply is still running; stop it first.".to_owned(),
            });
            return;
        }
        if let Err(error) = session.submit(&text).await {
            self.publisher.publish(WebEvent::Rejected {
                reason: error.to_string(),
            });
            return;
        }
        if !answer {
            self.publisher.publish(WebEvent::Committed);
            return;
        }
        match records_until_own_input(session).await {
            Ok(records) => {
                for record in records {
                    self.record(Ok(Some(record)));
                }
            }
            Err(error) => self.record(Err(error)),
        }
        self.publisher.publish(WebEvent::Committed);
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let event = match self.chat.start_turn(session, text, plan).await {
            Ok(()) => WebEvent::ReplyStarted,
            Err(error) => WebEvent::ReplyFailed {
                reason: error.to_string(),
            },
        };
        self.publisher.publish(event);
    }

    fn record(&mut self, next: Result<Option<ResolvedRecord>, SessionHostError>) {
        match next {
            Ok(Some(resolved)) => {
                let Some(open) = self.session.as_ref() else {
                    return;
                };
                if resolved.record.session() != open.id {
                    return;
                }
                if let Event::ModelRequestCompleted {
                    prompt_tokens,
                    completion_tokens,
                    ..
                } = resolved.record.event()
                {
                    self.usage = (
                        self.usage.0.saturating_add(1),
                        self.usage.1.saturating_add(u64::from(prompt_tokens.get())),
                        self.usage
                            .2
                            .saturating_add(u64::from(completion_tokens.get())),
                    );
                    self.publisher.publish(WebEvent::Usage {
                        requests: self.usage.0,
                        prompt_tokens: self.usage.1,
                        completion_tokens: self.usage.2,
                    });
                }
                if let (Some(text), Some(kind)) = (resolved.text, resolved.text_kind) {
                    let kind = match kind {
                        RecordTextKind::Directive => "directive",
                        RecordTextKind::UserInput => "user",
                    };
                    self.publisher.publish(WebEvent::Message {
                        kind: kind.to_owned(),
                        text,
                    });
                }
            }
            Ok(None) => {
                self.session = None;
                self.publisher.publish(WebEvent::Notice {
                    text: "The session's event stream ended; reopen the session.".to_owned(),
                });
                self.publisher.publish(WebEvent::Closed);
            }
            Err(error) => self.publisher.publish(WebEvent::Notice {
                text: format!("Reading the session failed: {error}"),
            }),
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
            } => WebEvent::Provider {
                provider: provider.to_owned(),
                model,
                effort: effort.name().to_owned(),
                models: models
                    .into_iter()
                    .map(|(effort, model)| (effort.name().to_owned(), model))
                    .collect(),
            },
            HostEvent::ProviderUnavailable(reason) => WebEvent::ProviderUnavailable { reason },
            HostEvent::Progress(ChatProgress::Text(text)) => WebEvent::ReplyText { text },
            HostEvent::Progress(ChatProgress::Request) => WebEvent::ReplyStep,
            HostEvent::Progress(ChatProgress::Reasoning(_)) => return,
            HostEvent::Progress(ChatProgress::Review(review)) => {
                let request_id = review.request_id.to_string();
                let prompt = review.prompt.clone();
                self.reviews.insert(request_id.clone(), review);
                WebEvent::Review { request_id, prompt }
            }
            HostEvent::Progress(ChatProgress::ReviewClosed(id)) => {
                let request_id = id.to_string();
                self.reviews.remove(&request_id);
                WebEvent::ReviewClosed { request_id }
            }
            HostEvent::Progress(ChatProgress::Applied(path)) => WebEvent::Applied { path },
            HostEvent::Finished(report) => {
                self.reviews.clear();
                WebEvent::ReplyFinished {
                    summary: report.summary(),
                    answer: report.answer,
                }
            }
            HostEvent::Failed(reason) => {
                self.reviews.clear();
                WebEvent::ReplyFailed { reason }
            }
        };
        self.publisher.publish(event);
    }

    async fn list(&self) -> Vec<SessionRow> {
        match list_sessions(&self.config.state_dir, LIST_LIMIT).await {
            Ok(rows) => rows
                .into_iter()
                .map(|row| SessionRow {
                    id: row.id.to_string(),
                    title: row.name,
                    workspace: row.workspace.display().to_string(),
                    updated_ms: row.updated_at.as_millisecond(),
                })
                .collect(),
            Err(error) => {
                self.publisher.publish(WebEvent::Notice {
                    text: format!("Listing sessions failed: {error}"),
                });
                Vec::new()
            }
        }
    }
}

async fn next_record(
    session: &mut Option<OfflineSession>,
) -> Result<Option<ResolvedRecord>, SessionHostError> {
    match session.as_mut() {
        Some(session) => session.next_record().await,
        None => std::future::pending().await,
    }
}

/// Run the session owner until `commands` closes; then stop the reply and close the session.
pub async fn run(
    config: BackendConfig,
    publisher: Publisher,
    mut commands: mpsc::Receiver<Command>,
) {
    let chat = ConversationHost::new(config.config.clone(), config.env_file.clone());
    let mut owner = Owner {
        config,
        publisher,
        session: None,
        chat,
        usage: (0, 0, 0),
        reviews: BTreeMap::new(),
    };
    owner.chat.start_pick();
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Open { resume } => owner.open(resume).await,
                    Command::Submit { text, plan } => owner.submit(text, plan).await,
                    Command::Interrupt => owner.chat.cancel_turn(),
                    Command::SetEffort(effort) => {
                        let event = match owner.chat.set_effort(effort) {
                            Ok(model) => WebEvent::ModelSwitched {
                                model,
                                effort: effort.name().to_owned(),
                            },
                            Err(text) => WebEvent::Notice { text },
                        };
                        owner.publisher.publish(event);
                    }
                    Command::Close => owner.close().await,
                    Command::Review { request_id, apply, note } => {
                        owner.review(&request_id, apply, note).await;
                    }
                    Command::List { reply } => {
                        let _delivered = reply.send(owner.list().await);
                    }
                }
            }
            next = next_record(&mut owner.session) => owner.record(next),
            event = owner.chat.next_event() => owner.host_event(event),
        }
    }
    owner.chat.shutdown().await;
    owner.close().await;
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/web/src/backend.rs"]
mod tests;
