//! Bounded local S1 worker RPC service. Native inference is lazy-loaded and isolated here.

#![forbid(unsafe_code)]

#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use polaris_proto::worker_auth::WorkerAuthenticator;
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, negotiate, pb};
use polaris_sdk::system_one::{DecisionInput, Question};
use polaris_system_one_engine::{EngineConfig, GlinerEngine};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc, oneshot, watch};
use tonic::{Request, Response, Status};

const MAX_BATCH: usize = 8;
const MAX_QUESTIONS: usize = 64;
const MAX_INPUT_BYTES: usize = 262_144;

mod ingest;
pub use ingest::IngestWorker;

#[derive(Clone)]
pub struct S1Worker {
    commands: mpsc::Sender<InferenceCommand>,
    thread: Arc<AsyncMutex<Option<std::thread::JoinHandle<()>>>>,
    permits: Arc<Semaphore>,
    closing: Arc<AtomicBool>,
    drain_lock: Arc<AsyncMutex<()>>,
    shutdown: watch::Sender<bool>,
}

enum InferenceCommand {
    Decide {
        work: Vec<(Vec<Question>, DecisionInput)>,
        _permit: tokio::sync::OwnedSemaphorePermit,
        response: oneshot::Sender<Result<Vec<pb::Decision>, String>>,
    },
    Evict {
        response: oneshot::Sender<bool>,
    },
    Shutdown {
        response: oneshot::Sender<()>,
    },
}

impl S1Worker {
    #[must_use]
    pub fn new(
        config: EngineConfig,
        shutdown: watch::Sender<bool>,
    ) -> Result<Self, std::io::Error> {
        let (commands, receiver) = mpsc::channel(2);
        let thread = std::thread::Builder::new()
            .name("polaris-s1-inference".to_owned())
            .spawn(move || inference_loop(config, receiver))?;
        Ok(Self {
            commands,
            thread: Arc::new(AsyncMutex::new(Some(thread))),
            permits: Arc::new(Semaphore::new(2)),
            closing: Arc::new(AtomicBool::new(false)),
            drain_lock: Arc::new(AsyncMutex::new(())),
            shutdown,
        })
    }
}

fn inference_loop(config: EngineConfig, mut receiver: mpsc::Receiver<InferenceCommand>) {
    // `gliner2-rs` contains thread-affine ONNX handles. Keep construction, inference and drop on
    // this dedicated thread; only owned SDK values and serialized protobuf results cross it.
    let mut engine: Option<GlinerEngine> = None;
    while let Some(command) = receiver.blocking_recv() {
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
                let result = (|| {
                    if engine.is_none() {
                        engine =
                            Some(GlinerEngine::load(config.clone()).map_err(|error| {
                                format!("configured model unavailable: {error}")
                            })?);
                    }
                    let loaded = engine
                        .as_ref()
                        .ok_or_else(|| "model initialization failed".to_owned())?;
                    work.into_iter()
                        .map(|(questions, input)| {
                            loaded
                                .decide(&questions, &input)
                                .map(|decision| pb::Decision::from(&decision))
                                .map_err(|error| error.to_string())
                        })
                        .collect::<Result<Vec<_>, _>>()
                })();
                let _ = response.send(result);
                drop(_permit_guard);
            }
            InferenceCommand::Evict { response } => {
                let _ = response.send(engine.take().is_some());
            }
            InferenceCommand::Shutdown { response } => {
                drop(engine.take());
                let _ = response.send(());
                break;
            }
        }
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
        _request: Request<pb::EvictModelsRequest>,
    ) -> Result<Response<pb::EvictModelsResponse>, Status> {
        if self.closing.load(Ordering::Acquire) {
            return Err(Status::unavailable("worker is shutting down"));
        }
        let (response, receiver) = oneshot::channel();
        self.commands
            .try_send(InferenceCommand::Evict { response })
            .map_err(|_| Status::resource_exhausted("inference queue is full or stopped"))?;
        let evicted = u32::from(
            receiver
                .await
                .map_err(|_| Status::unavailable("inference thread stopped"))?,
        );
        Ok(Response::new(pb::EvictModelsResponse { evicted }))
    }

    async fn health(
        &self,
        _request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        Ok(Response::new(pb::HealthResponse {
            protocol_version: Some(PROTOCOL_VERSION.into()),
            build_version: env!("CARGO_PKG_VERSION").to_owned(),
            ready: !self.closing.load(Ordering::Acquire) && !self.commands.is_closed(),
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
        let drained = self.drain_local(grace).await;
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = shutdown.send(true);
        });
        drained?;
        Ok(Response::new(pb::ShutdownResponse {}))
    }
}

impl S1Worker {
    pub async fn drain_local(&self, grace: Duration) -> Result<(), Status> {
        if grace > Duration::from_secs(120) {
            return Err(Status::invalid_argument(
                "shutdown grace exceeds 120 seconds",
            ));
        }
        let _drain_guard = self.drain_lock.lock().await;
        self.closing.store(true, Ordering::Release);
        tokio::time::timeout(grace, async {
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
            if let Some(thread) = self.thread.lock().await.take() {
                tokio::task::spawn_blocking(move || thread.join())
                    .await
                    .map_err(|_| Status::internal("inference thread join task failed"))?
                    .map_err(|_| Status::internal("inference thread panicked"))?;
            }
            Ok::<(), Status>(())
        })
        .await
        .map_err(|_| Status::deadline_exceeded("inference did not drain within shutdown grace"))?
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
    let result = tonic::transport::Server::builder()
        .max_concurrent_streams(Some(32))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            pb::worker_service_server::WorkerServiceServer::new(service)
                .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES),
            authenticator,
        ))
        .serve_with_incoming_shutdown(incoming, shutdown_signal(shutdown))
        .await;
    accept_task.abort();
    let _ = accept_task.await;
    result?;
    if let Ok(current) = std::fs::symlink_metadata(&path) {
        if current.dev() == bound_device && current.ino() == bound_inode {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

#[cfg(unix)]
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

    use super::S1Worker;
    use polaris_proto::pb;
    use polaris_system_one_engine::{EngineConfig, TextNormalization};
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
    let result = tonic::transport::Server::builder()
        .max_concurrent_streams(Some(32))
        .add_service(tonic::service::interceptor::InterceptedService::new(
            pb::worker_service_server::WorkerServiceServer::new(service)
                .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES),
            authenticator,
        ))
        .serve_with_incoming_shutdown(incoming, shutdown_signal(shutdown))
        .await;
    accept_task.abort();
    let _ = accept_task.await;
    result?;
    Ok(())
}

#[cfg(windows)]
async fn shutdown_signal(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}
