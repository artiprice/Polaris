//! Production local-model process loading over the existing platform and model-slot ports.
//!
//! Listener ownership is observed at startup and is not endpoint authentication. The adapter
//! separately validates the configured llama.cpp build, model alias, context, and chat template.

use std::{
    collections::BTreeMap,
    fmt,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use polaris_models::{
    EvictOutcome, EvictableSlot, ModelChoice, ModelEffort, ModelKind, ModelLease, ModelLoader,
    ModelRegistry, ModelsError, ProviderKind, SlotConfig, SlotState,
};
use polaris_nebius_cloud::token_factory::{
    BaseUrl, ClientConfig, LlamaCppChatClient, LlamaCppExpectedProfile, ProviderStream,
    ReasoningReplay,
};
use polaris_sdk::{
    effort::Effort,
    ids::ModelId,
    registry::{
        ChatChunkSource, ChatModel, ChatRequest, LocalEndpointOwnership, ModelError, PlatformError,
        PlatformOps, ProcessTree, PromptTokenCount, PromptTokenizer, SpawnSpec, StopOutcome,
        WorkerBootstrapPlatform,
    },
};
use thiserror::Error;
use tokio::time::{Instant, sleep, timeout_at};

const MAX_CONFIGURED_MODELS: usize = 128;
const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STOP_GRACE: Duration = Duration::from_secs(60);
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(7_200);

/// Public, secret-free process configuration for one pinned local llama.cpp model.
pub struct LocalModelProcessConfig {
    model: ModelId,
    process: SpawnSpec,
    endpoint: SocketAddr,
    base_url: BaseUrl,
    profile: LlamaCppExpectedProfile,
}

impl fmt::Debug for LocalModelProcessConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalModelProcessConfig")
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("profile", &self.profile)
            .field("process", &"[REDACTED]")
            .finish()
    }
}

impl LocalModelProcessConfig {
    /// Pin an exact local process, numeric loopback endpoint, and llama.cpp profile.
    ///
    /// # Errors
    /// Returns InvalidConfiguration for a wildcard, non-loopback, mismatched URL, or alias.
    pub fn new(
        model: ModelId,
        process: SpawnSpec,
        endpoint: SocketAddr,
        api_base_url: String,
        profile: LlamaCppExpectedProfile,
    ) -> Result<Self, LocalModelLifecycleError> {
        if !endpoint.ip().is_loopback() || model.as_str() != profile.model_alias() {
            return Err(LocalModelLifecycleError::InvalidConfiguration);
        }
        let base_url = BaseUrl::parse(&api_base_url)
            .map_err(|_| LocalModelLifecycleError::InvalidConfiguration)?;
        if base_url
            .literal_loopback_socket_addr()
            .map_err(|_| LocalModelLifecycleError::InvalidConfiguration)?
            != endpoint
        {
            return Err(LocalModelLifecycleError::InvalidConfiguration);
        }
        if process.args.len() > 256 || process.env.len() > 256 {
            return Err(LocalModelLifecycleError::InvalidConfiguration);
        }
        Ok(Self {
            model,
            process,
            endpoint,
            base_url,
            profile,
        })
    }
}

/// Bounds for process startup, endpoint probes, model requests, slot admission, and shutdown.
#[derive(Debug, Clone, Copy)]
pub struct LocalModelLifecycleConfig {
    /// Maximum wait for process ownership and exact profile readiness.
    pub startup_timeout: Duration,
    /// Maximum duration of each listener and profile readiness probe.
    pub probe_timeout: Duration,
    /// Delay between failed readiness observations.
    pub retry_interval: Duration,
    /// Maximum duration for a single request or stream.
    pub request_timeout: Duration,
    /// Graceful stop interval before the platform force-stops the retained tree.
    pub stop_grace: Duration,
    /// Existing lease, load, and unload policy for this model slot.
    pub slot: SlotConfig,
}

