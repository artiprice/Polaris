//! Kernel-side System-One client for the bounded `polaris.v1.WorkerService` RPC.
//!
//! The client supports configured local named-pipe and Unix-domain-socket endpoints and an injected
//! tonic channel. It validates the protocol handshake, limits each request, applies a separate
//! timeout to handshake and decision RPCs, and propagates worker errors. Concurrent decisions are
//! coalesced over the existing repeated-items RPC. Cached answers require a fresh authenticated
//! health response carrying the worker's loaded-model digest and generation.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use polaris_proto::worker_auth::WorkerCredential;
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, negotiate, pb};
use polaris_sdk::registry::{DecisionEngine, DecisionError};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, PiiEntitySpan, PiiSpanDetector, Question, QuestionKind,
};
use prost::Message;
use std::collections::HashSet;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

mod batch;
mod cache;
mod singleflight;

const MAX_INPUT_BYTES: usize = 262_144;

/// Bounds and deadline for calls through one worker channel.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    /// Maximum wall-clock time for each handshake, health, or decision RPC.
    pub timeout: Duration,
    /// Maximum combined primary text and context length in UTF-8 bytes.
    pub max_input_bytes: usize,
}

impl ClientConfig {
    /// Validates the timeout and input bound.
    ///
    /// # Errors
    /// Returns [`ClientError::InvalidConfiguration`] when any bound is outside the protocol cap.
    pub fn new(timeout: Duration, max_input_bytes: usize) -> Result<Self, ClientError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(120)
            || max_input_bytes == 0
            || max_input_bytes > MAX_INPUT_BYTES
        {
            return Err(ClientError::InvalidConfiguration);
        }
        Ok(Self {
            timeout,
            max_input_bytes,
        })
    }
}

/// Construction failures for an S1 client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A timeout or byte bound is zero or exceeds the supported protocol limit.
    #[error("invalid S1 client bounds")]
    InvalidConfiguration,
    /// The configured local endpoint is malformed or unsafe.
    #[error("invalid local worker endpoint")]
    InvalidEndpoint,
    /// The connected named-pipe server or Unix socket peer is not the owned worker process.
    #[error("local worker process identity mismatch")]
    PeerIdentityMismatch,
    /// The endpoint could not be opened.
    #[error("worker connection failed: {0}")]
    Connection(String),
}

/// `DecisionEngine` adapter for a single local S1 worker channel.
pub struct S1Client {
    channel: Channel,
    config: ClientConfig,
    credential: std::sync::Arc<WorkerCredential>,
    handshake: tokio::sync::OnceCell<()>,
    batch_tx: tokio::sync::mpsc::Sender<batch::PendingDecision>,
    batch_receiver: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<batch::PendingDecision>>>,
    batch_started: tokio::sync::OnceCell<()>,
    decision_cache: std::sync::Arc<cache::DecisionCache>,
    decision_flights: singleflight::SingleFlight,
}

