//! Bounded local S1 worker RPC service. Native inference is lazy-loaded and isolated here.

#![forbid(unsafe_code)]

use std::future::Future;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use polaris_proto::worker_auth::WorkerAuthenticator;
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, negotiate, pb};
use polaris_sdk::system_one::{DecisionInput, Question};
use polaris_system_one_engine::{EngineConfig, GlinerEngine, GlinerPiiEngine, PiiEngineConfig};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc, oneshot, watch};
use tonic::{Request, Response, Status};

const MAX_BATCH: usize = 8;
const MAX_QUESTIONS: usize = 64;
const MAX_INPUT_BYTES: usize = 262_144;
const MAX_PII_SPANS: usize = 4_096;
const TRANSPORT_SHUTDOWN_GRACE: Duration = Duration::from_secs(120);
type LoadedModelIdentity = ([u8; 32], u64);
type SharedModelIdentity = Arc<std::sync::RwLock<Option<LoadedModelIdentity>>>;
type SharedPiiModelIdentity = Arc<std::sync::RwLock<Option<LoadedModelIdentity>>>;
type InferenceThread = std::thread::JoinHandle<std::io::Result<()>>;

mod idle;
mod ingest;
mod model_pause;
pub use idle::{IdlePolicyError, S1IdlePolicy};
pub use ingest::IngestWorker;

#[derive(Clone)]
pub struct S1Worker {
    commands: mpsc::Sender<InferenceCommand>,
    thread: Arc<AsyncMutex<Option<InferenceThread>>>,
    permits: Arc<Semaphore>,
    closing: Arc<AtomicBool>,
    drain_lock: Arc<AsyncMutex<()>>,
    shutdown: watch::Sender<bool>,
    model_identity: SharedModelIdentity,
    pii_model_identity: SharedPiiModelIdentity,
    worker_incarnation: [u8; 16],
    pii_configured: bool,
    model_paused: Arc<AtomicBool>,
}

impl std::fmt::Debug for S1Worker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S1Worker")
            .field(
                "ready",
                &(!self.closing.load(Ordering::Acquire) && !self.commands.is_closed()),
            )
            .finish_non_exhaustive()
    }
}

enum InferenceCommand {
    Decide {
        work: Vec<(Vec<Question>, DecisionInput)>,
        _permit: tokio::sync::OwnedSemaphorePermit,
        response: oneshot::Sender<Result<Vec<pb::Decision>, String>>,
    },
    DetectPii {
        raw_text: String,
        _permit: tokio::sync::OwnedSemaphorePermit,
        response: oneshot::Sender<Result<Vec<pb::PiiEntitySpan>, String>>,
    },
    Evict {
        pause_epoch: u64,
        resume: bool,
        response: oneshot::Sender<Result<pb::EvictModelsResponse, &'static str>>,
    },
    Shutdown {
        response: oneshot::Sender<()>,
    },
}

impl S1Worker {
    /// Starts the bounded S1 service and its thread-affine native inference actor.
    ///
    /// The model is loaded lazily on the actor thread when the first valid decision arrives.
    pub fn new(
        config: EngineConfig,
        shutdown: watch::Sender<bool>,
    ) -> Result<Self, std::io::Error> {
        Self::with_idle_policy(config, S1IdlePolicy::default(), shutdown)
    }

    /// Starts the native owner with a validated idle model eviction interval.
    ///
    /// This drops only an unused native session. It does not pause admission for a heavy build,
    /// impose an RSS limit, or preempt native inference. Later valid work lazily reloads the model.
    ///
    /// # Errors
    /// Returns the OS error if the inference owner thread cannot be started. Owner-runtime
    /// initialization failure closes admission and is reported by requests and explicit drain.
    pub fn with_idle_policy(
        config: EngineConfig,
        idle_policy: S1IdlePolicy,
        shutdown: watch::Sender<bool>,
    ) -> Result<Self, std::io::Error> {
        Self::with_models(config, None, idle_policy, shutdown)
    }

