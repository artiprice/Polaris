//! The page's state and how server events and clicks change it. Pure: every network call is
//! returned as an [`Effect`] for the browser client to perform, so the whole flow is testable
//! natively.

use crate::events::{SessionRow, WebEvent};

/// Model connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Connection {
    /// Provider selection is running.
    #[default]
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

/// How a review ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewState {
    /// Waiting for the person.
    Waiting,
    /// The person decided; the answer is on its way.
    Deciding {
        /// Apply rather than decline.
        apply: bool,
    },
    /// Applied.
    Applied,
    /// Declined.
    Declined,
    /// The reply ended before a decision.
    Withdrawn,
}

/// One conversation entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// Typed input.
    User(String),
    /// A `$directive`.
    Directive(String),
    /// Model text (Markdown); `streaming` while the reply runs.
    Assistant {
        /// Text so far.
        text: String,
        /// Still receiving text.
        streaming: bool,
    },
    /// A neutral line (summaries, notes).
    System(String),
    /// A stopped reply.
    Warning(String),
    /// A failure.
    Error(String),
    /// A file the reply changed.
    Change(String),
    /// A change waiting for, or past, a decision.
    Review {
        /// Request id.
        request_id: String,
        /// Markdown prompt with the diff.
        prompt: String,
        /// Where the decision stands.
        state: ReviewState,
    },
}

/// An entry with a stable id for keyed rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Unique within the page.
    pub id: u64,
    /// Content.
    pub kind: EntryKind,
}

/// The open session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSession {
    /// Session id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Workspace path.
    pub workspace: String,
}

/// Network work for the browser client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// `POST /api/open` (a new session, or `resume`).
    Open {
        /// Session to reopen.
        resume: Option<String>,
    },
    /// `POST /api/submit`.
    Submit {
        /// Message text.
        text: String,
        /// Plan mode.
        plan: bool,
    },
    /// `POST /api/interrupt`.
    Interrupt,
    /// `POST /api/review`.
    Review {
        /// Request id.
        request_id: String,
        /// Apply rather than decline.
        apply: bool,
        /// Note for the model.
        note: Option<String>,
    },
    /// `POST /api/effort`.
    Effort(String),
    /// `GET /api/sessions` into [`UiState::sessions`].
    RefreshSessions,
}

/// Everything the page shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiState {
    /// Model connection.
    pub connection: Connection,
    /// `(effort, model)` choices.
    pub models: Vec<(String, String)>,
    /// Current effort.
    pub effort: String,
    /// The open session.
    pub session: Option<OpenSession>,
    /// Conversation, oldest first.
    pub entries: Vec<Entry>,
    /// A reply is running.
    pub working: bool,
    /// Composer text.
    pub composer: String,
    /// Plan mode for the next message.
    pub plan: bool,
    /// A message on its way: `(text, plan)`.
    pub pending: Option<(String, bool)>,
    /// A notice above the status line.
    pub notice: Option<String>,
    /// Requests, prompt tokens, completion tokens of the session.
    pub usage: (u64, u64, u64),
    /// Sidebar rows.
    pub sessions: Vec<SessionRow>,
    /// Entry receiving streamed text.
    reply: Option<usize>,
    /// A review card ended the streamed text; a later answer goes below it.
    after_review: bool,
    next_id: u64,
}

impl UiState {
    fn push(&mut self, kind: EntryKind) {
        self.next_id = self.next_id.saturating_add(1);
        self.entries.push(Entry {
            id: self.next_id,
            kind,
        });
    }

    fn clear_conversation(&mut self) {
        self.entries.clear();
        self.reply = None;
        self.after_review = false;
    }

