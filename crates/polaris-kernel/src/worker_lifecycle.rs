//! Authenticated S1 worker bootstrap, connection publication, and health probing.
//!
//! The client connector checks the server PID on every named-pipe stream on Windows and the peer
//! PID on every Unix stream before worker credentials are sent. The worker bearer token
//! authenticates RPC clients to the server. This module does not create or validate Windows named-
//! pipe ACLs and makes no same-user isolation claim. Model loading is lazy and `is_ready` reports
//! service admission, not model availability.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use polaris_kernel_supervisor::{HealthProbe, TaskError, WorkerLifecycle};
use polaris_proto::worker_auth::WorkerCredential;
use polaris_sdk::registry::{
    DecisionEngine, DecisionError, PlatformError, ProcessTree, WorkerBootstrapPlatform,
};
use polaris_sdk::system_one::{Decision, DecisionInput, Question};
use polaris_system_one::client::{ClientConfig, S1Client};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// A ready S1 client paired with the exact process generation that owns it.
#[derive(Clone)]
pub struct ReadyS1Worker {
    tree: ProcessTree,
    client: Arc<S1Client>,
}

impl fmt::Debug for ReadyS1Worker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadyS1Worker")
            .field("tree", &self.tree)
            .field("client", &"<authenticated S1 client>")
            .finish()
    }
}

impl ReadyS1Worker {
    /// The exact owned process tree associated with this connection.
    #[must_use]
    pub const fn tree(&self) -> ProcessTree {
        self.tree
    }

    /// Clones the authenticated client handle without exposing its credential.
    #[must_use]
    pub fn client(&self) -> Arc<S1Client> {
        Arc::clone(&self.client)
    }
}

impl DecisionEngine for ReadyS1Worker {
    async fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.client.decide(schema, input).await
    }
}

impl polaris_sdk::system_one::PiiSpanDetector for ReadyS1Worker {
    async fn detect_pii_spans(
        &self,
        raw_text: &str,
    ) -> Result<Vec<polaris_sdk::system_one::PiiEntitySpan>, DecisionError> {
        polaris_sdk::system_one::PiiSpanDetector::detect_pii_spans(self.client.as_ref(), raw_text)
            .await
    }
}

/// Decision adapter that resolves the authenticated client for each current worker generation.
///
/// A response is rejected if its exact process tree retired while the request was in flight.
#[derive(Clone)]
pub struct CurrentS1Engine<P: WorkerBootstrapPlatform> {
    lifecycle: Arc<S1WorkerLifecycle<P>>,
}

impl<P: WorkerBootstrapPlatform> fmt::Debug for CurrentS1Engine<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentS1Engine")
            .field("generation", &"resolved per request")
            .finish()
    }
}