    /// Starts a worker with a separately configured lazy PII companion model.
    ///
    /// Decide and PII native models share one owner and are never resident at the same time. The
    /// companion model has its own idle eviction deadline and independent health identity.
    ///
    /// # Errors
    /// Returns an I/O error if cryptographic incarnation generation or the inference owner thread
    /// cannot be initialized.
    pub fn with_pii_engine(
        config: EngineConfig,
        pii_config: PiiEngineConfig,
        idle_policy: S1IdlePolicy,
        shutdown: watch::Sender<bool>,
    ) -> Result<Self, std::io::Error> {
        Self::with_models(config, Some(pii_config), idle_policy, shutdown)
    }

    fn with_models(
        config: EngineConfig,
        pii_config: Option<PiiEngineConfig>,
        idle_policy: S1IdlePolicy,
        shutdown: watch::Sender<bool>,
    ) -> Result<Self, std::io::Error> {
        let pii_configured = pii_config.is_some();
        let mut worker_incarnation = [0_u8; 16];
        getrandom::fill(&mut worker_incarnation)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let (commands, receiver) = mpsc::channel(2);
        let model_identity = Arc::new(std::sync::RwLock::new(None));
        let owner_identity = Arc::clone(&model_identity);
        let pii_model_identity = Arc::new(std::sync::RwLock::new(None));
        let owner_pii_identity = Arc::clone(&pii_model_identity);
        let model_paused = Arc::new(AtomicBool::new(false));
        let owner_paused = Arc::clone(&model_paused);
        let thread = std::thread::Builder::new()
            .name("polaris-s1-inference".to_owned())
            .spawn(move || {
                inference_loop(
                    config,
                    pii_config,
                    idle_policy,
                    receiver,
                    owner_identity,
                    owner_pii_identity,
                    owner_paused,
                )
            })?;
        Ok(Self {
            commands,
            thread: Arc::new(AsyncMutex::new(Some(thread))),
            permits: Arc::new(Semaphore::new(2)),
            closing: Arc::new(AtomicBool::new(false)),
            drain_lock: Arc::new(AsyncMutex::new(())),
            shutdown,
            model_identity,
            pii_model_identity,
            worker_incarnation,
            pii_configured,
            model_paused,
        })
    }
}