    /// Whether a change waits for a decision.
    #[must_use]
    pub fn waiting_for_review(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry.kind,
                EntryKind::Review {
                    state: ReviewState::Waiting | ReviewState::Deciding { .. },
                    ..
                }
            )
        })
    }

    /// Status dot state and line; `elapsed_s` is the running reply's age.
    #[must_use]
    pub fn status(&self, elapsed_s: u64) -> (&'static str, String) {
        if self.waiting_for_review() {
            return (
                "waiting",
                "Waiting for your decision on a change".to_owned(),
            );
        }
        if self.working {
            return ("", format!("Working {elapsed_s}s · Esc to stop"));
        }
        match &self.connection {
            Connection::Connecting => ("", "Connecting to a model…".to_owned()),
            Connection::Ready { provider, model } => ("ready", format!("{model} · {provider}")),
            Connection::Offline(_) if self.attached() => {
                ("offline", "polarisd · no model replies".to_owned())
            }
            Connection::Offline(_) => ("offline", "Offline · no model replies".to_owned()),
        }
    }

    /// Whether the server records into a `polarisd` session (its provider-unavailable reason
    /// says so) rather than a local one.
    #[must_use]
    pub fn attached(&self) -> bool {
        matches!(&self.connection, Connection::Offline(reason) if reason.starts_with("Attached to polarisd"))
    }

    /// Send the composer text, or stop the running reply.
    pub fn submit(&mut self) -> Vec<Effect> {
        if self.working {
            self.notice = Some("Stopping…".to_owned());
            return vec![Effect::Interrupt];
        }
        let text = self.composer.clone();
        if text.trim().is_empty() || self.pending.is_some() {
            return Vec::new();
        }
        self.pending = Some((text.clone(), self.plan));
        self.notice = None;
        match self.session {
            Some(_) => vec![Effect::Submit {
                text,
                plan: self.plan,
            }],
            None => vec![Effect::Open { resume: None }],
        }
    }

    /// Apply or decline a waiting change.
    pub fn decide(&mut self, request_id: &str, apply: bool, note: &str) -> Vec<Effect> {
        let Some(EntryKind::Review { state, .. }) = self.entries.iter_mut().map(|entry| &mut entry.kind).find(
            |kind| matches!(kind, EntryKind::Review { request_id: id, state: ReviewState::Waiting, .. } if id == request_id),
        ) else {
            return Vec::new();
        };
        *state = ReviewState::Deciding { apply };
        let note = Some(note.trim().to_owned()).filter(|note| !note.is_empty());
        vec![Effect::Review {
            request_id: request_id.to_owned(),
            apply,
            note,
        }]
    }

    /// Reopen a listed session.
    pub fn open_listed(&mut self, id: &str) -> Vec<Effect> {
        if self.working {
            self.notice = Some("A reply is still running; stop it first.".to_owned());
            return Vec::new();
        }
        vec![Effect::Open {
            resume: Some(id.to_owned()),
        }]
    }

    /// Continue on the model for `effort`.
    pub fn choose_effort(&mut self, effort: &str) -> Vec<Effect> {
        if effort == self.effort {
            return Vec::new();
        }
        if self.working {
            self.notice = Some("Change the model after the reply finishes.".to_owned());
            return Vec::new();
        }
        vec![Effect::Effort(effort.to_owned())]
    }

    fn close_reviews(&mut self) {
        for entry in &mut self.entries {
            if let EntryKind::Review { state, .. } = &mut entry.kind {
                *state = match *state {
                    ReviewState::Waiting => ReviewState::Withdrawn,
                    ReviewState::Deciding { apply: true } => ReviewState::Applied,
                    ReviewState::Deciding { apply: false } => ReviewState::Declined,
                    done => done,
                };
            }
        }
    }

    /// End the streamed segment: keep its text, or drop it if nothing arrived.
    fn end_reply(&mut self) {
        if let Some(index) = self.reply.take() {
            let empty = matches!(
                self.entries.get(index).map(|entry| &entry.kind),
                Some(EntryKind::Assistant { text, .. }) if text.is_empty()
            );
            if empty {
                self.entries.remove(index);
            } else if let Some(EntryKind::Assistant { streaming, .. }) =
                self.entries.get_mut(index).map(|entry| &mut entry.kind)
            {
                *streaming = false;
            }
        }
    }

    fn reply_text(&mut self) -> Option<&mut String> {
        let index = self.reply?;
        match self.entries.get_mut(index).map(|entry| &mut entry.kind) {
            Some(EntryKind::Assistant { text, .. }) => Some(text),
            _ => None,
        }
    }

    /// Apply one server event.
    pub fn apply(&mut self, event: WebEvent) -> Vec<Effect> {
        match event {
            WebEvent::Provider {
                provider,
                model,
                effort,
                models,
            } => {
                self.connection = Connection::Ready { provider, model };
                self.models = models;
                self.effort = effort;
            }
            WebEvent::ProviderUnavailable { reason } => {
                self.connection = Connection::Offline(reason);
            }
            WebEvent::ModelSwitched { model, effort } => {
                if let Connection::Ready { model: current, .. } = &mut self.connection {
                    current.clone_from(&model);
                }
                self.notice = Some(format!("Model {model} · {effort} effort"));
                self.effort = effort;
            }
            WebEvent::Opening => self.clear_conversation(),
            WebEvent::Opened {
                id,
                title,
                workspace,
            } => {
                self.clear_conversation();
                self.working = false;
                self.session = Some(OpenSession {
                    id,
                    title,
                    workspace,
                });
                if let Connection::Offline(reason) = &self.connection {
                    let notice = if self.attached() {
                        reason.clone()
                    } else {
                        "Offline session · messages are recorded locally; no model replies."
                            .to_owned()
                    };
                    self.push(EntryKind::System(notice));
                }
                let mut effects = vec![Effect::RefreshSessions];
                if let Some((text, plan)) = self.pending.clone() {
                    effects.insert(0, Effect::Submit { text, plan });
                }
                return effects;
            }
            WebEvent::Closed => self.session = None,
            WebEvent::Message { kind, text } => self.push(match kind.as_str() {
                "user" => EntryKind::User(text),
                "directive" => EntryKind::Directive(text),
                _ => EntryKind::System(text),
            }),
            WebEvent::Committed => {
                if let Some((text, _)) = self.pending.take()
                    && self.composer == text
                {
                    self.composer.clear();
                }
                return vec![Effect::RefreshSessions];
            }
            WebEvent::Rejected { reason } => {
                self.pending = None;
                self.notice = Some(format!("Not sent: {reason}"));
            }
            WebEvent::ReplyStarted => {
                self.working = true;
                self.after_review = false;
                self.push(EntryKind::Assistant {
                    text: String::new(),
                    streaming: true,
                });
                self.reply = Some(self.entries.len().saturating_sub(1));
            }
            WebEvent::ReplyText { text } => {
                self.after_review = false;
                if self.reply.is_none() {
                    self.push(EntryKind::Assistant {
                        text: String::new(),
                        streaming: true,
                    });
                    self.reply = Some(self.entries.len().saturating_sub(1));
                }
                if let Some(current) = self.reply_text() {
                    current.push_str(&text);
                }
            }
            WebEvent::ReplyStep => {
                if let Some(current) = self.reply_text()
                    && !current.is_empty()
                {
                    current.push_str("\n\n");
                }
            }
            WebEvent::ReplyFinished { answer, summary } => {
                self.close_reviews();
                let answer = answer.filter(|answer| !answer.trim().is_empty());
                let streamed = self
                    .reply
                    .and_then(|index| self.entries.get(index))
                    .is_some_and(|entry| {
                        matches!(&entry.kind, EntryKind::Assistant { text, .. } if !text.trim().is_empty())
                    });
                if streamed {
                    self.end_reply();
                } else {
                    self.end_reply();
                    match answer {
                        Some(answer) => self.push(EntryKind::Assistant {
                            text: answer,
                            streaming: false,
                        }),
                        None if self.after_review => {}
                        None => self.push(EntryKind::Warning(
                            "The model stopped without an answer.".to_owned(),
                        )),
                    }
                }
                self.push(EntryKind::System(summary));
                self.working = false;
                self.after_review = false;
                if self.notice.as_deref() == Some("Stopping…") {
                    self.notice = None;
                }
                return vec![Effect::RefreshSessions];
            }
            WebEvent::ReplyFailed { reason } => {
                self.close_reviews();
                self.end_reply();
                if reason == "Interrupted" {
                    self.push(EntryKind::Warning("Stopped.".to_owned()));
                } else {
                    self.push(EntryKind::Error(reason));
                }
                self.working = false;
                self.after_review = false;
                if self.notice.as_deref() == Some("Stopping…") {
                    self.notice = None;
                }
            }
            WebEvent::Usage {
                requests,
                prompt_tokens,
                completion_tokens,
            } => self.usage = (requests, prompt_tokens, completion_tokens),
            WebEvent::Notice { text } => self.notice = Some(text),
            WebEvent::Report { title, lines } => self.push(EntryKind::Assistant {
                text: format!("**{title}**\n\n```text\n{}\n```", lines.join("\n")),
                streaming: false,
            }),
            WebEvent::Review { request_id, prompt } => {
                let known = self.entries.iter().any(|entry| {
                    matches!(&entry.kind, EntryKind::Review { request_id: id, .. } if *id == request_id)
                });
                if !known {
                    self.end_reply();
                    self.after_review = true;
                    self.push(EntryKind::Review {
                        request_id,
                        prompt,
                        state: ReviewState::Waiting,
                    });
                }
            }
            WebEvent::ReviewClosed { request_id } => {
                for entry in &mut self.entries {
                    if let EntryKind::Review {
                        request_id: id,
                        state,
                        ..
                    } = &mut entry.kind
                        && *id == request_id
                    {
                        *state = match *state {
                            ReviewState::Deciding { apply: true } => ReviewState::Applied,
                            ReviewState::Deciding { apply: false } => ReviewState::Declined,
                            ReviewState::Waiting => ReviewState::Withdrawn,
                            done => done,
                        };
                    }
                }
            }
            WebEvent::Applied { path } => self.push(EntryKind::Change(format!("Edited {path}"))),
            WebEvent::Deleted { path } => self.push(EntryKind::Change(format!("Deleted {path}"))),
            WebEvent::Unknown => {}
        }
        Vec::new()
    }
}