impl<P: WorkerBootstrapPlatform> CurrentS1Engine<P> {
    /// Creates a generation-resolving adapter over the host lifecycle.
    #[must_use]
    pub fn new(lifecycle: Arc<S1WorkerLifecycle<P>>) -> Self {
        Self { lifecycle }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> DecisionEngine for CurrentS1Engine<P> {
    async fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        let revision = self.lifecycle.inference_revision()?;
        let ready = self
            .lifecycle
            .connection()
            .map_err(|_| DecisionError::Unavailable("S1 worker state unavailable".to_owned()))?
            .ok_or_else(|| DecisionError::Unavailable("S1 worker is restarting".to_owned()))?;
        let tree = ready.tree();
        let result = ready.client.decide(schema, input).await;
        if self.lifecycle.inference_revision()? != revision {
            return Err(DecisionError::Unavailable(
                "S1 model admission changed".to_owned(),
            ));
        }
        match self.lifecycle.is_current(tree) {
            Ok(true) => result,
            Ok(false) => Err(DecisionError::Unavailable(
                "S1 worker generation changed".to_owned(),
            )),
            Err(_) => Err(DecisionError::Unavailable(
                "S1 worker state unavailable".to_owned(),
            )),
        }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> CurrentS1Engine<P> {
    /// Perform calibrated inference on the current exact owned worker generation.
    /// # Errors
    /// Missing/retired worker, pinned model mismatch, authenticated transport or inference failure.
    pub async fn decide_pinned(
        &self,
        expected: [u8; 32],
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        let revision = self.lifecycle.inference_revision()?;
        let ready = self
            .lifecycle
            .connection()
            .map_err(|_| DecisionError::Unavailable("S1 worker state unavailable".to_owned()))?
            .ok_or_else(|| DecisionError::Unavailable("S1 worker is restarting".to_owned()))?;
        let tree = ready.tree();
        let result = ready.client.decide_pinned(expected, schema, input).await;
        if self.lifecycle.inference_revision()? != revision {
            return Err(DecisionError::Unavailable(
                "S1 model admission changed".to_owned(),
            ));
        }
        match self.lifecycle.is_current(tree) {
            Ok(true) => result,
            Ok(false) | Err(_) => Err(DecisionError::Unavailable(
                "S1 worker generation changed".to_owned(),
            )),
        }
    }
}

/// PII detector that resolves the authenticated client for each current worker generation.
///
/// A response is rejected if its exact process tree retired while the request was in flight.
#[derive(Clone)]
pub struct CurrentS1PiiDetector<P: WorkerBootstrapPlatform> {
    lifecycle: Arc<S1WorkerLifecycle<P>>,
}

impl<P: WorkerBootstrapPlatform> fmt::Debug for CurrentS1PiiDetector<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentS1PiiDetector")
            .field("generation", &"resolved per request")
            .finish()
    }
}

impl<P: WorkerBootstrapPlatform> CurrentS1PiiDetector<P> {
    /// Creates a generation-resolving detector over the host lifecycle.
    #[must_use]
    pub fn new(lifecycle: Arc<S1WorkerLifecycle<P>>) -> Self {
        Self { lifecycle }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> polaris_sdk::system_one::PiiSpanDetector
    for CurrentS1PiiDetector<P>
{
    async fn detect_pii_spans(
        &self,
        raw_text: &str,
    ) -> Result<Vec<polaris_sdk::system_one::PiiEntitySpan>, DecisionError> {
        let revision = self.lifecycle.inference_revision()?;
        let ready = self
            .lifecycle
            .connection()
            .map_err(|_| DecisionError::Unavailable("S1 worker state unavailable".to_owned()))?
            .ok_or_else(|| DecisionError::Unavailable("S1 worker is restarting".to_owned()))?;
        let tree = ready.tree();
        let result = polaris_sdk::system_one::PiiSpanDetector::detect_pii_spans(
            ready.client.as_ref(),
            raw_text,
        )
        .await;
        if self.lifecycle.inference_revision()? != revision {
            return Err(DecisionError::Unavailable(
                "S1 model admission changed".to_owned(),
            ));
        }
        match self.lifecycle.is_current(tree) {
            Ok(true) => result,
            Ok(false) => Err(DecisionError::Unavailable(
                "S1 worker generation changed".to_owned(),
            )),
            Err(_) => Err(DecisionError::Unavailable(
                "S1 worker state unavailable".to_owned(),
            )),
        }
    }
}

/// Supervisor lifecycle adapter for one authenticated local S1 worker endpoint.
type ReadinessObserver = Arc<dyn Fn(bool) + Send + Sync>;
type ModelReadinessObserver = Arc<dyn Fn() + Send + Sync>;

pub struct S1WorkerLifecycle<P: WorkerBootstrapPlatform> {
    platform: Arc<P>,
    endpoint: PathBuf,
    client_config: ClientConfig,
    bootstrap_timeout: Duration,
    latest_tree: Mutex<Option<ProcessTree>>,
    ready: Mutex<Option<ReadyS1Worker>>,
    readiness_observer: Mutex<Option<ReadinessObserver>>,
    model_ready_observer: Mutex<Option<ModelReadinessObserver>>,
    requested_pause: AtomicU64,
    pause_counter: AtomicU64,
    admission_revision: AtomicU64,
    pause_owner: Arc<AsyncMutex<()>>,
    pause_transition: AsyncMutex<()>,
}

impl<P: WorkerBootstrapPlatform> fmt::Debug for S1WorkerLifecycle<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S1WorkerLifecycle")
            .field("endpoint", &self.endpoint)
            .field("client_config", &self.client_config)
            .field("bootstrap_timeout", &self.bootstrap_timeout)
            .field("generation_state", &"<private>")
            .finish_non_exhaustive()
    }
}

impl<P: WorkerBootstrapPlatform> S1WorkerLifecycle<P> {
    /// Creates an S1 lifecycle adapter for a configured local endpoint.
    ///
    /// The connector checks the server process identifier on each named-pipe stream on Windows
    /// and the peer PID on each Unix stream before tonic sends worker credentials. This does not
    /// create or validate Windows named-pipe ACLs; callers still need an appropriate local-user
    /// boundary for the endpoint.
    ///
    /// # Errors
    /// Returns a fixed sanitized error if client bounds or the bootstrap deadline are invalid.
    pub fn new(
        platform: Arc<P>,
        endpoint: PathBuf,
        client_config: ClientConfig,
        bootstrap_timeout: Duration,
    ) -> Result<Self, TaskError> {
        if ClientConfig::new(client_config.timeout, client_config.max_input_bytes).is_err()
            || bootstrap_timeout.is_zero()
            || bootstrap_timeout > Duration::from_secs(120)
        {
            return Err(TaskError::fatal("invalid S1 worker lifecycle bounds"));
        }
        Ok(Self {
            platform,
            endpoint,
            client_config,
            bootstrap_timeout,
            latest_tree: Mutex::new(None),
            ready: Mutex::new(None),
            readiness_observer: Mutex::new(None),
            model_ready_observer: Mutex::new(None),
            requested_pause: AtomicU64::new(0),
            pause_counter: AtomicU64::new(0),
            admission_revision: AtomicU64::new(0),
            pause_owner: Arc::new(AsyncMutex::new(())),
            pause_transition: AsyncMutex::new(()),
        })
    }