fn inference_loop(
    config: EngineConfig,
    pii_config: Option<PiiEngineConfig>,
    idle_policy: S1IdlePolicy,
    mut receiver: mpsc::Receiver<InferenceCommand>,
    model_identity: SharedModelIdentity,
    pii_model_identity: SharedPiiModelIdentity,
    model_paused: Arc<AtomicBool>,
) -> std::io::Result<()> {
    // `gliner2-rs` contains thread-affine ONNX handles. Keep construction, inference and drop on
    // this dedicated thread; only owned SDK values and serialized protobuf results cross it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    let mut session = idle::IdleSession::new();
    let mut model_generation = 0_u64;
    let mut pii_model: Option<GlinerPiiEngine> = None;
    let mut pii_model_generation = 0_u64;
    let mut pii_deadline = None;
    let mut pause = model_pause::ModelPause::default();
    loop {
        let now = std::time::Instant::now();
        let expired = session.expire(now);
        if expired {
            clear_model_identity(&model_identity);
        }
        if pii_deadline.is_some_and(|deadline| now >= deadline) {
            pii_model = None;
            pii_deadline = None;
            clear_model_identity(&pii_model_identity);
        }
        let received = runtime.block_on(async {
            let deadline = match (session.deadline(), pii_deadline) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
                (None, None) => None,
            };
            match deadline {
                Some(deadline) => {
                    tokio::time::timeout_at(
                        tokio::time::Instant::from_std(deadline),
                        receiver.recv(),
                    )
                    .await
                }
                None => Ok(receiver.recv().await),
            }
        });
        let command = match received {
            Ok(Some(command)) => command,
            Ok(None) => break,
            Err(_) => {
                let _evicted = session.evict();
                clear_model_identity(&model_identity);
                continue;
            }
        };
        match command {
            InferenceCommand::Decide {
                work,
                _permit,
                response,
            } => {
                let _permit_guard = _permit;
                if response.is_closed() {
                    continue;
                }
                if pause.paused() {
                    let _sent =
                        response.send(Err("inference paused for host execution".to_owned()));
                    continue;
                }
                pii_model = None;
                pii_deadline = None;
                clear_model_identity(&pii_model_identity);
                let result = (|| {
                    let was_loaded = session.is_loaded();
                    let loaded = session.get_or_load(|| {
                        GlinerEngine::load(config.clone())
                            .map_err(|error| format!("configured model unavailable: {error}"))
                    })?;
                    if !was_loaded {
                        if let Some(next_generation) = model_generation.checked_add(1) {
                            model_generation = next_generation;
                            if let Ok(mut current) = model_identity.write() {
                                *current = Some((loaded.model_identity_blake3(), model_generation));
                            }
                        } else {
                            clear_model_identity(&model_identity);
                        }
                    }
                    work.into_iter()
                        .map(|(questions, input)| {
                            loaded
                                .decide(&questions, &input)
                                .map(|decision| pb::Decision::from(&decision))
                                .map_err(|error| error.to_string())
                        })
                        .collect::<Result<Vec<_>, _>>()
                })();
                session.completed_work(std::time::Instant::now(), idle_policy)?;
                let _ = response.send(result);
                drop(_permit_guard);
            }
            InferenceCommand::DetectPii {
                raw_text,
                _permit,
                response,
            } => {
                let _permit_guard = _permit;
                if response.is_closed() {
                    continue;
                }
                if pause.paused() {
                    let _sent =
                        response.send(Err("inference paused for host execution".to_owned()));
                    continue;
                }
                let result = (|| {
                    let loaded = match &mut pii_model {
                        Some(engine) => engine,
                        None => {
                            session.evict();
                            clear_model_identity(&model_identity);
                            let pii_config = pii_config.as_ref().ok_or_else(|| {
                                "PII companion model is not configured".to_owned()
                            })?;
                            let engine =
                                GlinerPiiEngine::load(pii_config.clone()).map_err(|error| {
                                    format!("configured PII model unavailable: {error}")
                                })?;
                            let next_generation = pii_model_generation
                                .checked_add(1)
                                .ok_or_else(|| "PII model generation exhausted".to_owned())?;
                            pii_model_generation = next_generation;
                            if let Ok(mut current) = pii_model_identity.write() {
                                *current = Some((engine.model_identity_blake3(), next_generation));
                            }
                            pii_model.insert(engine)
                        }
                    };
                    loaded
                        .detect(&raw_text)
                        .map(|spans| {
                            spans
                                .into_iter()
                                .map(|span| pb::PiiEntitySpan {
                                    start: span.start(),
                                    end: span.end(),
                                    confidence: span.confidence(),
                                })
                                .collect()
                        })
                        .map_err(|error| error.to_string())
                })();
                pii_deadline = Some(
                    std::time::Instant::now()
                        .checked_add(idle_policy.idle_ttl())
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "PII idle deadline overflow",
                            )
                        })?,
                );
                let _ = response.send(result);
                drop(_permit_guard);
            }
            InferenceCommand::Evict {
                pause_epoch,
                resume,
                response,
            } => {
                if let Err(error) = pause.apply(pause_epoch, resume) {
                    let _sent = response.send(Err(error));
                    continue;
                }
                model_paused.store(pause.paused(), Ordering::Release);
                if resume {
                    let _sent = response.send(Ok(pb::EvictModelsResponse {
                        evicted: 0,
                        pause_epoch: pause.epoch(),
                        paused: pause.paused(),
                    }));
                    continue;
                }
                let decide_evicted = session.evict();
                let pii_evicted = pii_model.take().is_some();
                pii_deadline = None;
                if decide_evicted {
                    clear_model_identity(&model_identity);
                }
                if pii_evicted {
                    clear_model_identity(&pii_model_identity);
                }
                let count = match (decide_evicted, pii_evicted) {
                    (true, true) => 2,
                    (true, false) | (false, true) => 1,
                    (false, false) => 0,
                };
                let _sent = response.send(Ok(pb::EvictModelsResponse {
                    evicted: count,
                    pause_epoch: pause.epoch(),
                    paused: pause.paused(),
                }));
            }
            InferenceCommand::Shutdown { response } => {
                let _evicted = session.evict();
                clear_model_identity(&model_identity);
                clear_model_identity(&pii_model_identity);
                let _ = response.send(());
                break;
            }
        }
    }
    Ok(())
}