impl Default for LocalModelLifecycleConfig {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(120),
            probe_timeout: Duration::from_secs(5),
            retry_interval: Duration::from_millis(250),
            request_timeout: Duration::from_secs(300),
            stop_grace: Duration::from_secs(5),
            slot: SlotConfig::default(),
        }
    }
}

/// Errors constructing the production local-model service.
#[derive(Debug, Error)]
pub enum LocalModelLifecycleError {
    /// Configuration is outside the supported bounds or does not match a pinned model.
    #[error("invalid local model lifecycle configuration")]
    InvalidConfiguration,
    /// The existing catalog or override registry rejected the declared model set.
    #[error(transparent)]
    Registry(#[from] ModelsError),
}

/// Owned local model process slots selected through the configured catalog and effort policy.
pub struct LocalModelRuntime<P: PlatformOps> {
    registry: ModelRegistry,
    slots: BTreeMap<ModelId, EvictableSlot<OwnedLocalModel<P>, LocalModelLoader<P>>>,
}

impl<P: PlatformOps> fmt::Debug for LocalModelRuntime<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalModelRuntime")
            .field("configured_models", &self.slots.len())
            .finish_non_exhaustive()
    }
}

impl<P> LocalModelRuntime<P>
where
    P: WorkerBootstrapPlatform + LocalEndpointOwnership + 'static,
{
    /// Compose actual process loaders into existing catalog and slot lifecycle contracts.
    ///
    /// # Errors
    /// Returns Registry or InvalidConfiguration when model bindings are missing, duplicated,
    /// unsupported, or outside strict lifecycle bounds.
    pub fn new(
        platform: Arc<P>,
        mut registry: ModelRegistry,
        overrides: &[ModelEffort],
        processes: Vec<LocalModelProcessConfig>,
        config: LocalModelLifecycleConfig,
    ) -> Result<Self, LocalModelLifecycleError> {
        validate_config(config)?;
        if processes.is_empty() || processes.len() > MAX_CONFIGURED_MODELS {
            return Err(LocalModelLifecycleError::InvalidConfiguration);
        }
        let _report = registry.apply_subagent_models(overrides)?;
        let mut slots = BTreeMap::new();
        for process in processes {
            let Some(spec) = registry.get(&process.model) else {
                return Err(LocalModelLifecycleError::InvalidConfiguration);
            };
            if spec.kind != ModelKind::Chat || spec.provider != ProviderKind::OpenAiCompatible {
                return Err(LocalModelLifecycleError::InvalidConfiguration);
            }
            let id = process.model.clone();
            let loader = LocalModelLoader {
                platform: Arc::clone(&platform),
                process,
                config,
            };
            let slot = EvictableSlot::new(loader, config.slot)?;
            if slots.insert(id, slot).is_some() {
                return Err(LocalModelLifecycleError::InvalidConfiguration);
            }
        }
        Ok(Self { registry, slots })
    }

    /// Resolve a model without loading it, honoring registry overrides and explicit constraints.
    ///
    /// # Errors
    /// Returns the existing registry policy errors or when no process is configured for the choice.
    pub fn choice_for_effort(
        &self,
        effort: Effort,
        allowed_efforts: &[Effort],
        allowed_models: &[ModelId],
    ) -> Result<ModelChoice, ModelsError> {
        let choice = self
            .registry
            .constrained_model(effort, allowed_efforts, allowed_models)?;
        if !self.slots.contains_key(&choice.model) {
            return Err(ModelsError::UnknownModel);
        }
        Ok(choice)
    }

    /// Acquire the selected local model with its exact tokenizer and chat adapter under one lease.
    ///
    /// # Errors
    /// Returns registry policy errors, actual startup failures, lease limits, or slot failures.
    pub async fn acquire_for_effort(
        &self,
        effort: Effort,
        allowed_efforts: &[Effort],
        allowed_models: &[ModelId],
    ) -> Result<SelectedLocalModelLease<P>, ModelsError> {
        let choice = self.choice_for_effort(effort, allowed_efforts, allowed_models)?;
        let slot = self
            .slots
            .get(&choice.model)
            .ok_or(ModelsError::UnknownModel)?;
        let lease = slot.acquire().await?;
        Ok(SelectedLocalModelLease {
            owner: Arc::new(LeasedLocalModel { lease }),
            choice,
            effort,
        })
    }

    /// Observe the existing model slot state without exposing its backend handle.
    ///
    /// # Errors
    /// Unknown model id or slot accounting integrity failure.
    pub fn slot_state(&self, model: &ModelId) -> Result<SlotState, ModelsError> {
        self.slots
            .get(model)
            .ok_or(ModelsError::UnknownModel)?
            .state()
    }

    /// Permanently close every configured model slot by one shared absolute deadline.
    ///
    /// # Errors
    /// Returns private model-slot lifecycle failures.
    pub async fn close(&self, deadline: Duration) -> Result<Vec<EvictOutcome>, ModelsError> {
        let end = Instant::now()
            .checked_add(deadline)
            .ok_or(ModelsError::Config("shutdown deadline range"))?;
        let mut tasks = tokio::task::JoinSet::new();
        for slot in self.slots.values() {
            let remaining = end.saturating_duration_since(Instant::now());
            let owned_slot = slot.clone();
            tasks.spawn(async move { owned_slot.close(remaining).await });
        }
        let mut outcomes = Vec::with_capacity(self.slots.len());
        while let Some(result) = tasks.join_next().await {
            let outcome =
                result.map_err(|_| ModelsError::Load(polaris_models::LoadError::TaskFailed))??;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}

fn validate_config(config: LocalModelLifecycleConfig) -> Result<(), LocalModelLifecycleError> {
    if config.startup_timeout.is_zero()
        || config.startup_timeout > MAX_STARTUP_TIMEOUT
        || config.probe_timeout.is_zero()
        || config.probe_timeout > MAX_PROBE_TIMEOUT
        || config.retry_interval.is_zero()
        || config.request_timeout.is_zero()
        || config.request_timeout > MAX_REQUEST_TIMEOUT
        || config.stop_grace > MAX_STOP_GRACE
        || config.stop_grace > config.slot.unload_timeout
        || config.startup_timeout > config.slot.load_timeout
        || config.slot.max_leases == 0
        || config.slot.load_timeout.is_zero()
        || config.slot.unload_timeout.is_zero()
    {
        return Err(LocalModelLifecycleError::InvalidConfiguration);
    }
    Ok(())
}

struct LocalModelLoader<P> {
    platform: Arc<P>,
    process: LocalModelProcessConfig,
    config: LocalModelLifecycleConfig,
}

impl<P> fmt::Debug for LocalModelLoader<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalModelLoader")
            .field("model", &self.process.model)
            .field("endpoint", &self.process.endpoint)
            .finish_non_exhaustive()
    }
}

