//! Frontend-neutral conversation host: provider selection, the active session's
//! [`ChatAgent`], its running turn and model switching. The terminal and desktop frontends drive
//! the same host and only translate its [`HostEvent`]s into their own messages.
use crate::{
    ChatAgent, ChatError, ChatProgress, ChatSettings, ChatTurn, OfflineSession, RecordTextKind,
    ResolvedRecord, SessionHostError, chat::skipped_reasons, choose_provider,
};
use polaris_config::ProviderName;
use polaris_nebius_cloud_token_factory::{HostedChatModel, ProviderChoice, provider_model};
use polaris_sdk::{effort::Effort, ids::ModelId};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Sanitized summary of one finished turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    /// Final assistant text, if any.
    pub answer: Option<String>,
    /// Model requests made.
    pub attempts: u32,
    /// Tool rounds executed.
    pub tool_rounds: u32,
    /// Runtime stop reason.
    pub stop: String,
    /// Wall-clock duration of the turn.
    pub elapsed: Duration,
    /// Workspace paths of the changes applied in this turn, in order, each once.
    pub changed_files: Vec<String>,
}

impl TurnReport {
    /// "Worked for 3s · 2 requests · 1 tool round · 1 file changed".
    #[must_use]
    pub fn summary(&self) -> String {
        let requests = if self.attempts == 1 {
            "1 request".to_owned()
        } else {
            format!("{} requests", self.attempts)
        };
        let tools = match self.tool_rounds {
            0 => String::new(),
            1 => " · 1 tool round".to_owned(),
            rounds => format!(" · {rounds} tool rounds"),
        };
        let files = match self.changed_files.len() {
            0 => String::new(),
            1 => " · 1 file changed".to_owned(),
            count => format!(" · {count} files changed"),
        };
        format!(
            "Worked for {} · {requests}{tools}{files}",
            polaris_ui_model::view::format_duration_ms(
                u64::try_from(self.elapsed.as_millis()).unwrap_or(u64::MAX)
            )
        )
    }
}

/// What the host reports to its frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// A provider was selected for this run.
    ProviderReady {
        /// Provider name (`token_factory`, `openrouter`, `local`).
        provider: &'static str,
        /// Selected model id.
        model: String,
        /// Effort the model was chosen for.
        effort: Effort,
        /// The provider's model for every effort.
        models: Vec<(Effort, String)>,
        /// `provider: reason` for providers skipped before the selection.
        skipped: Vec<String>,
    },
    /// No provider is usable; the text lists the reasons.
    ProviderUnavailable(String),
    /// Live output of the running turn.
    Progress(ChatProgress),
    /// The running turn finished.
    Finished(TurnReport),
    /// The running turn failed or was interrupted (`"Interrupted"`).
    Failed(String),
}

/// Text of [`HostEvent::Failed`] after an interruption.
pub const INTERRUPTED: &str = "Interrupted";

struct ReadyModel {
    provider: ProviderName,
    effort: Effort,
    model_id: ModelId,
    model: Arc<HostedChatModel>,
    context_tokens: u32,
}

type TurnOutput = (ChatAgent<HostedChatModel>, Result<ChatTurn, ChatError>);

struct RunningTurn {
    handle: tokio::task::JoinHandle<TurnOutput>,
    cancel: CancellationToken,
    progress: mpsc::Receiver<ChatProgress>,
    progress_closed: bool,
    started: Instant,
    /// Deltas received while the join result was being collected.
    tail: Vec<ChatProgress>,
    /// Paths of applied changes, in order, each once.
    changed: Vec<String>,
}

impl RunningTurn {
    fn note(&mut self, item: &ChatProgress) {
        if let ChatProgress::Applied(path) = item
            && !self.changed.contains(path)
        {
            self.changed.push(path.clone());
        }
    }
}

/// Provider selection, one session's conversation and its running turn. One conversation is
/// bound to one session kernel: call [`Self::reset_conversation`] after switching sessions.
pub struct ConversationHost {
    config: polaris_config::PolarisConfig,
    env_file: Option<PathBuf>,
    pick: Option<tokio::task::JoinHandle<Result<ProviderChoice, ChatError>>>,
    ready: Option<ReadyModel>,
    unavailable: Option<String>,
    agent: Option<ChatAgent<HostedChatModel>>,
    turn: Option<RunningTurn>,
    /// Events computed but not yet returned by [`Self::next_event`].
    queued: std::collections::VecDeque<HostEvent>,
}

impl std::fmt::Debug for ConversationHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationHost")
            .field("ready", &self.ready.as_ref().map(|ready| &ready.model_id))
            .field("running", &self.turn.is_some())
            .finish_non_exhaustive()
    }
}