fn clear_model_identity(identity: &std::sync::RwLock<Option<LoadedModelIdentity>>) {
    if let Ok(mut current) = identity.write() {
        *current = None;
    }
}

#[tonic::async_trait]
impl pb::worker_service_server::WorkerService for S1Worker {
    async fn handshake(
        &self,
        request: Request<pb::WorkerHandshakeRequest>,
    ) -> Result<Response<pb::WorkerHandshakeResponse>, Status> {
        let peer = request
            .into_inner()
            .protocol_version
            .ok_or_else(|| Status::invalid_argument("protocol_version is required"))?;
        let peer = ProtocolVersion::try_from(peer)
            .map_err(|_| Status::invalid_argument("invalid protocol version"))?;
        let version = negotiate(PROTOCOL_VERSION, peer)
            .map_err(|_| Status::failed_precondition("protocol major version mismatch"))?;
        Ok(Response::new(pb::WorkerHandshakeResponse {
            protocol_version: Some(version.into()),
            kind: i32::from(pb::WorkerKind::S1),
            worker_version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }

    async fn decide(
        &self,
        request: Request<pb::DecideRequest>,
    ) -> Result<Response<pb::DecideResponse>, Status> {
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        if self.model_paused.load(Ordering::Acquire) {
            return Err(Status::unavailable("native inference is paused"));
        }
        let items = request.into_inner().items;
        if items.is_empty() || items.len() > MAX_BATCH {
            return Err(Status::resource_exhausted(
                "decision batch must contain 1 to 8 items",
            ));
        }
        let mut work = Vec::with_capacity(items.len());
        for item in items {
            if item.questions.is_empty() || item.questions.len() > MAX_QUESTIONS {
                return Err(Status::invalid_argument(
                    "each decision needs 1 to 64 questions",
                ));
            }
            let input = item
                .input
                .ok_or_else(|| Status::invalid_argument("decision input is required"))?;
            let context_bytes = input.context.as_ref().map_or(0, String::len);
            let total = input
                .text
                .len()
                .checked_add(context_bytes)
                .and_then(|size| size.checked_add(if input.context.is_some() { 2 } else { 0 }))
                .ok_or_else(|| Status::resource_exhausted("input size overflow"))?;
            if total > MAX_INPUT_BYTES {
                return Err(Status::resource_exhausted(
                    "decision input exceeds 262144 bytes",
                ));
            }
            let questions = item
                .questions
                .into_iter()
                .map(Question::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| Status::invalid_argument("decision schema is malformed"))?;
            work.push((
                questions,
                DecisionInput {
                    text: input.text,
                    context: input.context,
                },
            ));
        }
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("inference capacity is full"))?;
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        let (response, receiver) = oneshot::channel();
        self.commands
            .try_send(InferenceCommand::Decide {
                work,
                _permit: permit,
                response,
            })
            .map_err(|_| Status::resource_exhausted("inference queue is full or stopped"))?;
        let decisions = receiver
            .await
            .map_err(|_| Status::unavailable("inference thread stopped"))?
            .map_err(|_| Status::unavailable("configured model inference failed"));
        let decisions = decisions?;
        Ok(Response::new(pb::DecideResponse { decisions }))
    }

    async fn detect_pii(
        &self,
        request: Request<pb::DetectPiiRequest>,
    ) -> Result<Response<pb::DetectPiiResponse>, Status> {
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        if self.model_paused.load(Ordering::Acquire) {
            return Err(Status::unavailable("native inference is paused"));
        }
        if !self.pii_configured {
            return Err(Status::unavailable("PII companion model is not configured"));
        }
        let raw_text = request.into_inner().raw_text;
        if raw_text.len() > polaris_proto::limits::MAX_PII_INPUT_BYTES {
            return Err(Status::resource_exhausted("PII input exceeds 16384 bytes"));
        }
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("inference capacity is full"))?;
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        let (response, receiver) = oneshot::channel();
        self.commands
            .try_send(InferenceCommand::DetectPii {
                raw_text,
                _permit: permit,
                response,
            })
            .map_err(|_| Status::resource_exhausted("inference queue is full or stopped"))?;
        let spans = receiver
            .await
            .map_err(|_| Status::unavailable("inference thread stopped"))?
            .map_err(|_| Status::unavailable("configured PII model inference failed"))?;
        if spans.len() > MAX_PII_SPANS {
            return Err(Status::resource_exhausted(
                "PII model returned more spans than the protocol cap",
            ));
        }
        Ok(Response::new(pb::DetectPiiResponse { spans }))
    }

    async fn embed(
        &self,
        _request: Request<pb::EmbedRequest>,
    ) -> Result<Response<pb::EmbedResponse>, Status> {
        Err(Status::unimplemented(
            "S1 worker does not provide embeddings",
        ))
    }

    async fn ingest(
        &self,
        _request: Request<pb::IngestRequest>,
    ) -> Result<Response<pb::IngestResponse>, Status> {
        Err(Status::unimplemented(
            "ingest requires configured CAS access and the ingest worker mode",
        ))
    }

    async fn evict_models(
        &self,
        request: Request<pb::EvictModelsRequest>,
    ) -> Result<Response<pb::EvictModelsResponse>, Status> {
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        let (response, receiver) = oneshot::channel();
        let request = request.into_inner();
        self.commands
            .try_send(InferenceCommand::Evict {
                pause_epoch: request.pause_epoch,
                resume: request.resume,
                response,
            })
            .map_err(|_| Status::resource_exhausted("inference queue is full or stopped"))?;
        let result = receiver
            .await
            .map_err(|_| Status::unavailable("inference thread stopped"))?
            .map_err(Status::failed_precondition)?;
        if result.evicted > 2 {
            return Err(Status::internal(
                "native eviction count exceeded model capacity",
            ));
        }
        Ok(Response::new(result))
    }

    async fn health(
        &self,
        _request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        let (loaded_model_identity_blake3, loaded_model_generation) =
            match self.model_identity.read() {
                Ok(identity) => match *identity {
                    Some((digest, generation)) => (digest.to_vec(), generation),
                    None => (Vec::new(), 0),
                },
                Err(_) => (Vec::new(), 0),
            };
        let (loaded_pii_model_identity_blake3, loaded_pii_model_generation) =
            match self.pii_model_identity.read() {
                Ok(identity) => match *identity {
                    Some((digest, generation)) => (digest.to_vec(), generation),
                    None => (Vec::new(), 0),
                },
                Err(_) => (Vec::new(), 0),
            };
        Ok(Response::new(pb::HealthResponse {
            protocol_version: Some(PROTOCOL_VERSION.into()),
            build_version: env!("CARGO_PKG_VERSION").to_owned(),
            ready: !self.closing.load(Ordering::Acquire) && !self.commands.is_closed(),
            loaded_model_identity_blake3,
            loaded_model_generation,
            worker_incarnation: self.worker_incarnation.to_vec(),
            loaded_pii_model_identity_blake3,
            loaded_pii_model_generation,
        }))
    }

    async fn shutdown(
        &self,
        request: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        let grace_ms = request.into_inner().grace_ms;
        if grace_ms > 120_000 {
            return Err(Status::invalid_argument("shutdown grace exceeds 120000 ms"));
        }
        let grace = Duration::from_millis(u64::from(grace_ms));
        let _ = self.shutdown.send(true);
        self.drain_local(grace).await?;
        Ok(Response::new(pb::ShutdownResponse {}))
    }
}

impl S1Worker {
    /// Closes admission, drains queued native work and drops the model on its owner thread.
    ///
    /// A grace period above 120 seconds is rejected. A deadline error means native inference did
    /// not finish in time; it is not forcibly preempted.
    pub async fn drain_local(&self, grace: Duration) -> Result<(), Status> {
        if grace > Duration::from_secs(120) {
            return Err(Status::invalid_argument(
                "shutdown grace exceeds 120 seconds",
            ));
        }
        self.closing.store(true, Ordering::Release);
        tokio::time::timeout(grace, async {
            let _drain_guard = self.drain_lock.lock().await;
            if !self.commands.is_closed() {
                let (response, receiver) = oneshot::channel();
                self.commands
                    .send(InferenceCommand::Shutdown { response })
                    .await
                    .map_err(|_| Status::unavailable("inference thread stopped"))?;
                receiver
                    .await
                    .map_err(|_| Status::unavailable("inference thread stopped"))?;
            }
            let mut thread_guard = self.thread.lock().await;
            if let Some(thread) = thread_guard.as_ref() {
                while !thread.is_finished() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
            if let Some(thread) = thread_guard.take() {
                thread
                    .join()
                    .map_err(|_| Status::internal("inference thread panicked"))?
                    .map_err(|_| Status::internal("inference owner runtime failed"))?;
            }
            Ok::<(), Status>(())
        })
        .await
        .map_err(|_| Status::deadline_exceeded("inference did not drain within shutdown grace"))?
    }
}

enum ServerWaitError<E> {
    Server(E),
    Deadline,
}

async fn bounded_server<F, E>(
    server: F,
    shutdown: watch::Receiver<bool>,
    grace: Duration,
) -> Result<(), ServerWaitError<E>>
where
    F: Future<Output = Result<(), E>>,
{
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result.map_err(ServerWaitError::Server),
        _ = shutdown_signal(shutdown) => {}
    }
    match tokio::time::timeout(grace, &mut server).await {
        Ok(result) => result.map_err(ServerWaitError::Server),
        Err(_) => Err(ServerWaitError::Deadline),
    }
}

fn server_wait_result<E>(
    result: Result<(), ServerWaitError<E>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match result {
        Ok(()) => Ok(()),
        Err(ServerWaitError::Server(error)) => Err(Box::new(error)),
        Err(ServerWaitError::Deadline) => Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "worker transport did not drain within the configured shutdown grace",
        ))),
    }
}