impl<P> ModelLoader<OwnedLocalModel<P>> for LocalModelLoader<P>
where
    P: WorkerBootstrapPlatform + LocalEndpointOwnership + 'static,
{
    async fn load(&self) -> Result<OwnedLocalModel<P>, polaris_models::LoadError> {
        let tree = self
            .platform
            .spawn_tree(&self.process.process)
            .map_err(platform_load_error)?;
        let guard = OwnedProcessTree::new(Arc::clone(&self.platform), tree);
        let started = Instant::now();
        let deadline = started
            .checked_add(self.config.startup_timeout)
            .ok_or(polaris_models::LoadError::InvalidConfiguration)?;
        let client_config = client_config(self.config);
        loop {
            if Instant::now() >= deadline {
                return Err(polaris_models::LoadError::Timeout);
            }
            let probe_deadline = Instant::now()
                .checked_add(self.config.probe_timeout)
                .ok_or(polaris_models::LoadError::InvalidConfiguration)?
                .min(deadline);
            let owned = timeout_at(
                probe_deadline,
                self.platform
                    .owns_loopback_tcp_listener(&guard.tree, self.process.endpoint),
            )
            .await;
            let Ok(Ok(true)) = owned else {
                if !sleep_until_retry(deadline, self.config.retry_interval).await {
                    return Err(polaris_models::LoadError::Timeout);
                }
                continue;
            };
            let profile_deadline = Instant::now()
                .checked_add(self.config.probe_timeout)
                .ok_or(polaris_models::LoadError::InvalidConfiguration)?
                .min(deadline);
            let connected = timeout_at(
                profile_deadline,
                LlamaCppChatClient::connect(
                    self.process.base_url.clone(),
                    self.process.profile.clone(),
                    client_config,
                ),
            )
            .await;
            if let Ok(Ok(client)) = connected {
                let ownership_deadline = Instant::now()
                    .checked_add(self.config.probe_timeout)
                    .ok_or(polaris_models::LoadError::InvalidConfiguration)?
                    .min(deadline);
                let still_owned = timeout_at(
                    ownership_deadline,
                    self.platform
                        .owns_loopback_tcp_listener(&guard.tree, self.process.endpoint),
                )
                .await;
                if matches!(still_owned, Ok(Ok(true))) {
                    return Ok(OwnedLocalModel {
                        model: self.process.model.clone(),
                        tree: guard,
                        client,
                    });
                }
            }
            if !sleep_until_retry(deadline, self.config.retry_interval).await {
                return Err(polaris_models::LoadError::Timeout);
            }
        }
    }

    async fn unload(&self, model: &OwnedLocalModel<P>) -> Result<(), polaris_models::LoadError> {
        model.tree.stop(self.config.stop_grace).await
    }
}