impl std::fmt::Debug for S1Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S1Client")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl S1Client {
    /// Wraps a tonic channel created by the composition root.
    ///
    /// # Errors
    /// Returns [`ClientError::InvalidConfiguration`] for an invalid deadline or bound.
    pub fn new(
        channel: Channel,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        validate_config(config)?;
        let (batch_tx, batch_receiver) = tokio::sync::mpsc::channel(batch::QUEUE_CAPACITY);
        Ok(Self {
            channel,
            config,
            credential: std::sync::Arc::new(credential),
            handshake: tokio::sync::OnceCell::new(),
            batch_tx,
            batch_receiver: tokio::sync::Mutex::new(Some(batch_receiver)),
            batch_started: tokio::sync::OnceCell::new(),
            decision_cache: std::sync::Arc::new(cache::DecisionCache::new()),
            decision_flights: singleflight::SingleFlight::default(),
        })
    }

    /// Checks protocol compatibility and the worker's authenticated admission readiness.
    ///
    /// `true` means the service currently accepts work. It does not mean a model is loaded or
    /// that model inference has been validated. The handshake and Health RPC each have their own
    /// configured deadline, and both requests carry this client's worker credential.
    ///
    /// # Errors
    /// Returns [`DecisionError::Timeout`] on a call deadline, [`DecisionError::Unavailable`] when
    /// the service cannot be reached, or [`DecisionError::Model`] for authentication or protocol
    /// failures.
    pub async fn is_ready(&self) -> Result<bool, DecisionError> {
        self.ensure_handshake().await?;
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(pb::HealthRequest {}))
            .map_err(status_error)?;
        let response = tokio::time::timeout(self.config.timeout, client.health(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        validate_health_response(response)
    }

    /// Checks authenticated service readiness and proves that an S1 model is currently loaded.
    ///
    /// Unlike [`Self::is_ready`], this returns `true` only when the health response contains a
    /// complete loaded-model digest, nonzero generation, and worker incarnation. Hosts may use it
    /// to recover an availability circuit; service readiness alone is not enough.
    /// # Errors
    /// Returns the same bounded transport, authentication, and protocol failures as health RPCs.
    pub async fn is_model_ready(&self) -> Result<bool, DecisionError> {
        self.ensure_handshake().await?;
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(pb::HealthRequest {}))
            .map_err(status_error)?;
        let response = tokio::time::timeout(self.config.timeout, client.health(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        if !response.ready {
            return Ok(false);
        }
        let remote_version = response
            .protocol_version
            .ok_or_else(|| DecisionError::Model("worker protocol version is missing".to_owned()))?;
        let remote_version = ProtocolVersion::try_from(remote_version)
            .map_err(|_| DecisionError::Model("worker protocol version is invalid".to_owned()))?;
        negotiate(PROTOCOL_VERSION, remote_version)
            .map_err(|_| DecisionError::Model("worker protocol is incompatible".to_owned()))?;
        cache::ModelIdentity::from_health(&response)
            .map(|identity| identity.is_some())
            .map_err(|_| DecisionError::Model("worker model identity is incomplete".to_owned()))
    }

    /// Obtain a decision fenced to the exact calibrated native-model identity.
    /// A cold worker may execute one discarded warmup request before the identity is available.
    /// No permission decision is returned from that warmup. The accepted result requires equal
    /// authenticated digest, model generation and worker incarnation before and after inference.
    /// # Errors
    /// Request, handshake, deadline, identity mismatch/unavailability or inference failure.
    /// Concurrent eviction/model replacement fails closed and never reuses a foreign result.
    pub async fn decide_pinned(
        &self,
        expected: [u8; 32],
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.validate_request(schema, input)?;
        let deadline = tokio::time::Instant::now()
            .checked_add(self.config.timeout)
            .ok_or(DecisionError::Timeout)?;
        tokio::time::timeout_at(deadline, self.ensure_handshake())
            .await
            .map_err(|_| DecisionError::Timeout)??;
        let before = match self.loaded_model_identity(deadline).await {
            Some(identity) if identity.has_digest(expected) => identity,
            Some(_) | None => {
                let _discarded = tokio::time::timeout_at(deadline, self.decide_rpc(schema, input))
                    .await
                    .map_err(|_| DecisionError::Timeout)??;
                self.loaded_model_identity(deadline).await.ok_or_else(|| {
                    DecisionError::Unavailable("calibrated model identity unavailable".to_owned())
                })?
            }
        };
        if !before.has_digest(expected) {
            return Err(DecisionError::Model(
                "calibrated model identity mismatch".to_owned(),
            ));
        }
        let decision = tokio::time::timeout_at(deadline, self.decide_rpc(schema, input))
            .await
            .map_err(|_| DecisionError::Timeout)??;
        if self.loaded_model_identity(deadline).await != Some(before) {
            return Err(DecisionError::Unavailable(
                "calibrated model generation changed".to_owned(),
            ));
        }
        Ok(decision)
    }

    /// Requests the worker to drop its currently resident S1 model and returns its acknowledgement.
    ///
    /// The worker serializes this RPC with its model operations. A successful result reports the
    /// number of models it acknowledges dropping, which must be zero or one for the single native
    /// engine. This RPC does not pause heavy-build admission, preempt active work, or prove that
    /// resident-set size decreased.
    ///
    /// # Errors
    /// Returns [`DecisionError::Timeout`] when the handshake or eviction call reaches its deadline,
    /// [`DecisionError::Unavailable`] when the worker is unavailable, or [`DecisionError::Model`]
    /// for authentication, protocol, or other worker failures.
    pub async fn evict_models(&self) -> Result<u32, DecisionError> {
        self.ensure_handshake().await?;
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(pb::EvictModelsRequest::default()))
            .map_err(status_error)?;
        let response = tokio::time::timeout(self.config.timeout, client.evict_models(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        if response.evicted > 2 {
            return Err(DecisionError::Model(
                "worker acknowledged more evictions than its native model capacity permits"
                    .to_owned(),
            ));
        }
        Ok(response.evicted)
    }

    /// Pause native admission and evict both model types under an explicit host epoch.
    /// Old workers that do not acknowledge the epoch are rejected before a heavy task may start.
    /// # Errors
    /// Zero epoch, invalid timeout, authentication, queue/native drain deadline or invalid acknowledgement.
    pub async fn pause_models(&self, epoch: u64, timeout: Duration) -> Result<u32, DecisionError> {
        self.model_pause_rpc(epoch, false, timeout).await
    }

    /// Resume only the exact acknowledged pause epoch. Exact duplicate releases are idempotent.
    /// # Errors
    /// Zero/stale epoch, timeout, transport/authentication or acknowledgement failure.
    pub async fn resume_models(&self, epoch: u64, timeout: Duration) -> Result<(), DecisionError> {
        let _evicted = self.model_pause_rpc(epoch, true, timeout).await?;
        Ok(())
    }

    async fn model_pause_rpc(
        &self,
        epoch: u64,
        resume: bool,
        timeout: Duration,
    ) -> Result<u32, DecisionError> {
        if epoch == 0 || timeout.is_zero() || timeout > Duration::from_secs(120) {
            return Err(DecisionError::InvalidSchema(
                "invalid model pause bounds".to_owned(),
            ));
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or(DecisionError::Timeout)?;
        tokio::time::timeout_at(deadline, self.ensure_handshake())
            .await
            .map_err(|_| DecisionError::Timeout)??;
        let mut client = worker_client(self.channel.clone());
        let mut request = self
            .credential
            .authorize_request(Request::new(pb::EvictModelsRequest {
                pause_epoch: epoch,
                resume,
            }))
            .map_err(status_error)?;
        request.set_timeout(deadline.saturating_duration_since(tokio::time::Instant::now()));
        self.decision_cache.clear();
        let response = tokio::time::timeout_at(deadline, client.evict_models(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        if response.pause_epoch != epoch || response.paused == resume || response.evicted > 2 {
            return Err(DecisionError::Model(
                "worker model pause acknowledgement rejected".to_owned(),
            ));
        }
        Ok(response.evicted)
    }

    /// Requests semantic PII spans from the independent local companion model.
    ///
    /// The exact raw text is sent only over the authenticated worker channel. Matched text and
    /// labels are never returned. The method has a hard input/result bound and the configured RPC
    /// timeout; callers must keep the input local and pass spans only to the PII vault.
    ///
    /// # Errors
    /// Returns typed timeout, unavailable, invalid-output, or worker errors. Old workers that do
    /// not implement this additive RPC return unavailable and allow the vault's DFA fallback.
    pub async fn detect_pii_spans(
        &self,
        raw_text: &str,
    ) -> Result<Vec<PiiEntitySpan>, DecisionError> {
        const MAX_PII_SPANS: usize = 4096;
        if raw_text.len() > self.config.max_input_bytes
            || raw_text.len() > polaris_proto::limits::MAX_PII_INPUT_BYTES
        {
            return Err(DecisionError::InvalidSchema(
                "PII input exceeds the configured byte limit".to_owned(),
            ));
        }
        self.ensure_handshake().await?;
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(pb::DetectPiiRequest {
                raw_text: raw_text.to_owned(),
            }))
            .map_err(status_error)?;
        let response = tokio::time::timeout(self.config.timeout, client.detect_pii(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        if response.spans.len() > MAX_PII_SPANS {
            return Err(DecisionError::Model(
                "PII worker returned too many spans".to_owned(),
            ));
        }
        let mut spans = Vec::new();
        spans
            .try_reserve(response.spans.len())
            .map_err(|_| DecisionError::Unavailable("PII span allocation failed".to_owned()))?;
        let mut previous_end = 0_u32;
        for span in response.spans {
            if span.start >= span.end
                || span.start < previous_end
                || usize::try_from(span.end).map_or(true, |end| end > raw_text.len())
                || usize::try_from(span.start)
                    .ok()
                    .zip(usize::try_from(span.end).ok())
                    .and_then(|(start, end)| raw_text.get(start..end))
                    .is_none()
            {
                return Err(DecisionError::Model(
                    "PII worker returned an invalid UTF-8 byte span".to_owned(),
                ));
            }
            let span = PiiEntitySpan::new(span.start, span.end, span.confidence).map_err(|_| {
                DecisionError::Model("PII worker returned an invalid confidence".to_owned())
            })?;
            previous_end = span.end();
            spans.push(span);
        }
        Ok(spans)
    }

    /// Connects to a local Windows named pipe whose name begins with `polaris-`.
    ///
    /// The service rejects remote clients and creates the first pipe instance exclusively. Access
    /// for local processes depends on the creator token's default pipe DACL; this crate does not
    /// verify that the DACL restricts access to a particular user or process.
    ///
    /// # Errors
    /// Returns an error for an invalid local pipe name or a failed tonic connection.
    #[cfg(windows)]
    pub async fn connect_named_pipe(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        validate_config(config)?;
        let name = path.to_str().ok_or(ClientError::InvalidEndpoint)?;
        let prefix = r"\\.\pipe\polaris-";
        let suffix = name
            .strip_prefix(prefix)
            .ok_or(ClientError::InvalidEndpoint)?;
        if suffix.is_empty()
            || suffix.len() > 128
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let connector = tower::service_fn(move |_uri| {
            let pipe_path = path.clone();
            let timeout = config.timeout;
            async move {
                let pipe_name = pipe_path.to_str().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid pipe name")
                })?;
                let deadline = tokio::time::Instant::now()
                    .checked_add(timeout)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "worker connection deadline is invalid",
                        )
                    })?;
                loop {
                    match tokio::net::windows::named_pipe::ClientOptions::new().open(pipe_name) {
                        Ok(client) => return Ok(TokioIo::new(client)),
                        Err(error)
                            if retryable_pipe_open_error(&error)
                                && tokio::time::Instant::now() < deadline =>
                        {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_timeout(config.timeout)
            .connect_with_connector(connector)
            .await
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        Self::new(channel, config, credential)
    }

    /// Connects only when each named-pipe stream is served by the expected process identifier.
    /// The identifier is checked on the exact stream tonic will use, including reconnects.
    ///
    /// # Errors
    /// Returns an error for an invalid pipe name, failed PID query, mismatched server process, or
    /// failed tonic connection.
    #[cfg(windows)]
    pub async fn connect_named_pipe_owned(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
        expected_server_pid: u32,
    ) -> Result<Self, ClientError> {
        use interprocess::os::windows::named_pipe::{pipe_mode, tokio::DuplexPipeStream};

        validate_config(config)?;
        let name = path.to_str().ok_or(ClientError::InvalidEndpoint)?;
        let prefix = r"\\.\pipe\polaris-";
        let suffix = name
            .strip_prefix(prefix)
            .ok_or(ClientError::InvalidEndpoint)?;
        if suffix.is_empty()
            || suffix.len() > 128
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let identity_mismatch = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let connector_identity_mismatch = std::sync::Arc::clone(&identity_mismatch);
        let connector = tower::service_fn(move |_uri| {
            let pipe_path = path.clone();
            let timeout = config.timeout;
            let rejected_identity = std::sync::Arc::clone(&connector_identity_mismatch);
            async move {
                let pipe_name = pipe_path.to_str().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid pipe name")
                })?;
                let deadline = tokio::time::Instant::now()
                    .checked_add(timeout)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "worker connection deadline is invalid",
                        )
                    })?;
                loop {
                    match DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path(pipe_name).await {
                        Ok(stream) => {
                            let server_pid = stream.server_process_id()?;
                            if server_pid != expected_server_pid {
                                rejected_identity.store(true, std::sync::atomic::Ordering::Release);
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::PermissionDenied,
                                    "named-pipe server process identity mismatch",
                                ));
                            }
                            return Ok(TokioIo::new(stream));
                        }
                        Err(error)
                            if retryable_pipe_open_error(&error)
                                && tokio::time::Instant::now() < deadline =>
                        {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_timeout(config.timeout)
            .connect_with_connector(connector)
            .await
            .map_err(|error| {
                if identity_mismatch.load(std::sync::atomic::Ordering::Acquire) {
                    ClientError::PeerIdentityMismatch
                } else {
                    ClientError::Connection(error.to_string())
                }
            })?;
        Self::new(channel, config, credential)
    }

    /// Connects to a Unix socket in a private directory.
    ///
    /// # Errors
    /// Returns an error when the path is invalid, its parent is not private, or connection fails.
    #[cfg(unix)]
    pub async fn connect_unix(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        use std::os::unix::fs::PermissionsExt;

        validate_config(config)?;
        if !path.is_absolute() || path.as_os_str().len() > 100 {
            return Err(ClientError::InvalidEndpoint);
        }
        let parent = path.parent().ok_or(ClientError::InvalidEndpoint)?;
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let connector = tower::service_fn(move |_uri| {
            let socket_path = path.clone();
            async move {
                tokio::net::UnixStream::connect(socket_path)
                    .await
                    .map(TokioIo::new)
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_timeout(config.timeout)
            .connect_with_connector(connector)
            .await
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        Self::new(channel, config, credential)
    }

    /// Connects only when each Unix stream peer PID equals the expected worker process.
    /// The PID is checked on the exact stream tonic will use, including reconnects; platforms that
    /// do not report a peer PID fail closed.
    ///
    /// # Errors
    /// Returns an error for an invalid socket path, non-private parent, unavailable/mismatched peer
    /// PID, or failed tonic connection.
    #[cfg(unix)]
    pub async fn connect_unix_owned(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
        expected_server_pid: u32,
    ) -> Result<Self, ClientError> {
        use std::os::unix::fs::PermissionsExt;

        validate_config(config)?;
        if !path.is_absolute() || path.as_os_str().len() > 100 {
            return Err(ClientError::InvalidEndpoint);
        }
        let parent = path.parent().ok_or(ClientError::InvalidEndpoint)?;
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let connector = tower::service_fn(move |_uri| {
            let socket_path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(socket_path).await?;
                let peer_pid = stream.peer_cred()?.pid().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Unix socket peer PID is unavailable",
                    )
                })?;
                if peer_pid != expected_server_pid {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Unix socket peer process identity mismatch",
                    ));
                }
                Ok(TokioIo::new(stream))
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_timeout(config.timeout)
            .connect_with_connector(connector)
            .await
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        Self::new(channel, config, credential)
    }

    async fn ensure_handshake(&self) -> Result<(), DecisionError> {
        self.handshake
            .get_or_try_init(|| async {
                let mut client = worker_client(self.channel.clone());
                let request = pb::WorkerHandshakeRequest {
                    protocol_version: Some(PROTOCOL_VERSION.into()),
                };
                let request = self
                    .credential
                    .authorize_request(Request::new(request))
                    .map_err(status_error)?;
                let response = tokio::time::timeout(self.config.timeout, client.handshake(request))
                    .await
                    .map_err(|_| DecisionError::Timeout)?
                    .map_err(status_error)?
                    .into_inner();
                let remote_version = response.protocol_version.ok_or_else(|| {
                    DecisionError::Model("worker omitted protocol version".to_owned())
                })?;
                let remote_version = ProtocolVersion::try_from(remote_version).map_err(|_| {
                    DecisionError::Model("worker sent invalid protocol version".to_owned())
                })?;
                negotiate(PROTOCOL_VERSION, remote_version).map_err(|_| {
                    DecisionError::Model("worker protocol major version mismatch".to_owned())
                })?;
                let kind = pb::WorkerKind::try_from(response.kind).map_err(|_| {
                    DecisionError::Model("worker sent invalid service kind".to_owned())
                })?;
                if kind != pb::WorkerKind::S1 {
                    return Err(DecisionError::Model(
                        "worker is not an S1 service".to_owned(),
                    ));
                }
                Ok(())
            })
            .await
            .map(|_| ())
    }

    async fn decide_rpc(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.validate_request(schema, input)?;
        self.ensure_handshake().await?;
        let decision_deadline = tokio::time::Instant::now()
            .checked_add(self.config.timeout)
            .ok_or(DecisionError::Timeout)?;
        let request = pb::DecisionRequest {
            questions: schema.iter().map(pb::Question::from).collect(),
            input: Some(input.into()),
        };
        loop {
            let identity = self.loaded_model_identity(decision_deadline).await;
            if let Some(identity) = identity {
                if let Some(decision) = self.decision_cache.get(identity, &request) {
                    return Ok(decision);
                }
                match self
                    .decision_flights
                    .join(identity, request.encode_to_vec())
                {
                    singleflight::Role::Waiter(mut waiter) => {
                        let shared = waiter.wait_until(decision_deadline).await;
                        if tokio::time::Instant::now() >= decision_deadline {
                            return Err(DecisionError::Timeout);
                        }
                        if let Some(decision) = shared
                            && self.loaded_model_identity(decision_deadline).await == Some(identity)
                        {
                            return Ok(decision);
                        }
                        continue;
                    }
                    singleflight::Role::Owner(owner) => {
                        let result = self
                            .enqueue_decision(&request, schema, decision_deadline)
                            .await;
                        if let Ok(decision) = &result
                            && self.loaded_model_identity(decision_deadline).await == Some(identity)
                        {
                            owner.complete(decision.clone());
                        }
                        return result;
                    }
                    singleflight::Role::Bypass => {
                        return self
                            .enqueue_decision(&request, schema, decision_deadline)
                            .await;
                    }
                }
            }
            self.decision_cache.clear();
            return self
                .enqueue_decision(&request, schema, decision_deadline)
                .await;
        }
    }

    async fn enqueue_decision(
        &self,
        request: &pb::DecisionRequest,
        schema: &[Question],
        decision_deadline: tokio::time::Instant,
    ) -> Result<Decision, DecisionError> {
        self.batch_started
            .get_or_init(|| async {
                if let Some(receiver) = self.batch_receiver.lock().await.take() {
                    tokio::spawn(batch::run(
                        receiver,
                        self.channel.clone(),
                        self.config,
                        std::sync::Arc::clone(&self.credential),
                        std::sync::Arc::clone(&self.decision_cache),
                    ));
                }
            })
            .await;
        let (response, receiver) = tokio::sync::oneshot::channel();
        let pending = batch::PendingDecision {
            request: request.clone(),
            schema: schema.to_vec(),
            deadline: decision_deadline,
            response,
        };
        self.batch_tx
            .try_send(pending)
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    DecisionError::Unavailable("S1 decision queue is full".to_owned())
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    DecisionError::Unavailable("S1 decision dispatcher stopped".to_owned())
                }
            })?;
        tokio::time::timeout_at(decision_deadline, receiver)
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(|_| DecisionError::Unavailable("S1 decision dispatcher stopped".to_owned()))?
    }

    async fn loaded_model_identity(
        &self,
        deadline: tokio::time::Instant,
    ) -> Option<cache::ModelIdentity> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(pb::HealthRequest {}))
            .ok()?;
        let response = tokio::time::timeout(
            remaining.min(Duration::from_millis(50)),
            client.health(request),
        )
        .await
        .ok()?
        .ok()?
        .into_inner();
        if !response.ready {
            return None;
        }
        let remote_version = response.protocol_version?;
        let remote_version = ProtocolVersion::try_from(remote_version).ok()?;
        negotiate(PROTOCOL_VERSION, remote_version).ok()?;
        cache::ModelIdentity::from_health(&response).ok()?
    }

    fn validate_request(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<(), DecisionError> {
        if schema.is_empty() || schema.len() > 64 {
            return Err(DecisionError::InvalidSchema(
                "question count must be between 1 and 64".to_owned(),
            ));
        }
        const MAX_SCHEMA_BYTES: usize = 65_536;
        let mut question_ids = HashSet::with_capacity(schema.len());
        let mut prompts = HashSet::with_capacity(schema.len());
        let mut schema_bytes = 0_usize;
        let context_len = match &input.context {
            Some(context) => context.len(),
            None => 0,
        };
        let separator = if input.context.is_some() {
            2_usize
        } else {
            0_usize
        };
        let total = input
            .text
            .len()
            .checked_add(context_len)
            .and_then(|size| size.checked_add(separator))
            .ok_or_else(|| DecisionError::InvalidSchema("input size overflow".to_owned()))?;
        if total > self.config.max_input_bytes {
            return Err(DecisionError::InvalidSchema(
                "input exceeds the configured S1 byte limit".to_owned(),
            ));
        }
        for question in schema {
            if !question_ids.insert(question.id.as_str()) {
                return Err(DecisionError::InvalidSchema(
                    "question IDs must be unique".to_owned(),
                ));
            }
            let (prompt, label_bytes) = match &question.kind {
                QuestionKind::Choice { options } => {
                    if options.is_empty() || options.len() > 128 {
                        return Err(DecisionError::InvalidSchema(
                            "choice count must be between 1 and 128".to_owned(),
                        ));
                    }
                    let mut unique = HashSet::with_capacity(options.len());
                    if options.iter().any(|label| !unique.insert(label.as_str())) {
                        return Err(DecisionError::InvalidSchema(
                            "choice labels must be unique".to_owned(),
                        ));
                    }
                    let label_bytes = options
                        .iter()
                        .try_fold(0_usize, |total, label| {
                            total.checked_add(label.as_str().len())
                        })
                        .ok_or_else(|| {
                            DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                        })?;
                    (question.id.as_str(), label_bytes)
                }
                QuestionKind::Score { min, max } => {
                    if min > max
                        || !matches!(i64::from(*max).checked_sub(i64::from(*min)), Some(0..=256))
                    {
                        return Err(DecisionError::InvalidSchema(
                            "score range must be ordered and span at most 256".to_owned(),
                        ));
                    }
                    let label_bytes = (*min..=*max)
                        .try_fold(0_usize, |total, score| {
                            total.checked_add(score.to_string().len())
                        })
                        .ok_or_else(|| {
                            DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                        })?;
                    (question.id.as_str(), label_bytes)
                }
                QuestionKind::Noul { statement } => {
                    if statement.trim().is_empty() || statement.len() > 4096 {
                        return Err(DecisionError::InvalidSchema(
                            "noul statement must be non-empty and at most 4096 bytes".to_owned(),
                        ));
                    }
                    (statement.as_str(), 5)
                }
            };
            if prompt.len() > 4096 || !prompts.insert(prompt) {
                return Err(DecisionError::InvalidSchema(
                    "model prompt is oversized or duplicated".to_owned(),
                ));
            }
            schema_bytes = schema_bytes
                .checked_add(question.id.as_str().len())
                .and_then(|size| size.checked_add(prompt.len()))
                .and_then(|size| size.checked_add(label_bytes))
                .ok_or_else(|| {
                    DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                })?;
            if schema_bytes > MAX_SCHEMA_BYTES {
                return Err(DecisionError::InvalidSchema(
                    "aggregate question schema exceeds 65536 bytes".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn retryable_pipe_open_error(error: &std::io::Error) -> bool {
    // Win32 ERROR_PIPE_BUSY is 231. Tokio documents this raw OS code for a pipe that exists but
    // has no free server instance; it is not guaranteed to map to ErrorKind::WouldBlock.
    error.raw_os_error() == Some(231)
        || matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::WouldBlock
        )
}

fn validate_config(config: ClientConfig) -> Result<(), ClientError> {
    if config.timeout.is_zero()
        || config.timeout > Duration::from_secs(120)
        || config.max_input_bytes == 0
        || config.max_input_bytes > MAX_INPUT_BYTES
    {
        return Err(ClientError::InvalidConfiguration);
    }
    Ok(())
}

impl DecisionEngine for S1Client {
    async fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.decide_rpc(schema, input).await
    }
}

impl PiiSpanDetector for S1Client {
    async fn detect_pii_spans(&self, raw_text: &str) -> Result<Vec<PiiEntitySpan>, DecisionError> {
        Self::detect_pii_spans(self, raw_text).await
    }
}

fn worker_client(channel: Channel) -> pb::worker_service_client::WorkerServiceClient<Channel> {
    pb::worker_service_client::WorkerServiceClient::new(channel)
        .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
        .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
}

fn status_error(status: Status) -> DecisionError {
    match status.code() {
        tonic::Code::DeadlineExceeded => DecisionError::Timeout,
        tonic::Code::Unavailable | tonic::Code::ResourceExhausted => {
            DecisionError::Unavailable(format!("worker RPC returned {}", status.code()))
        }
        _ => DecisionError::Model(format!("worker RPC returned {}", status.code())),
    }
}

fn validate_health_response(response: pb::HealthResponse) -> Result<bool, DecisionError> {
    let remote_version = response.protocol_version.ok_or_else(|| {
        DecisionError::Model("worker health response omitted protocol version".to_owned())
    })?;
    let remote_version = ProtocolVersion::try_from(remote_version).map_err(|_| {
        DecisionError::Model("worker health response has invalid protocol version".to_owned())
    })?;
    negotiate(PROTOCOL_VERSION, remote_version).map_err(|_| {
        DecisionError::Model("worker health protocol major version mismatch".to_owned())
    })?;
    Ok(response.ready)
}

fn validate_response(schema: &[Question], decision: &Decision) -> Result<(), DecisionError> {
    if decision.answers.len() != schema.len() {
        return Err(DecisionError::Model(
            "worker returned the wrong answer count".to_owned(),
        ));
    }
    let mut returned_ids = HashSet::with_capacity(decision.answers.len());
    for (question, answer) in schema.iter().zip(&decision.answers) {
        if answer.question != question.id || !returned_ids.insert(answer.question.as_str()) {
            return Err(DecisionError::Model(
                "worker returned unexpected or duplicate question IDs".to_owned(),
            ));
        }
        match (&question.kind, &answer.answer) {
            (QuestionKind::Choice { options }, Answer::Choice { selected, probs }) => {
                let expected = options
                    .iter()
                    .map(|label| label.as_str())
                    .collect::<HashSet<_>>();
                let actual = probs
                    .iter()
                    .map(|score| score.label.as_str())
                    .collect::<HashSet<_>>();
                if probs.len() != options.len()
                    || actual.len() != probs.len()
                    || actual != expected
                    || !expected.contains(selected.as_str())
                    || probs.first().is_none_or(|score| score.label != *selected)
                    || !probs
                        .iter()
                        .zip(probs.iter().skip(1))
                        .all(|(left, right)| left.probability >= right.probability)
                {
                    return Err(DecisionError::Model(
                        "worker returned mismatched choice labels".to_owned(),
                    ));
                }
            }
            (QuestionKind::Score { min, max }, Answer::Score { value, .. })
                if value >= min && value <= max => {}
            (QuestionKind::Noul { .. }, Answer::Noul { .. }) => {}
            _ => {
                return Err(DecisionError::Model(
                    "worker returned a mismatched answer kind or score".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{ClientConfig, S1Client, WorkerCredential, validate_response};
    use polaris_sdk::registry::DecisionError;
    use polaris_sdk::system_one::{
        Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
        QuestionId, QuestionKind,
    };

    fn client(max_input_bytes: usize) -> Result<S1Client, Box<dyn std::error::Error>> {
        let channel = Endpoint::from_static("http://[::]:50051").connect_lazy();
        let config = ClientConfig::new(Duration::from_secs(2), max_input_bytes)?;
        Ok(S1Client::new(
            channel,
            config,
            WorkerCredential::generate()?,
        )?)
    }

    use tonic::transport::Endpoint;

    #[derive(Clone)]
    struct HealthService {
        ready: bool,
        model_loaded: bool,
        version: Option<polaris_proto::pb::ProtocolVersion>,
        handshakes: Arc<AtomicUsize>,
        health_calls: Arc<AtomicUsize>,
    }

    #[tonic::async_trait]
    impl polaris_proto::pb::worker_service_server::WorkerService for HealthService {
        async fn handshake(
            &self,
            _request: tonic::Request<polaris_proto::pb::WorkerHandshakeRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::WorkerHandshakeResponse>, tonic::Status>
        {
            self.handshakes.fetch_add(1, Ordering::Relaxed);
            Ok(tonic::Response::new(
                polaris_proto::pb::WorkerHandshakeResponse {
                    protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
                    kind: i32::from(polaris_proto::pb::WorkerKind::S1),
                    worker_version: "test".to_owned(),
                },
            ))
        }

        async fn decide(
            &self,
            _request: tonic::Request<polaris_proto::pb::DecideRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::DecideResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test health service"))
        }

        async fn detect_pii(
            &self,
            _request: tonic::Request<polaris_proto::pb::DetectPiiRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::DetectPiiResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test health service"))
        }

        async fn embed(
            &self,
            _request: tonic::Request<polaris_proto::pb::EmbedRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::EmbedResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test health service"))
        }

        async fn ingest(
            &self,
            _request: tonic::Request<polaris_proto::pb::IngestRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::IngestResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test health service"))
        }

        async fn evict_models(
            &self,
            _request: tonic::Request<polaris_proto::pb::EvictModelsRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::EvictModelsResponse>, tonic::Status>
        {
            Err(tonic::Status::unimplemented("test health service"))
        }

        async fn health(
            &self,
            _request: tonic::Request<polaris_proto::pb::HealthRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::HealthResponse>, tonic::Status> {
            self.health_calls.fetch_add(1, Ordering::Relaxed);
            Ok(tonic::Response::new(polaris_proto::pb::HealthResponse {
                protocol_version: self.version,
                build_version: "test".to_owned(),
                ready: self.ready,
                loaded_model_identity_blake3: if self.model_loaded {
                    vec![4; 32]
                } else {
                    Vec::new()
                },
                loaded_model_generation: if self.model_loaded { 1 } else { 0 },
                worker_incarnation: vec![7; 16],
                loaded_pii_model_identity_blake3: Vec::new(),
                loaded_pii_model_generation: 0,
            }))
        }

        async fn shutdown(
            &self,
            _request: tonic::Request<polaris_proto::pb::ShutdownRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::ShutdownResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test health service"))
        }
    }

    async fn health_client(
        ready: bool,
        model_loaded: bool,
        version: Option<polaris_proto::pb::ProtocolVersion>,
    ) -> Result<
        (
            S1Client,
            tokio::sync::oneshot::Sender<()>,
            Arc<AtomicUsize>,
            Arc<AtomicUsize>,
            tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let credential = WorkerCredential::generate()?;
        let authenticator = polaris_proto::worker_auth::WorkerAuthenticator::new(&credential);
        let handshakes = Arc::new(AtomicUsize::new(0));
        let health_calls = Arc::new(AtomicUsize::new(0));
        let service = HealthService {
            ready,
            model_loaded,
            version,
            handshakes: Arc::clone(&handshakes),
            health_calls: Arc::clone(&health_calls),
        };
        let service =
            polaris_proto::pb::worker_service_server::WorkerServiceServer::with_interceptor(
                service,
                authenticator,
            );
        let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_receiver.await;
                })
                .await
        });
        let channel = Endpoint::from_shared(format!("http://{address}"))?
            .connect()
            .await?;
        let client = S1Client::new(
            channel,
            ClientConfig::new(Duration::from_secs(2), 64)?,
            credential,
        )?;
        Ok((client, shutdown_sender, handshakes, health_calls, server))
    }

    #[tokio::test]
    async fn request_validation_counts_context_separator_and_utf8_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let client = client(9)?;
        let question = Question {
            id: QuestionId::new("kind")?,
            kind: QuestionKind::Choice {
                options: vec![Label::new("bug")?],
            },
        };
        let input = DecisionInput {
            text: "éé".to_owned(),
            context: Some("abcd".to_owned()),
        };
        assert!(matches!(
            client.validate_request(&[question], &input),
            Err(DecisionError::InvalidSchema(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn request_validation_rejects_empty_schema_without_transport()
    -> Result<(), Box<dyn std::error::Error>> {
        let client = client(64)?;
        let input = DecisionInput {
            text: "document".to_owned(),
            context: None,
        };
        assert!(matches!(
            client.validate_request(&[], &input),
            Err(DecisionError::InvalidSchema(_))
        ));
        Ok(())
    }

    #[test]
    fn response_validation_rejects_answers_for_another_question()
    -> Result<(), Box<dyn std::error::Error>> {
        let question = Question {
            id: QuestionId::new("kind")?,
            kind: QuestionKind::Choice {
                options: vec![Label::new("bug")?],
            },
        };
        let decision = Decision {
            answers: vec![QuestionAnswer {
                question: QuestionId::new("other")?,
                answer: Answer::Choice {
                    selected: Label::new("bug")?,
                    probs: vec![LabelScore {
                        label: Label::new("bug")?,
                        probability: Probability::new(1.0)?,
                    }],
                },
            }],
            latency_us: 1,
        };
        assert!(validate_response(&[question], &decision).is_err());
        Ok(())
    }

    #[test]
    fn response_validation_rejects_unselected_or_unsorted_choice_scores()
    -> Result<(), Box<dyn std::error::Error>> {
        let question = Question {
            id: QuestionId::new("kind")?,
            kind: QuestionKind::Choice {
                options: vec![Label::new("bug")?, Label::new("feature")?],
            },
        };
        let decision = Decision {
            answers: vec![QuestionAnswer {
                question: QuestionId::new("kind")?,
                answer: Answer::Choice {
                    selected: Label::new("feature")?,
                    probs: vec![
                        LabelScore {
                            label: Label::new("bug")?,
                            probability: Probability::new(0.9)?,
                        },
                        LabelScore {
                            label: Label::new("feature")?,
                            probability: Probability::new(0.1)?,
                        },
                    ],
                },
            }],
            latency_us: 1,
        };
        assert!(validate_response(&[question], &decision).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn schema_bound_counts_derived_ordinal_labels() -> Result<(), Box<dyn std::error::Error>>
    {
        let client = client(64)?;
        let schema = (0..24)
            .map(|index| -> Result<Question, Box<dyn std::error::Error>> {
                Ok(Question {
                    id: QuestionId::new(&format!("ordinal_{index}"))?,
                    kind: QuestionKind::Score {
                        min: i32::MIN,
                        max: i32::MIN + 256,
                    },
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let input = DecisionInput {
            text: "document".to_owned(),
            context: None,
        };
        assert!(matches!(
            client.validate_request(&schema, &input),
            Err(DecisionError::InvalidSchema(_))
        ));
        Ok(())
    }

    #[test]
    fn client_input_bound_matches_worker_cap() {
        assert!(ClientConfig::new(Duration::from_secs(1), 262_145).is_err());
        assert!(ClientConfig::new(Duration::from_secs(121), 1).is_err());
    }

    #[test]
    fn pipe_retry_classifies_pipe_busy_without_retrying_acl_denial() {
        let pipe_busy = std::io::Error::from_raw_os_error(231);
        let access_denied = std::io::Error::from_raw_os_error(5);
        assert!(super::retryable_pipe_open_error(&pipe_busy));
        assert!(!super::retryable_pipe_open_error(&access_denied));
    }

    #[test]
    fn health_response_validation_distinguishes_admission_from_model_readiness() {
        let response = polaris_proto::pb::HealthResponse {
            protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            build_version: "worker".to_owned(),
            ready: false,
            loaded_model_identity_blake3: Vec::new(),
            loaded_model_generation: 0,
            worker_incarnation: vec![0; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        };
        assert_eq!(super::validate_health_response(response), Ok(false));
    }

    #[test]
    fn health_response_validation_rejects_missing_and_incompatible_versions() {
        let missing = polaris_proto::pb::HealthResponse {
            protocol_version: None,
            build_version: String::new(),
            ready: true,
            loaded_model_identity_blake3: Vec::new(),
            loaded_model_generation: 0,
            worker_incarnation: vec![0; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        };
        assert!(super::validate_health_response(missing).is_err());

        let incompatible = polaris_proto::pb::HealthResponse {
            protocol_version: Some(polaris_proto::pb::ProtocolVersion { major: 2, minor: 0 }),
            build_version: String::new(),
            ready: true,
            loaded_model_identity_blake3: Vec::new(),
            loaded_model_generation: 0,
            worker_incarnation: vec![0; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        };
        assert!(super::validate_health_response(incompatible).is_err());
    }

    #[tokio::test]
    async fn readiness_does_not_report_ready_when_authenticated_rpc_cannot_connect()
    -> Result<(), Box<dyn std::error::Error>> {
        let client = client(64)?;
        assert!(client.is_ready().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn readiness_authenticates_handshake_and_health_and_reports_admission_only()
    -> Result<(), Box<dyn std::error::Error>> {
        for expected_ready in [true, false] {
            let (client, shutdown, handshakes, health_calls, server) = health_client(
                expected_ready,
                false,
                Some(polaris_proto::PROTOCOL_VERSION.into()),
            )
            .await?;
            assert_eq!(client.is_ready().await?, expected_ready);
            assert_eq!(handshakes.load(Ordering::Relaxed), 1);
            assert_eq!(health_calls.load(Ordering::Relaxed), 1);
            let _ = shutdown.send(());
            server.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn model_readiness_requires_authenticated_loaded_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        for (ready, loaded, expected) in [
            (true, false, false),
            (true, true, true),
            (false, true, false),
        ] {
            let (client, shutdown, _, _, server) =
                health_client(ready, loaded, Some(polaris_proto::PROTOCOL_VERSION.into())).await?;
            assert_eq!(client.is_model_ready().await?, expected);
            let _ = shutdown.send(());
            server.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn readiness_rejects_invalid_health_protocol_versions()
    -> Result<(), Box<dyn std::error::Error>> {
        let invalid_versions = [
            None,
            Some(polaris_proto::pb::ProtocolVersion { major: 2, minor: 0 }),
            Some(polaris_proto::pb::ProtocolVersion {
                major: u32::MAX,
                minor: 0,
            }),
        ];
        for version in invalid_versions {
            let (client, shutdown, _, _, server) = health_client(true, false, version).await?;
            assert!(client.is_ready().await.is_err());
            let _ = shutdown.send(());
            server.await??;
        }
        Ok(())
    }

    #[derive(Clone)]
    struct EvictionService {
        evicted: u32,
        delay: Duration,
        failure: Option<(tonic::Code, String)>,
        handshakes: Arc<AtomicUsize>,
        eviction_calls: Arc<AtomicUsize>,
    }

    #[tonic::async_trait]
    impl polaris_proto::pb::worker_service_server::WorkerService for EvictionService {
        async fn handshake(
            &self,
            _request: tonic::Request<polaris_proto::pb::WorkerHandshakeRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::WorkerHandshakeResponse>, tonic::Status>
        {
            self.handshakes.fetch_add(1, Ordering::Relaxed);
            Ok(tonic::Response::new(
                polaris_proto::pb::WorkerHandshakeResponse {
                    protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
                    kind: i32::from(polaris_proto::pb::WorkerKind::S1),
                    worker_version: "test".to_owned(),
                },
            ))
        }

        async fn decide(
            &self,
            _request: tonic::Request<polaris_proto::pb::DecideRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::DecideResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }

        async fn detect_pii(
            &self,
            _request: tonic::Request<polaris_proto::pb::DetectPiiRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::DetectPiiResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }

        async fn embed(
            &self,
            _request: tonic::Request<polaris_proto::pb::EmbedRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::EmbedResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }

        async fn ingest(
            &self,
            _request: tonic::Request<polaris_proto::pb::IngestRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::IngestResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }

        async fn evict_models(
            &self,
            _request: tonic::Request<polaris_proto::pb::EvictModelsRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::EvictModelsResponse>, tonic::Status>
        {
            self.eviction_calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            if let Some((code, message)) = &self.failure {
                return Err(tonic::Status::new(*code, message.clone()));
            }
            Ok(tonic::Response::new(
                polaris_proto::pb::EvictModelsResponse {
                    evicted: self.evicted,
                    ..Default::default()
                },
            ))
        }

        async fn health(
            &self,
            _request: tonic::Request<polaris_proto::pb::HealthRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::HealthResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }

        async fn shutdown(
            &self,
            _request: tonic::Request<polaris_proto::pb::ShutdownRequest>,
        ) -> Result<tonic::Response<polaris_proto::pb::ShutdownResponse>, tonic::Status> {
            Err(tonic::Status::unimplemented("test eviction service"))
        }
    }

    async fn eviction_client(
        timeout: Duration,
        evicted: u32,
        delay: Duration,
        failure: Option<(tonic::Code, String)>,
    ) -> Result<
        (
            S1Client,
            tokio::sync::oneshot::Sender<()>,
            Arc<AtomicUsize>,
            Arc<AtomicUsize>,
            tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        ),
        Box<dyn std::error::Error>,
    > {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let credential = WorkerCredential::generate()?;
        let authenticator = polaris_proto::worker_auth::WorkerAuthenticator::new(&credential);
        let handshakes = Arc::new(AtomicUsize::new(0));
        let eviction_calls = Arc::new(AtomicUsize::new(0));
        let service = EvictionService {
            evicted,
            delay,
            failure,
            handshakes: Arc::clone(&handshakes),
            eviction_calls: Arc::clone(&eviction_calls),
        };
        let service =
            polaris_proto::pb::worker_service_server::WorkerServiceServer::with_interceptor(
                service,
                authenticator,
            );
        let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_receiver.await;
                })
                .await
        });
        let channel = Endpoint::from_shared(format!("http://{address}"))?
            .connect()
            .await?;
        let client = S1Client::new(channel, ClientConfig::new(timeout, 64)?, credential)?;
        Ok((client, shutdown_sender, handshakes, eviction_calls, server))
    }

    #[tokio::test]
    async fn eviction_authenticates_handshake_and_rpc_and_returns_drop_acknowledgement()
    -> Result<(), Box<dyn std::error::Error>> {
        let (client, shutdown, handshakes, eviction_calls, server) =
            eviction_client(Duration::from_secs(2), 1, Duration::ZERO, None).await?;
        assert_eq!(client.evict_models().await?, 1);
        assert_eq!(handshakes.load(Ordering::Relaxed), 1);
        assert_eq!(eviction_calls.load(Ordering::Relaxed), 1);
        let _ = shutdown.send(());
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn eviction_rejects_acknowledgement_above_two_native_models()
    -> Result<(), Box<dyn std::error::Error>> {
        let (client, shutdown, _, _, server) =
            eviction_client(Duration::from_secs(2), 3, Duration::ZERO, None).await?;
        assert!(matches!(
            client.evict_models().await,
            Err(DecisionError::Model(_))
        ));
        let _ = shutdown.send(());
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn eviction_obeys_configured_deadline() -> Result<(), Box<dyn std::error::Error>> {
        let (client, shutdown, _, _, server) = eviction_client(
            Duration::from_millis(20),
            1,
            Duration::from_millis(100),
            None,
        )
        .await?;
        assert!(matches!(
            client.evict_models().await,
            Err(DecisionError::Timeout)
        ));
        let _ = shutdown.send(());
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn eviction_maps_worker_status_errors() -> Result<(), Box<dyn std::error::Error>> {
        for (code, expected_unavailable) in [
            (tonic::Code::Unavailable, true),
            (tonic::Code::PermissionDenied, false),
        ] {
            let (client, shutdown, _, _, server) = eviction_client(
                Duration::from_secs(2),
                0,
                Duration::ZERO,
                Some((code, "test failure".to_owned())),
            )
            .await?;
            let result = client.evict_models().await;
            if expected_unavailable {
                assert!(matches!(result, Err(DecisionError::Unavailable(_))));
            } else {
                assert!(matches!(result, Err(DecisionError::Model(_))));
            }
            let _ = shutdown.send(());
            server.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn eviction_rejects_missing_authentication() -> Result<(), Box<dyn std::error::Error>> {
        let (mut client, shutdown, handshakes, eviction_calls, server) =
            eviction_client(Duration::from_secs(2), 1, Duration::ZERO, None).await?;
        client.credential = std::sync::Arc::new(WorkerCredential::generate()?);
        if client.handshake.set(()).is_err() {
            return Err("test could not bypass the handshake to exercise EvictModels auth".into());
        }
        assert!(matches!(
            client.evict_models().await,
            Err(DecisionError::Model(_))
        ));
        assert_eq!(handshakes.load(Ordering::Relaxed), 0);
        assert_eq!(eviction_calls.load(Ordering::Relaxed), 0);
        let _ = shutdown.send(());
        server.await??;
        Ok(())
    }
}