    /// Install the host readiness observer used to fence policy epochs.
    ///
    /// The callback is invoked only after lifecycle locks are released. Callers should capture
    /// weak references so the lifecycle cannot keep its policy owner alive through a cycle.
    /// # Errors
    /// Returns a fixed sanitized error if lifecycle state is poisoned.
    pub fn set_readiness_observer(
        &self,
        observer: Arc<dyn Fn(bool) + Send + Sync>,
    ) -> Result<(), TaskError> {
        *lock_state(&self.readiness_observer)? = Some(observer);
        Ok(())
    }

    /// Install an observer for authenticated health reports that prove an S1 model is loaded.
    /// The callback runs after RPC completion and must not retain the policy owner strongly.
    /// # Errors
    /// Returns a fixed sanitized error if lifecycle state is poisoned.
    pub fn set_model_ready_observer(
        &self,
        observer: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), TaskError> {
        *lock_state(&self.model_ready_observer)? = Some(observer);
        Ok(())
    }

    /// Returns the published ready connection, if one exists.
    ///
    /// The returned value carries the exact process tree and a credential-redacted client handle.
    pub fn connection(&self) -> Result<Option<ReadyS1Worker>, TaskError> {
        let state = lock_state(&self.ready)?;
        Ok(state.clone())
    }

    fn inference_revision(&self) -> Result<u64, DecisionError> {
        let revision = self.admission_revision.load(Ordering::Acquire);
        if self.requested_pause.load(Ordering::Acquire) != 0
            || self.admission_revision.load(Ordering::Acquire) != revision
        {
            return Err(DecisionError::Unavailable(
                "S1 models paused for host execution".to_owned(),
            ));
        }
        Ok(revision)
    }

    fn advance_admission_revision(&self) -> Result<(), TaskError> {
        let _previous = self
            .admission_revision
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| TaskError::fatal("S1 admission revision exhausted"))?;
        Ok(())
    }

    fn begin_generation(&self, tree: ProcessTree) -> Result<(), TaskError> {
        let mut latest = lock_state(&self.latest_tree)?;
        *latest = Some(tree);
        let mut ready = lock_state(&self.ready)?;
        *ready = None;
        Ok(())
    }