/// Default dotenv file: an explicit path, else `<global>/.env` when it exists. A repository
/// `.env` is never read implicitly: an untrusted checkout must not choose the account that
/// receives the user's prompts.
#[must_use]
pub fn default_env_file(explicit: Option<&Path>, global_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_owned());
    }
    let global = global_dir.join(".env");
    global.is_file().then_some(global)
}

impl ConversationHost {
    /// A host that has not contacted any provider yet.
    #[must_use]
    pub fn new(config: polaris_config::PolarisConfig, env_file: Option<PathBuf>) -> Self {
        Self {
            config,
            env_file,
            pick: None,
            ready: None,
            unavailable: None,
            agent: None,
            turn: None,
            queued: std::collections::VecDeque::new(),
        }
    }

    /// Start provider selection once; later calls are no-ops. Must run inside a runtime.
    pub fn start_pick(&mut self) {
        if self.pick.is_some() || self.ready.is_some() || self.unavailable.is_some() {
            return;
        }
        let config = self.config.clone();
        let env_file = self.env_file.clone();
        self.pick = Some(tokio::spawn(async move {
            choose_provider(&config, env_file.as_deref(), ChatSettings::default().effort).await
        }));
    }

    /// Whether a provider is selected.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready.is_some()
    }

    /// Whether a turn is running.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.turn.is_some()
    }

    /// Why prompts cannot be answered right now.
    #[must_use]
    pub fn unavailable_reason(&self) -> String {
        match (&self.unavailable, &self.pick) {
            (Some(reason), _) => format!("No model provider is available: {reason}"),
            (None, Some(_)) => "Still connecting to a model provider; draft retained".to_owned(),
            (None, None) => "No model provider is configured; draft retained".to_owned(),
        }
    }

    /// Start answering `text` on `session`'s kernel. The prompt must already be recorded.
    /// # Errors
    /// No provider, a turn already running, or agent setup failure.
    pub async fn start_turn(
        &mut self,
        session: &OfflineSession,
        text: String,
        plan: bool,
    ) -> Result<(), ChatError> {
        if self.turn.is_some() {
            return Err(ChatError::Setup("a turn is already running"));
        }
        let ready = self
            .ready
            .as_ref()
            .ok_or(ChatError::Setup("no model provider"))?;
        let mut agent = match self.agent.take() {
            Some(agent) => agent,
            None => {
                ChatAgent::new(
                    session.kernel(),
                    session.workspace(),
                    ready.model_id.clone(),
                    Arc::clone(&ready.model),
                    ready.context_tokens,
                    ChatSettings {
                        effort: ready.effort,
                        ..ChatSettings::default()
                    },
                )
                .await?
            }
        };
        agent.set_plan_mode(plan);
        let kernel = session.kernel().clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (progress_tx, progress) = mpsc::channel(64);
        let handle = tokio::spawn(async move {
            let result = agent.turn(&kernel, &text, &token, progress_tx).await;
            (agent, result)
        });
        self.turn = Some(RunningTurn {
            handle,
            cancel,
            progress,
            progress_closed: false,
            started: Instant::now(),
            tail: Vec::new(),
            changed: Vec::new(),
        });
        Ok(())
    }

    /// Interrupt the running turn; its [`HostEvent::Failed`] follows.
    pub fn cancel_turn(&self) {
        if let Some(turn) = self.turn.as_ref() {
            turn.cancel.cancel();
        }
    }

    /// Forget the carried conversation (`/clear`).
    pub fn clear(&mut self) {
        if let Some(agent) = self.agent.as_mut() {
            agent.clear();
        }
    }

    /// Drop the conversation of a session that was closed or switched away from.
    pub fn reset_conversation(&mut self) {
        self.agent = None;
    }

    /// Continue on the provider's model for `effort`; returns the model id.
    /// # Errors
    /// A running turn, no provider, or no configured model for `effort`.
    pub fn set_effort(&mut self, effort: Effort) -> Result<String, String> {
        if self.turn.is_some() {
            return Err("A reply is still running; change the model after it finishes".to_owned());
        }
        let reason = self.unavailable_reason();
        let ready = self.ready.as_mut().ok_or(reason)?;
        let model_id = provider_model(&self.config, ready.provider, effort)
            .cloned()
            .ok_or_else(|| "The provider has no model configured".to_owned())?;
        let model = if model_id == ready.model_id {
            Arc::clone(&ready.model)
        } else {
            Arc::new(ready.model.rebind(model_id.clone()))
        };
        if let Some(agent) = self.agent.as_mut() {
            agent
                .switch_model(
                    model_id.clone(),
                    Arc::clone(&model),
                    ready.context_tokens,
                    effort,
                )
                .map_err(|error| error.to_string())?;
        }
        ready.model = model;
        ready.model_id = model_id.clone();
        ready.effort = effort;
        Ok(model_id.as_str().to_owned())
    }

    /// Next selection result, progress item or turn outcome. Cancel-safe: a dropped call loses
    /// nothing.
    pub async fn next_event(&mut self) -> HostEvent {
        if let Some(event) = self.queued.pop_front() {
            return event;
        }
        if let Some(pick) = self.pick.as_mut() {
            let result = pick.await;
            self.pick = None;
            return self
                .picked(result.unwrap_or(Err(ChatError::Setup("provider selection task failed"))));
        }
        let joined = {
            let Some(turn) = self.turn.as_mut() else {
                return std::future::pending().await;
            };
            tokio::select! {
                biased;
                progress = turn.progress.recv(), if !turn.progress_closed => match progress {
                    Some(item) => {
                        turn.note(&item);
                        return HostEvent::Progress(item);
                    }
                    None => {
                        turn.progress_closed = true;
                        (&mut turn.handle).await
                    }
                },
                joined = &mut turn.handle => joined,
            }
        };
        let Some(mut turn) = self.turn.take() else {
            return std::future::pending().await;
        };
        while let Ok(item) = turn.progress.try_recv() {
            turn.note(&item);
            turn.tail.push(item);
        }
        self.queued
            .extend(turn.tail.drain(..).map(HostEvent::Progress));
        let outcome = match joined {
            Ok((agent, result)) => {
                self.agent = Some(agent);
                match result {
                    Ok(done) => HostEvent::Finished(TurnReport {
                        answer: done.answer,
                        attempts: done.attempts,
                        tool_rounds: done.tool_rounds,
                        stop: done.stop,
                        elapsed: turn.started.elapsed(),
                        changed_files: std::mem::take(&mut turn.changed),
                    }),
                    Err(ChatError::Cancelled) => HostEvent::Failed(INTERRUPTED.to_owned()),
                    Err(error) => HostEvent::Failed(error.to_string()),
                }
            }
            Err(_) => HostEvent::Failed(
                "The conversation task failed; start a new session to continue".to_owned(),
            ),
        };
        self.queued.push_back(outcome);
        match self.queued.pop_front() {
            Some(event) => event,
            None => std::future::pending().await,
        }
    }

    fn picked(&mut self, result: Result<ProviderChoice, ChatError>) -> HostEvent {
        let choice = match result {
            Ok(choice) => choice,
            Err(error) => {
                let reason = error.to_string();
                self.unavailable = Some(reason.clone());
                return HostEvent::ProviderUnavailable(reason);
            }
        };
        let skipped: Vec<String> = choice
            .skipped
            .iter()
            .map(|skip| format!("{}: {}", skip.provider.as_str(), skip.reason.as_str()))
            .collect();
        let reasons = skipped_reasons(&choice);
        let Some(selected) = choice.selected else {
            self.unavailable = Some(reasons.clone());
            return HostEvent::ProviderUnavailable(reasons);
        };
        let model_id = selected.model.model().clone();
        let effort = ChatSettings::default().effort;
        let models = Effort::ALL
            .iter()
            .filter_map(|level| {
                provider_model(&self.config, selected.provider, *level)
                    .map(|model| (*level, model.as_str().to_owned()))
            })
            .collect();
        let event = HostEvent::ProviderReady {
            provider: selected.provider.as_str(),
            model: model_id.as_str().to_owned(),
            effort,
            models,
            skipped,
        };
        self.ready = Some(ReadyModel {
            provider: selected.provider,
            effort,
            context_tokens: selected.model.max_context_tokens().get(),
            model_id,
            model: Arc::new(selected.model),
        });
        event
    }

    /// Cancel a running turn and wait a bounded time for it to settle.
    pub async fn shutdown(&mut self) {
        if let Some(mut turn) = self.turn.take() {
            turn.cancel.cancel();
            let _settled = tokio::time::timeout(Duration::from_secs(10), &mut turn.handle).await;
        }
        if let Some(pick) = self.pick.take() {
            pick.abort();
        }
    }
}

/// Records up to and including the active session's own typed input, so a frontend shows the
/// prompt before any streamed reply. Stops quietly after five seconds or 4096 records.
/// # Errors
/// The record stream ended or a payload could not be resolved.
pub async fn records_until_own_input(
    session: &mut OfflineSession,
) -> Result<Vec<ResolvedRecord>, SessionHostError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut records = Vec::new();
    while records.len() < 4096 {
        match tokio::time::timeout_at(deadline, session.next_record()).await {
            Ok(Ok(Some(item))) => {
                let own = item.record.session() == session.id
                    && item.text_kind == Some(RecordTextKind::UserInput);
                records.push(item);
                if own {
                    break;
                }
            }
            Ok(Ok(None)) => {
                return Err(SessionHostError::Unavailable("kernel event stream closed"));
            }
            Ok(Err(error)) => return Err(error),
            Err(_elapsed) => break,
        }
    }
    Ok(records)
}