async fn sleep_until_retry(deadline: Instant, delay: Duration) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    let retry_at = now
        .checked_add(delay)
        .map_or(deadline, |value| value.min(deadline));
    sleep(retry_at.saturating_duration_since(now)).await;
    Instant::now() < deadline
}

fn client_config(config: LocalModelLifecycleConfig) -> ClientConfig {
    ClientConfig {
        request_timeout: config.request_timeout,
        connect_timeout: config.probe_timeout,
        stream_idle_timeout: config.request_timeout,
        max_sse_frame_bytes: 256 * 1024,
        max_events_per_chunk: 64,
        max_chat_stream_bytes: 64 * 1024 * 1024,
        max_text_bytes: 32 * 1024 * 1024,
        max_tool_argument_bytes: 8 * 1024 * 1024,
        max_tool_calls: 128,
        max_json_response_bytes: 16 * 1024 * 1024,
        max_request_bytes: 16 * 1024 * 1024,
        max_429_retries: 0,
        max_5xx_retries: 0,
        retry_base_delay: Duration::from_millis(50),
        max_retry_delay: Duration::from_millis(50),
        reasoning_replay: ReasoningReplay::Disabled,
    }
}

fn platform_load_error(error: PlatformError) -> polaris_models::LoadError {
    let _error = error;
    polaris_models::LoadError::Unavailable
}

struct OwnedProcessTree<P: PlatformOps> {
    platform: Arc<P>,
    tree: ProcessTree,
    active: AtomicBool,
}

impl<P: PlatformOps> fmt::Debug for OwnedProcessTree<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedProcessTree")
            .field("root_pid", &self.tree.root_pid)
            .field("active", &self.active.load(Ordering::Acquire))
            .finish()
    }
}

impl<P: PlatformOps> OwnedProcessTree<P> {
    fn new(platform: Arc<P>, tree: ProcessTree) -> Self {
        Self {
            platform,
            tree,
            active: AtomicBool::new(true),
        }
    }
}

impl<P: PlatformOps> OwnedProcessTree<P> {
    async fn stop(&self, grace: Duration) -> Result<(), polaris_models::LoadError> {
        if !self.active.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self.platform.graceful_stop(&self.tree, grace).await;
        match result {
            Ok(StopOutcome::Exited { .. } | StopOutcome::Killed) => {
                self.active.store(false, Ordering::Release);
                Ok(())
            }
            Err(error) => Err(platform_load_error(error)),
        }
    }
}