    fn publish_generation(&self, ready_worker: ReadyS1Worker) -> Result<bool, TaskError> {
        let latest = lock_state(&self.latest_tree)?;
        if !generation_matches(*latest, ready_worker.tree) {
            return Ok(false);
        }
        let mut ready = lock_state(&self.ready)?;
        *ready = Some(ready_worker);
        Ok(true)
    }

    fn ready_for(&self, tree: ProcessTree) -> Result<Option<ReadyS1Worker>, TaskError> {
        let state = lock_state(&self.ready)?;
        match state.as_ref() {
            Some(ready) if ready.tree == tree => Ok(Some(ready.clone())),
            Some(_) | None => Ok(None),
        }
    }

    /// Returns whether this exact process tree remains the published generation.
    pub fn is_current(&self, tree: ProcessTree) -> Result<bool, TaskError> {
        let latest = lock_state(&self.latest_tree)?;
        let ready = lock_state(&self.ready)?;
        Ok(*latest == Some(tree) && ready.as_ref().is_some_and(|worker| worker.tree == tree))
    }

    /// Return the exact lifecycle generation being initialized or served.
    pub fn active_tree(&self) -> Result<Option<ProcessTree>, TaskError> {
        Ok(*lock_state(&self.latest_tree)?)
    }

    fn report_readiness(&self, ready: bool) {
        let observer = match self.readiness_observer.lock() {
            Ok(observer) => observer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(observer) = observer {
            observer(ready);
        }
    }

    fn report_model_ready(&self) {
        let observer = match self.model_ready_observer.lock() {
            Ok(observer) => observer.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(observer) = observer {
            observer();
        }
    }

    async fn connect(
        &self,
        tree: ProcessTree,
        credential: WorkerCredential,
    ) -> Result<S1Client, TaskError> {
        #[cfg(windows)]
        {
            S1Client::connect_named_pipe_owned(
                self.endpoint.clone(),
                self.client_config,
                credential,
                tree.root_pid,
            )
            .await
            .map_err(|_| TaskError::transient("S1 worker local connection failed"))
        }
        #[cfg(unix)]
        {
            S1Client::connect_unix_owned(
                self.endpoint.clone(),
                self.client_config,
                credential,
                tree.root_pid,
            )
            .await
            .map_err(|_| TaskError::transient("S1 worker local connection failed"))
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = credential;
            Err(TaskError::fatal("S1 worker local transport is unsupported"))
        }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> HealthProbe for S1WorkerLifecycle<P> {
    async fn check(&self, tree: ProcessTree) -> Result<(), TaskError> {
        let ready = self
            .ready_for(tree)?
            .ok_or_else(|| TaskError::transient("S1 worker generation is not initialized"))?;
        match ready.client.is_ready().await {
            Ok(true) => {
                if ready
                    .client
                    .is_model_ready()
                    .await
                    .map_err(readiness_task_error)?
                    && self.is_current(tree)?
                    && self.requested_pause.load(Ordering::Acquire) == 0
                {
                    self.report_model_ready();
                }
                Ok(())
            }
            Ok(false) => Err(TaskError::transient("S1 worker admission is closed")),
            Err(error) => Err(readiness_task_error(error)),
        }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> WorkerLifecycle for S1WorkerLifecycle<P> {
    async fn initialize(&self, tree: ProcessTree) -> Result<(), TaskError> {
        self.begin_generation(tree)?;
        self.report_readiness(false);
        self.require_live_tree(tree).await?;
        let credential = WorkerCredential::generate()
            .map_err(|_| TaskError::fatal("S1 worker credential generation failed"))?;
        self.platform
            .bootstrap_tree(&tree, credential.bootstrap_secret(), self.bootstrap_timeout)
            .await
            .map_err(|_| TaskError::transient("S1 worker credential bootstrap failed"))?;
        self.require_live_tree(tree).await?;
        let client = Arc::new(self.connect(tree, credential).await?);
        match client.is_ready().await {
            Ok(true) => {}
            Ok(false) => return Err(TaskError::transient("S1 worker admission is closed")),
            Err(error) => return Err(readiness_task_error(error)),
        }
        self.require_live_tree(tree).await?;
        let transition = self.pause_transition.lock().await;
        let pause_epoch = self.requested_pause.load(Ordering::Acquire);
        if pause_epoch != 0 {
            client
                .pause_models(pause_epoch, self.bootstrap_timeout)
                .await
                .map_err(readiness_task_error)?;
            self.require_live_tree(tree).await?;
        }
        let ready_worker = ReadyS1Worker { tree, client };
        if !self.publish_generation(ready_worker)? {
            return Err(TaskError::transient("S1 worker generation was superseded"));
        }
        drop(transition);
        self.report_readiness(self.requested_pause.load(Ordering::Acquire) == 0);
        Ok(())
    }

    fn retire(&self, tree: ProcessTree) {
        let mut latest = match self.latest_tree.lock() {
            Ok(latest) => latest,
            Err(poisoned) => {
                *poisoned.into_inner() = None;
                self.latest_tree.clear_poison();
                clear_poisoned_option(&self.ready);
                self.report_readiness(false);
                return;
            }
        };
        if *latest == Some(tree) {
            *latest = None;
            let mut ready = match self.ready.lock() {
                Ok(ready) => ready,
                Err(poisoned) => {
                    *poisoned.into_inner() = None;
                    self.ready.clear_poison();
                    drop(latest);
                    self.report_readiness(false);
                    return;
                }
            };
            if ready.as_ref().is_some_and(|current| current.tree == tree) {
                *ready = None;
            }
            drop(ready);
            drop(latest);
            self.report_readiness(false);
        }
    }
}

impl<P: WorkerBootstrapPlatform + 'static> S1WorkerLifecycle<P> {
    /// Pause/evict native models before admitting a heavy host operation.
    /// The lease serializes heavy owners. Dropping it never silently reopens inference; use
    /// `resume` after the operation and cleanup finish, or explicit abandoned-pause recovery.
    /// # Errors
    /// Invalid deadline, exhausted epoch, unavailable/old worker, pause acknowledgement or timeout.
    pub async fn pause_models(
        self: &Arc<Self>,
        timeout: Duration,
    ) -> Result<S1ModelPause<P>, TaskError> {
        validate_pause_timeout(timeout)?;
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| TaskError::fatal("S1 model pause deadline overflow"))?;
        tokio::time::timeout_at(deadline, async {
            let ownership = Arc::clone(&self.pause_owner).lock_owned().await;
            if self.requested_pause.load(Ordering::Acquire) != 0 {
                return Err(TaskError::fatal(
                    "abandoned S1 model pause requires explicit recovery",
                ));
            }
            let previous = self
                .pause_counter
                .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| TaskError::fatal("S1 model pause epoch exhausted"))?;
            let epoch = previous
                .checked_add(1)
                .ok_or_else(|| TaskError::fatal("S1 model pause epoch exhausted"))?;
            self.requested_pause.store(epoch, Ordering::Release);
            self.advance_admission_revision()?;
            self.report_readiness(false);
            self.acknowledge_pause(epoch, deadline).await?;
            Ok(S1ModelPause {
                lifecycle: Arc::clone(self),
                epoch,
                timeout,
                _ownership: ownership,
            })
        })
        .await
        .map_err(|_| TaskError::transient("S1 model pause timed out"))?
    }

    /// Explicitly recover an abandoned pause only when no heavy-operation lease is alive.
    /// It first obtains a fresh acknowledged pause, then releases that new epoch. This is a trusted
    /// host lifecycle operation, never an inference or model-facing permission.
    /// # Errors
    /// Live heavy owner, timeout, unavailable worker, stale epoch or unconfirmed native admission.
    pub async fn recover_model_admission(&self, timeout: Duration) -> Result<(), TaskError> {
        validate_pause_timeout(timeout)?;
        let _ownership = Arc::clone(&self.pause_owner)
            .try_lock_owned()
            .map_err(|_| TaskError::transient("S1 heavy operation still owns model pause"))?;
        if self.requested_pause.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        let previous = self
            .pause_counter
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| TaskError::fatal("S1 model pause epoch exhausted"))?;
        let epoch = previous
            .checked_add(1)
            .ok_or_else(|| TaskError::fatal("S1 model pause epoch exhausted"))?;
        self.requested_pause.store(epoch, Ordering::Release);
        self.advance_admission_revision()?;
        self.report_readiness(false);
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| TaskError::fatal("S1 model pause deadline overflow"))?;
        tokio::time::timeout_at(deadline, async {
            self.acknowledge_pause(epoch, deadline).await?;
            self.release_pause(epoch, deadline).await
        })
        .await
        .map_err(|_| TaskError::transient("S1 model admission recovery timed out"))?
    }

    async fn acknowledge_pause(
        &self,
        epoch: u64,
        deadline: tokio::time::Instant,
    ) -> Result<(), TaskError> {
        loop {
            let transition = self.pause_transition.lock().await;
            if self.requested_pause.load(Ordering::Acquire) != epoch {
                return Err(TaskError::fatal("S1 model pause owner changed"));
            }
            if let Some(ready) = self.connection()? {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(TaskError::transient("S1 model pause timed out"));
                }
                let result = ready
                    .client
                    .pause_models(epoch, remaining.min(Duration::from_secs(120)))
                    .await;
                if self.is_current(ready.tree())? {
                    match result {
                        Ok(_) => return Ok(()),
                        Err(DecisionError::Model(_) | DecisionError::InvalidSchema(_)) => {
                            return Err(TaskError::fatal("S1 model pause protocol rejected"));
                        }
                        Err(DecisionError::Timeout | DecisionError::Unavailable(_)) => {}
                    }
                }
            }
            drop(transition);
            if tokio::time::Instant::now() >= deadline {
                return Err(TaskError::transient("S1 model pause timed out"));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn release_pause(
        &self,
        epoch: u64,
        deadline: tokio::time::Instant,
    ) -> Result<(), TaskError> {
        loop {
            let transition = self.pause_transition.lock().await;
            if self.requested_pause.load(Ordering::Acquire) != epoch {
                return Err(TaskError::fatal("S1 model pause release owner changed"));
            }
            let ready = self.connection()?;
            let service_ready = ready.is_some();
            if let Some(ready) = ready {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(TaskError::transient("S1 model resume timed out"));
                }
                let result = ready
                    .client
                    .resume_models(epoch, remaining.min(Duration::from_secs(120)))
                    .await;
                let current = self.is_current(ready.tree())?;
                match result {
                    Ok(()) if current => {}
                    Err(DecisionError::Model(_) | DecisionError::InvalidSchema(_)) if current => {
                        return Err(TaskError::fatal("S1 model resume protocol rejected"));
                    }
                    Ok(()) | Err(_) => {
                        drop(transition);
                        if tokio::time::Instant::now() >= deadline {
                            return Err(TaskError::transient("S1 model resume timed out"));
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        continue;
                    }
                }
            }
            self.advance_admission_revision()?;
            self.requested_pause.store(0, Ordering::Release);
            drop(transition);
            self.report_readiness(service_ready);
            return Ok(());
        }
    }

    async fn require_live_tree(&self, tree: ProcessTree) -> Result<(), TaskError> {
        match self.platform.try_wait_tree(&tree).await {
            Ok(None) => Ok(()),
            Ok(Some(_)) | Err(PlatformError::NotFound(_)) => Err(TaskError::transient(
                "S1 worker process exited during initialization",
            )),
            Err(_) => Err(TaskError::transient(
                "S1 worker process identity is unavailable",
            )),
        }
    }
}

/// Owned heavy-operation pause. Native models stay evicted until explicit acknowledged release.
/// Dropping this lease releases only the host ownership lock and leaves inference fail-closed.
pub struct S1ModelPause<P: WorkerBootstrapPlatform> {
    lifecycle: Arc<S1WorkerLifecycle<P>>,
    epoch: u64,
    timeout: Duration,
    _ownership: OwnedMutexGuard<()>,
}
impl<P: WorkerBootstrapPlatform> fmt::Debug for S1ModelPause<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("S1ModelPause(<owned native admission pause>)")
    }
}
impl<P: WorkerBootstrapPlatform + 'static> S1ModelPause<P> {
    /// Resume only this epoch after heavy work and all of its cleanup have settled.
    /// # Errors
    /// A stale owner, worker replacement, protocol/transport failure or release deadline.
    pub async fn resume(self) -> Result<(), TaskError> {
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| TaskError::fatal("S1 model resume deadline overflow"))?;
        tokio::time::timeout_at(deadline, self.lifecycle.release_pause(self.epoch, deadline))
            .await
            .map_err(|_| TaskError::transient("S1 model resume timed out"))?
    }
}

