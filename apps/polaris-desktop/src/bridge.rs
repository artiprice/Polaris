//! Kernel bridge: implements the window's [`polaris_ui_gui::backend`] contract over an offline
//! kernel session ([`polaris_ui_session::OfflineSession`]).
//!
//! One background thread owns a tokio runtime. Requests arrive on a std channel (forwarded into
//! the runtime by a blocking task); one loop selects over requests and the open session's record
//! stream, and reports [`BackendEvent`]s back on another std channel that the UI thread drains.
//! The kernel starts lazily on the first `Open`, so the welcome screen never waits for it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use polaris_sdk::ids::SessionId;
use polaris_ui_gui::backend::{
    BackendEvent, BackendLink, BackendRequest, MessageKind, SearchHit, SessionSummary,
};
use polaris_ui_session::{
    OfflineSession, RecordTextKind, SessionHostError, SessionMeta, list_sessions, search_history,
};

use crate::DesktopError;

/// Rows shown in the sidebar (newest first).
const LIST_LIMIT: u32 = 100;
/// Search hits returned per query.
const SEARCH_LIMIT: u32 = 50;

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

/// Starts the bridge thread for the session store at `state_dir` and new sessions in
/// `workspace`.
///
/// # Errors
/// [`DesktopError::Bridge`] when the thread or its runtime cannot start.
pub fn spawn(
    state_dir: PathBuf,
    workspace: PathBuf,
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
            runtime.block_on(run(state_dir, workspace, request_rx, event_tx));
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
}

impl Bridgework {
    /// `false` once the window is gone (nobody to report to).
    fn emit(&self, event: BackendEvent) -> bool {
        self.events.send(event).is_ok()
    }

    async fn close_session(&mut self) {
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

    async fn submit(&self, text: &str) {
        let Some(session) = self.session.as_ref() else {
            let _sent = self.emit(BackendEvent::Rejected(String::from("no session is open")));
            return;
        };
        let event = match session.submit(text).await {
            Ok(_receipt) => BackendEvent::Committed,
            Err(error) => BackendEvent::Rejected(error.to_string()),
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

async fn run(
    state_dir: PathBuf,
    workspace: PathBuf,
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
    };
    loop {
        tokio::select! {
            request = forward_rx.recv() => {
                let Some(request) = request else {
                    break;
                };
                match request {
                    BackendRequest::Open { resume } => work.open(resume).await,
                    BackendRequest::Submit { text } => work.submit(&text).await,
                    BackendRequest::List => work.list().await,
                    BackendRequest::Search { query } => work.search(&query).await,
                    BackendRequest::Close => work.close_session().await,
                }
            }
            next = next_record(&mut work.session) => work.record(next),
        }
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