#[cfg(unix)]
pub async fn serve_unix<S>(
    path: PathBuf,
    service: S,
    authenticator: WorkerAuthenticator,
    shutdown: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: pb::worker_service_server::WorkerService + Send + Sync + 'static,
{
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::ReceiverStream;

    if !path.is_absolute() || path.as_os_str().len() > 100 {
        return Err("invalid Unix socket path".into());
    }
    let parent = path.parent().ok_or("socket parent is required")?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("Unix socket parent must be a private, non-symlink directory".into());
    }
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(
            "Unix socket path already exists; refusing to displace another listener".into(),
        );
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let bound_identity = std::fs::symlink_metadata(&path)?;
    let bound_device = bound_identity.dev();
    let bound_inode = bound_identity.ino();
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    let accept_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    if sender.send(Ok::<_, std::io::Error>(stream)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let incoming = ReceiverStream::new(receiver);
    let server = tonic::transport::Server::builder()
        .max_concurrent_streams(Some(32))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            pb::worker_service_server::WorkerServiceServer::new(service)
                .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES),
            authenticator,
        ))
        .serve_with_incoming_shutdown(incoming, shutdown_signal(shutdown.clone()));
    let result = bounded_server(server, shutdown, TRANSPORT_SHUTDOWN_GRACE).await;
    accept_task.abort();
    let _ = accept_task.await;
    if let Ok(current) = std::fs::symlink_metadata(&path) {
        if current.dev() == bound_device && current.ino() == bound_inode {
            let _ = std::fs::remove_file(path);
        }
    }
    server_wait_result(result)
}