fn validate_pause_timeout(timeout: Duration) -> Result<(), TaskError> {
    if timeout.is_zero() || timeout > Duration::from_secs(120) {
        return Err(TaskError::fatal("invalid S1 model pause timeout"));
    }
    Ok(())
}

fn readiness_task_error(error: DecisionError) -> TaskError {
    match error {
        DecisionError::Unavailable(_) | DecisionError::Timeout => {
            TaskError::transient("S1 worker health probe failed")
        }
        DecisionError::InvalidSchema(_) | DecisionError::Model(_) => {
            TaskError::fatal("S1 worker health protocol validation failed")
        }
    }
}

fn lock_state<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, TaskError> {
    match mutex.lock() {
        Ok(guard) => Ok(guard),
        Err(_) => Err(TaskError::fatal("S1 worker lifecycle state is unavailable")),
    }
}

fn generation_matches(current: Option<ProcessTree>, expected: ProcessTree) -> bool {
    current == Some(expected)
}

fn clear_poisoned_option<T>(mutex: &Mutex<Option<T>>) {
    match mutex.lock() {
        Ok(mut value) => *value = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
    mutex.clear_poison();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{CurrentS1PiiDetector, S1WorkerLifecycle};
    use polaris_kernel_supervisor::{TaskError, WorkerLifecycle};
    use polaris_sdk::registry::{
        MemoryPressure, PlatformError, PlatformOps, ProcessExit, ProcessTree, SpawnSpec,
        StopOutcome as PlatformStopOutcome, WorkerBootstrapPlatform,
    };
    use polaris_system_one::client::ClientConfig;

    struct TestPlatform {
        bootstrap_calls: AtomicUsize,
        fail_bootstrap: bool,
    }

    impl PlatformOps for TestPlatform {
        fn spawn_tree(&self, _spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
            Err(PlatformError::Unsupported("test spawn is unused"))
        }

        async fn graceful_stop(
            &self,
            _tree: &ProcessTree,
            _grace: Duration,
        ) -> Result<PlatformStopOutcome, PlatformError> {
            Ok(PlatformStopOutcome::Exited { code: Some(0) })
        }

        fn kill_tree(&self, _tree: &ProcessTree) -> Result<(), PlatformError> {
            Ok(())
        }

        async fn try_wait_tree(
            &self,
            _tree: &ProcessTree,
        ) -> Result<Option<ProcessExit>, PlatformError> {
            Ok(None)
        }

        fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
            Ok(MemoryPressure::Normal)
        }
    }

    impl WorkerBootstrapPlatform for TestPlatform {
        fn bootstrap_tree(
            &self,
            _tree: &ProcessTree,
            _secret: secrecy::SecretString,
            _timeout: Duration,
        ) -> impl std::future::Future<Output = Result<(), PlatformError>> + Send {
            self.bootstrap_calls.fetch_add(1, Ordering::Relaxed);
            let fail_bootstrap = self.fail_bootstrap;
            async move {
                if fail_bootstrap {
                    Err(PlatformError::Policy(
                        "private bootstrap detail must not escape".to_owned(),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }

    fn lifecycle(
        fail_bootstrap: bool,
    ) -> Result<(S1WorkerLifecycle<TestPlatform>, Arc<TestPlatform>), Box<dyn std::error::Error>>
    {
        let platform = Arc::new(TestPlatform {
            bootstrap_calls: AtomicUsize::new(0),
            fail_bootstrap,
        });
        let client_config = ClientConfig::new(Duration::from_secs(1), 64)?;
        let lifecycle = S1WorkerLifecycle::new(
            Arc::clone(&platform),
            PathBuf::from("invalid-worker-endpoint"),
            client_config,
            Duration::from_secs(1),
        )?;
        Ok((lifecycle, platform))
    }

    fn tree(root_pid: u32, handle_id: u64) -> ProcessTree {
        ProcessTree {
            root_pid,
            handle_id,
        }
    }

    #[tokio::test]
    async fn initialization_bootstraps_before_connect_and_never_publishes_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let (lifecycle, platform) = lifecycle(false)?;
        let error = lifecycle
            .initialize(tree(17, 71))
            .await
            .err()
            .ok_or("invalid endpoint unexpectedly initialized")?;
        assert_eq!(
            error.to_string(),
            "supervised task failed: S1 worker local connection failed",
            "connection failures must be sanitized"
        );
        assert_eq!(platform.bootstrap_calls.load(Ordering::Relaxed), 1);
        assert!(lifecycle.connection()?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn bootstrap_errors_are_sanitized_and_do_not_publish_connections()
    -> Result<(), Box<dyn std::error::Error>> {
        let (lifecycle, platform) = lifecycle(true)?;
        let error = lifecycle
            .initialize(tree(18, 72))
            .await
            .err()
            .ok_or("failed bootstrap unexpectedly initialized")?;
        assert_eq!(
            error.to_string(),
            "supervised task failed: S1 worker credential bootstrap failed",
            "platform details must not escape the adapter"
        );
        assert!(!error.to_string().contains("private bootstrap detail"));
        assert_eq!(platform.bootstrap_calls.load(Ordering::Relaxed), 1);
        assert!(lifecycle.connection()?.is_none());
        Ok(())
    }

    #[test]
    fn retiring_an_old_generation_does_not_clear_the_new_generation() -> Result<(), TaskError> {
        let (lifecycle, _) =
            lifecycle(false).map_err(|_| TaskError::fatal("test lifecycle construction failed"))?;
        let old_tree = tree(19, 73);
        let new_tree = tree(20, 74);
        lifecycle.begin_generation(old_tree)?;
        lifecycle.begin_generation(new_tree)?;

        lifecycle.retire(old_tree);

        let latest = super::lock_state(&lifecycle.latest_tree)?;
        assert_eq!(
            *latest,
            Some(new_tree),
            "old retirement must preserve the new generation"
        );
        assert!(
            !super::generation_matches(*latest, old_tree),
            "old generation must be fenced from publication"
        );
        assert!(
            super::generation_matches(*latest, new_tree),
            "new generation must remain publishable"
        );
        Ok(())
    }

    #[test]
    fn readiness_observer_receives_generation_transitions() -> Result<(), Box<dyn std::error::Error>>
    {
        let (lifecycle, _) = lifecycle(false)?;
        let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_by_callback = Arc::clone(&observed);
        lifecycle.set_readiness_observer(Arc::new(move |ready| {
            if let Ok(mut values) = observed_by_callback.lock() {
                values.push(ready);
            }
        }))?;
        lifecycle.report_readiness(false);
        lifecycle.report_readiness(true);
        lifecycle.report_readiness(false);
        let values = observed
            .lock()
            .map_err(|_| "readiness test lock poisoned")?;
        assert_eq!(
            values.as_slice(),
            [false, true, false],
            "every host transition must fence readiness"
        );
        Ok(())
    }

    #[tokio::test]
    async fn current_detector_refuses_to_reuse_a_missing_worker_client()
    -> Result<(), Box<dyn std::error::Error>> {
        let (lifecycle, _) = lifecycle(false)?;
        let detector = CurrentS1PiiDetector::new(Arc::new(lifecycle));
        let error =
            polaris_sdk::system_one::PiiSpanDetector::detect_pii_spans(&detector, "masked input")
                .await
                .err()
                .ok_or("detector unexpectedly succeeded without an active worker")?;
        assert!(
            matches!(error, polaris_sdk::registry::DecisionError::Unavailable(_)),
            "missing generation must return unavailable"
        );
        Ok(())
    }
}