impl<P: PlatformOps> Drop for OwnedProcessTree<P> {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::AcqRel)
            && let Err(error) = self.platform.kill_tree(&self.tree)
        {
            tracing::error!(
                root_pid = self.tree.root_pid,
                error = %error,
                "local model force-stop request was not confirmed"
            );
        }
    }
}

struct OwnedLocalModel<P: PlatformOps> {
    model: ModelId,
    tree: OwnedProcessTree<P>,
    client: LlamaCppChatClient,
}

impl<P: PlatformOps> fmt::Debug for OwnedLocalModel<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedLocalModel")
            .field("model", &self.model)
            .field("tree", &self.tree)
            .field("client", &self.client)
            .finish()
    }
}

struct LeasedLocalModel<P: PlatformOps> {
    lease: ModelLease<OwnedLocalModel<P>>,
}

impl<P: PlatformOps> fmt::Debug for LeasedLocalModel<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeasedLocalModel").finish_non_exhaustive()
    }
}

impl<P: PlatformOps> LeasedLocalModel<P> {
    fn model(&self) -> Result<&OwnedLocalModel<P>, ModelError> {
        self.lease
            .model()
            .map_err(|error| ModelError::Transport(error.to_string()))
    }
}

/// Selected model client whose tokenizer, chat calls, and streams share one lifecycle lease.
pub struct SelectedLocalModelLease<P: PlatformOps> {
    owner: Arc<LeasedLocalModel<P>>,
    choice: ModelChoice,
    effort: Effort,
}

impl<P: PlatformOps> fmt::Debug for SelectedLocalModelLease<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectedLocalModelLease")
            .field("choice", &self.choice)
            .finish_non_exhaustive()
    }
}

impl<P: PlatformOps> SelectedLocalModelLease<P> {
    /// The exact registry choice that selected this local model.
    #[must_use]
    pub const fn choice(&self) -> &ModelChoice {
        &self.choice
    }

    fn validate_request(&self, request: &ChatRequest) -> Result<(), ModelError> {
        if request.model != self.choice.model || request.effort != self.effort {
            return Err(ModelError::BadRequest(
                "request does not match selected model choice".to_owned(),
            ));
        }
        Ok(())
    }
}

impl<P: PlatformOps + Send + Sync + 'static> PromptTokenizer for SelectedLocalModelLease<P> {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        self.validate_request(request)?;
        self.owner.model()?.client.count_prompt(request).await
    }
}

impl<P: PlatformOps + Send + Sync + 'static> ChatModel for SelectedLocalModelLease<P> {
    type Stream = LeasedLocalModelStream<P>;

    async fn stream(&self, request: ChatRequest) -> Result<Self::Stream, ModelError> {
        self.validate_request(&request)?;
        let stream = self.owner.model()?.client.stream(request).await?;
        Ok(LeasedLocalModelStream {
            stream: Some(stream),
            owner: Some(Arc::clone(&self.owner)),
        })
    }
}

/// Streaming adapter that retains the loaded-process lease until completion or drop.
pub struct LeasedLocalModelStream<P: PlatformOps> {
    stream: Option<ProviderStream>,
    owner: Option<Arc<LeasedLocalModel<P>>>,
}

impl<P: PlatformOps> fmt::Debug for LeasedLocalModelStream<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeasedLocalModelStream")
            .field("owner", &self.owner.as_ref())
            .finish_non_exhaustive()
    }
}

impl<P: PlatformOps + Send + Sync + 'static> ChatChunkSource for LeasedLocalModelStream<P> {
    async fn next_chunk(&mut self) -> Option<Result<polaris_sdk::registry::ChatChunk, ModelError>> {
        let result = self.stream.as_mut()?.next_chunk().await;
        if result.is_none() || result.as_ref().is_some_and(Result::is_err) {
            self.stream = None;
            self.owner = None;
        }
        result
    }
}