#[cfg(unix)]
async fn shutdown_signal(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(windows)]
pub async fn serve_named_pipe<S>(
    name: String,
    service: S,
    authenticator: WorkerAuthenticator,
    shutdown: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: pb::worker_service_server::WorkerService + Send + Sync + 'static,
{
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::net::windows::named_pipe::ServerOptions;
    use tokio_stream::wrappers::ReceiverStream;

    // Remote clients are rejected. The Windows default pipe DACL uses the creator's token; this
    // implementation does not verify a particular user/process restriction in that DACL.
    use tonic::transport::server::Connected;

    struct PipeIo(tokio::net::windows::named_pipe::NamedPipeServer);
    impl Connected for PipeIo {
        type ConnectInfo = ();
        fn connect_info(&self) {}
    }
    impl AsyncRead for PipeIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_read(cx, buffer)
        }
    }
    impl AsyncWrite for PipeIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, bytes)
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    let prefix = r"\\.\pipe\polaris-";
    let suffix = name
        .strip_prefix(prefix)
        .ok_or("pipe name must begin with \\\\.\\pipe\\polaris-")?;
    if suffix.is_empty()
        || suffix.len() > 128
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid local pipe name".into());
    }
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(true)
        .reject_remote_clients(true);
    let first = options.create(&name)?;
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    let accept_task = tokio::spawn(async move {
        let mut next = first;
        loop {
            // Establish the next exclusive instance before publishing this connected one.
            let mut options = ServerOptions::new();
            options.reject_remote_clients(true);
            let replacement = match options.create(&name) {
                Ok(server) => server,
                Err(_) => break,
            };
            if next.connect().await.is_err()
                || sender.send(Ok::<_, io::Error>(PipeIo(next))).await.is_err()
            {
                break;
            }
            next = replacement;
        }
    });
    let incoming = ReceiverStream::new(receiver);
    let server = tonic::transport::Server::builder()
        .max_concurrent_streams(Some(32))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            pb::worker_service_server::WorkerServiceServer::new(service)
                .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES),
            authenticator,
        ))
        .serve_with_incoming_shutdown(incoming, shutdown_signal(shutdown.clone()));
    let result = bounded_server(server, shutdown, TRANSPORT_SHUTDOWN_GRACE).await;
    accept_task.abort();
    let _ = accept_task.await;
    server_wait_result(result)
}

#[cfg(windows)]
async fn shutdown_signal(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{S1IdlePolicy, S1Worker, bounded_server};
    use polaris_proto::pb;
    use polaris_system_one_engine::{EngineConfig, PiiEngineConfig, TextNormalization};
    use tokio::sync::watch;
    use tonic::Request;

    fn worker() -> Result<(S1Worker, watch::Receiver<bool>), std::io::Error> {
        let (shutdown, receiver) = watch::channel(false);
        let config = EngineConfig::new(
            PathBuf::from("missing-model"),
            PathBuf::from("missing-runtime.dll"),
            TextNormalization::Raw,
        );
        Ok((S1Worker::new(config, shutdown)?, receiver))
    }

    fn worker_with_pii_config() -> Result<(S1Worker, watch::Receiver<bool>), std::io::Error> {
        let (shutdown, receiver) = watch::channel(false);
        let config = EngineConfig::new(
            PathBuf::from("missing-decide-model"),
            PathBuf::from("missing-runtime.dll"),
            TextNormalization::Raw,
        );
        let pii = PiiEngineConfig::new(
            PathBuf::from("missing-pii-model"),
            PathBuf::from("missing-runtime.dll"),
        );
        Ok((
            S1Worker::with_pii_engine(config, pii, S1IdlePolicy::default(), shutdown)?,
            receiver,
        ))
    }

    #[tokio::test]
    async fn empty_decision_batch_is_rejected_before_model_loading()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _shutdown) = worker()?;
        let response = pb::worker_service_server::WorkerService::decide(
            &worker,
            Request::new(pb::DecideRequest { items: Vec::new() }),
        )
        .await;
        assert!(matches!(response, Err(status) if status.code() == tonic::Code::ResourceExhausted));
        Ok(())
    }

    #[tokio::test]
    async fn health_has_no_loaded_model_identity_before_first_decision()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _shutdown) = worker()?;
        let response = pb::worker_service_server::WorkerService::health(
            &worker,
            Request::new(pb::HealthRequest {}),
        )
        .await?
        .into_inner();
        assert!(response.loaded_model_identity_blake3.is_empty());
        assert_eq!(response.loaded_model_generation, 0);
        assert_eq!(response.worker_incarnation.len(), 16);
        assert!(response.loaded_pii_model_identity_blake3.is_empty());
        assert_eq!(response.loaded_pii_model_generation, 0);
        worker.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn worker_incarnation_changes_across_process_owners()
    -> Result<(), Box<dyn std::error::Error>> {
        let (first, _first_shutdown) = worker()?;
        let (second, _second_shutdown) = worker()?;
        let first_health = pb::worker_service_server::WorkerService::health(
            &first,
            Request::new(pb::HealthRequest {}),
        )
        .await?
        .into_inner();
        let second_health = pb::worker_service_server::WorkerService::health(
            &second,
            Request::new(pb::HealthRequest {}),
        )
        .await?
        .into_inner();
        assert_ne!(
            first_health.worker_incarnation,
            second_health.worker_incarnation
        );
        first.drain_local(Duration::from_secs(1)).await?;
        second.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn pii_rpc_unavailable_is_explicit_when_no_companion_model_is_configured()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _shutdown) = worker()?;
        let result = pb::worker_service_server::WorkerService::detect_pii(
            &worker,
            Request::new(pb::DetectPiiRequest {
                raw_text: "Alice Example".to_owned(),
            }),
        )
        .await;
        assert!(matches!(result, Err(status) if status.code() == tonic::Code::Unavailable));
        worker.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn pii_input_cap_is_checked_before_loading_configured_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _shutdown) = worker_with_pii_config()?;
        let result = pb::worker_service_server::WorkerService::detect_pii(
            &worker,
            Request::new(pb::DetectPiiRequest {
                raw_text: "x".repeat(polaris_proto::limits::MAX_PII_INPUT_BYTES + 1),
            }),
        )
        .await;
        assert!(matches!(result, Err(status) if status.code() == tonic::Code::ResourceExhausted));
        worker.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn ingest_and_embedding_do_not_claim_s1_results() -> Result<(), Box<dyn std::error::Error>>
    {
        let (worker, _shutdown) = worker()?;
        let embedding = pb::worker_service_server::WorkerService::embed(
            &worker,
            Request::new(pb::EmbedRequest { texts: Vec::new() }),
        )
        .await;
        let ingest = pb::worker_service_server::WorkerService::ingest(
            &worker,
            Request::new(pb::IngestRequest {
                source: None,
                media_type: String::new(),
            }),
        )
        .await;
        assert!(matches!(embedding, Err(status) if status.code() == tonic::Code::Unimplemented));
        assert!(matches!(ingest, Err(status) if status.code() == tonic::Code::Unimplemented));
        Ok(())
    }

    #[tokio::test]
    async fn canceled_shutdown_still_signals_transport_stop()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, mut shutdown) = worker()?;
        let drain_guard = worker.drain_lock.lock().await;
        let shutdown_worker = worker.clone();
        let shutdown_call = tokio::spawn(async move {
            pb::worker_service_server::WorkerService::shutdown(
                &shutdown_worker,
                Request::new(pb::ShutdownRequest { grace_ms: 60_000 }),
            )
            .await
        });
        tokio::time::timeout(Duration::from_millis(100), shutdown.changed()).await??;
        shutdown_call.abort();
        match shutdown_call.await {
            Ok(_) | Err(_) => {}
        }
        drop(drain_guard);
        worker.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn drain_lock_wait_observes_requested_grace() -> Result<(), Box<dyn std::error::Error>> {
        let (worker, _shutdown) = worker()?;
        let drain_guard = worker.drain_lock.lock().await;
        let started = tokio::time::Instant::now();
        let draining_worker = worker.clone();
        let mut drainage =
            tokio::spawn(
                async move { draining_worker.drain_local(Duration::from_millis(20)).await },
            );
        tokio::time::sleep(Duration::from_millis(1)).await;
        let health = pb::worker_service_server::WorkerService::health(
            &worker,
            Request::new(pb::HealthRequest {}),
        )
        .await?;
        assert!(!health.into_inner().ready);
        let result = tokio::time::timeout(Duration::from_millis(200), &mut drainage).await;
        assert!(
            matches!(result, Ok(Ok(Err(status))) if status.code() == tonic::Code::DeadlineExceeded)
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(drain_guard);
        worker.drain_local(Duration::from_secs(1)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn transport_shutdown_wait_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let (_shutdown, receiver) = watch::channel(true);
        let result = bounded_server(
            std::future::pending::<Result<(), std::io::Error>>(),
            receiver,
            Duration::from_millis(20),
        )
        .await;
        assert!(matches!(result, Err(super::ServerWaitError::Deadline)));
        Ok(())
    }
}
