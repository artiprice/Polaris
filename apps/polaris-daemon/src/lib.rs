//! Headless local kernel daemon over the authenticated `polaris-rpc` service.
#![forbid(unsafe_code)]

use futures::Stream;
use polaris_kernel::{Kernel, KernelError, KernelHandle, LocalKernelConfig};
use polaris_rpc::{
    BoundServer, LocalEndpoint, MAX_SESSION_TEXT_BYTES, Readiness, RequestSubject,
    RpcAuthenticator, RpcLimits, RpcService, SessionTextApi, TerminalApi, TextFuture, Transport,
    TransportError, TransportLimits, TrustedFacts, TrustedFactsResolver,
};
use polaris_sdk::contracts::Event;
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_sdk::registry::{CommandReceipt, EventSource, KernelApi};
use polaris_security_capability::{PublicKeyBytes, ResourcePath, Right};
pub use polaris_ui_session::responder::ReplyConfig;
use secrecy::SecretString;
use std::{
    future::Future,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::sync::watch;
use tonic::Status;

/// Daemon resource and drain bounds, validated before opening a listener.
#[derive(Debug, Clone, Copy)]
pub struct DaemonSettings {
    /// Maximum queued event records for one client, in the RPC-supported range 1..=1024.
    pub event_buffer: usize,
    /// Event queue backpressure deadline.
    pub backpressure_timeout: Duration,
    /// Maximum simultaneous accepted connections, in the RPC-supported range 1..=1024.
    pub max_connections: usize,
    /// Maximum HTTP/2 streams per connection, in the RPC-supported range 1..=128.
    pub streams_per_connection: u32,
    /// Grace period before the RPC transport cancels lingering connections, at most 120 seconds.
    pub shutdown_grace: Duration,
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            event_buffer: 128,
            backpressure_timeout: Duration::from_secs(2),
            max_connections: 64,
            streams_per_connection: 16,
            shutdown_grace: Duration::from_secs(10),
        }
    }
}

impl DaemonSettings {
    /// Validate all values before constructing the kernel or binding a socket.
    /// # Errors
    /// Returns an error when a bound is zero or exceeds the RPC transport limits.
    pub fn validate(self) -> Result<Self, DaemonError> {
        if !(1..=1024).contains(&self.event_buffer)
            || self.backpressure_timeout.is_zero()
            || self.backpressure_timeout > Duration::from_secs(30)
            || !(1..=1024).contains(&self.max_connections)
            || !(1..=128).contains(&self.streams_per_connection)
            || self.shutdown_grace.is_zero()
            || self.shutdown_grace > Duration::from_secs(120)
        {
            return Err(DaemonError::Configuration(
                "daemon bounds are outside supported limits",
            ));
        }
        Ok(self)
    }

    fn rpc_limits(self) -> RpcLimits {
        RpcLimits {
            event_buffer: self.event_buffer,
            backpressure_timeout: self.backpressure_timeout,
        }
    }

    fn transport_limits(self) -> TransportLimits {
        TransportLimits {
            max_connections: self.max_connections,
            streams_per_connection: self.streams_per_connection,
            shutdown_grace: self.shutdown_grace,
        }
    }
}

/// Listener and resource settings used when hosting a kernel API.
#[derive(Debug, Clone)]
pub struct DaemonListenerConfig {
    /// Local or explicitly protected TCP endpoint.
    pub endpoint: EndpointSettings,
    /// Bounded RPC and connection settings.
    pub settings: DaemonSettings,
}

/// Explicit endpoint selection. Exactly one local or TCP endpoint must be configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSettings {
    /// Windows named-pipe basename; only local `polaris-*` names are accepted.
    NamedPipe(String),
    /// Unix-domain socket path in an existing private directory.
    Unix(PathBuf),
    /// TCP bind. Public addresses require all three mutual TLS file paths.
    Tcp {
        /// Socket address.
        address: SocketAddr,
        /// Server certificate PEM file.
        certificate: Option<PathBuf>,
        /// Server private key PEM file.
        key: Option<PathBuf>,
        /// Client CA PEM file.
        client_ca: Option<PathBuf>,
    },
}

impl EndpointSettings {
    /// Select and validate one endpoint from command-line values.
    /// # Errors
    /// Rejects multiple/missing endpoints, malformed local paths/names, and public plaintext TCP.
    pub fn select(
        pipe: Option<&str>,
        uds: Option<&str>,
        listen: Option<SocketAddr>,
    ) -> Result<Self, DaemonError> {
        match (pipe, uds, listen) {
            (Some(name), None, None) => {
                LocalEndpoint::named_pipe(name)?;
                Ok(Self::NamedPipe(name.to_owned()))
            }
            (None, Some(path), None) => {
                let path = PathBuf::from(path);
                LocalEndpoint::unix(path.clone())?;
                Ok(Self::Unix(path))
            }
            (None, None, Some(address)) => Self::tcp(address, None, None, None),
            _ => Err(DaemonError::Configuration(
                "select exactly one of --pipe, --uds, or --listen",
            )),
        }
    }

    /// Validate a TCP listener and optional complete mutual TLS file set.
    /// # Errors
    /// Public TCP without TLS, or a partially specified TLS identity.
    pub fn tcp(
        address: SocketAddr,
        certificate: Option<&str>,
        key: Option<&str>,
        client_ca: Option<&str>,
    ) -> Result<Self, DaemonError> {
        let tls_count = [certificate, key, client_ca]
            .into_iter()
            .filter(Option::is_some)
            .count();
        if (tls_count != 0 && tls_count != 3) || (!address.ip().is_loopback() && tls_count != 3) {
            return Err(DaemonError::Configuration(
                "non-loopback TCP requires --tls-cert, --tls-key, and --client-ca together",
            ));
        }
        Ok(Self::Tcp {
            address,
            certificate: certificate.map(PathBuf::from),
            key: key.map(PathBuf::from),
            client_ca: client_ca.map(PathBuf::from),
        })
    }

    fn validate(&self) -> Result<(), DaemonError> {
        match self {
            Self::NamedPipe(name) => {
                let _endpoint = LocalEndpoint::named_pipe(name)?;
                Ok(())
            }
            Self::Unix(path) => {
                let _endpoint = LocalEndpoint::unix(path.clone())?;
                Ok(())
            }
            Self::Tcp {
                address,
                certificate,
                key,
                client_ca,
            } => {
                let cert = certificate.as_deref().and_then(Path::to_str);
                let key_path = key.as_deref().and_then(Path::to_str);
                let ca = client_ca.as_deref().and_then(Path::to_str);
                if certificate.is_some() && cert.is_none()
                    || key.is_some() && key_path.is_none()
                    || client_ca.is_some() && ca.is_none()
                {
                    return Err(DaemonError::Configuration("TLS paths must be valid UTF-8"));
                }
                let _checked = Self::tcp(*address, cert, key_path, ca)?;
                Ok(())
            }
        }
    }

    async fn bind(&self) -> Result<BoundServer, DaemonError> {
        let transport = match self {
            Self::NamedPipe(name) => Transport::Local(LocalEndpoint::named_pipe(name)?),
            Self::Unix(path) => Transport::Local(LocalEndpoint::unix(path.clone())?),
            Self::Tcp {
                address,
                certificate: Some(certificate),
                key: Some(key),
                client_ca: Some(client_ca),
            } => {
                let cert = read_pem(certificate)?;
                let key = read_pem(key)?;
                let ca = read_pem(client_ca)?;
                Transport::Tcp {
                    addr: *address,
                    tls: Some(polars_tls(cert, key, ca)?),
                }
            }
            Self::Tcp {
                address,
                certificate: None,
                key: None,
                client_ca: None,
            } => Transport::Tcp {
                addr: *address,
                tls: None,
            },
            Self::Tcp { .. } => {
                return Err(DaemonError::Configuration("incomplete TLS file set"));
            }
        };
        Ok(BoundServer::bind(transport).await?)
    }
}

/// Production composition inputs. The issuer public key is verification-only; this daemon does
/// not generate, print, or persist a private capability authority.
#[derive(Debug)]
pub struct DaemonConfig {
    /// Kernel state and session identities.
    pub kernel: LocalKernelConfig,
    /// Local or explicitly protected TCP endpoint.
    pub endpoint: EndpointSettings,
    /// Trusted Biscuit issuer public key.
    pub token_root: PublicKeyBytes,
    /// Bounded RPC and connection settings.
    pub settings: DaemonSettings,
}

impl DaemonConfig {
    /// Validate all daemon values before startup.
    /// # Errors
    /// Invalid resource bounds, endpoint policy, workspace path, or identity configuration.
    pub fn validate(self) -> Result<Self, DaemonError> {
        let _settings = self.settings.validate()?;
        self.endpoint.validate()?;
        if !self.kernel.workspace.is_dir() {
            return Err(DaemonError::Configuration(
                "workspace must be an existing directory",
            ));
        }
        if self.kernel.root.as_os_str().is_empty() {
            return Err(DaemonError::Configuration("state directory is required"));
        }
        Ok(self)
    }
}

/// Startup, RPC serving and orderly kernel shutdown errors.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Invalid CLI or resource configuration.
    #[error("invalid daemon configuration: {0}")]
    Configuration(&'static str),
    /// Kernel startup or teardown failure.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// Listener, TLS or graceful transport shutdown failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Capability resource construction failure.
    #[error(transparent)]
    Capability(#[from] polaris_security_capability::CapabilityError),
    /// TLS material read or validation failure.
    #[error("could not read bounded TLS material")]
    TlsMaterial(#[source] std::io::Error),
    /// Operating-system shutdown signal registration failed.
    #[error("could not register daemon shutdown signal")]
    Signal(#[source] std::io::Error),
    /// Tokio runtime construction failed.
    #[error("could not create async runtime")]
    Runtime(#[source] std::io::Error),
    /// Redacted logging pipeline startup failed.
    #[error(transparent)]
    Logging(#[from] polaris_log::LogError),
    /// Redacted log records did not drain before the bounded process exit.
    #[error("daemon log flush did not finish before its deadline")]
    LogFlush,
}

/// A fresh local issuer and one client credential for attached frontends.
///
/// The private issuer key lives only in this process; the credential reads and writes this
/// session's RPC resource (`Read` + `ProposeDiff`) for `ttl`, and nothing else.
/// # Errors
/// Entropy or token encoding failure, or a TTL outside 1 second ..= 7 days.
pub fn issue_local_client_token(
    session: SessionId,
    frame: polaris_sdk::ids::FrameId,
    ttl: Duration,
) -> Result<(PublicKeyBytes, polaris_sdk::contracts::TokenBytes), DaemonError> {
    use polaris_security_capability::{Authority, ResourceScope, Rights, Scope, SystemClock};
    if ttl < Duration::from_secs(1) || ttl > Duration::from_secs(7 * 24 * 3600) {
        return Err(DaemonError::Configuration(
            "client token lifetime must be between 1 second and 7 days",
        ));
    }
    let authority = Authority::new_session()?;
    let scope = Scope {
        resource: ResourceScope::Exact(ResourcePath::new(&format!("sessions/{session}/rpc"))?),
        max_churn_lines: 0,
        ttl,
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read, Right::ProposeDiff]),
    };
    let token = authority.issue_root(frame, &scope, &SystemClock)?;
    Ok((authority.public_key(), token.to_bytes().clone()))
}

/// Lowercase hex of a credential, the form `--token-file` readers expect.
#[must_use]
pub fn credential_hex(token: &polaris_sdk::contracts::TokenBytes) -> String {
    token
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Read a verification-only Ed25519 key from exactly 32 raw bytes or 64 hexadecimal digits.
/// ASCII whitespace around hexadecimal input is accepted; key contents are never included in errors.
/// # Errors
/// File read failure, invalid encoding/length, or invalid Ed25519 public point.
pub fn read_token_public_key(path: &Path) -> Result<PublicKeyBytes, DaemonError> {
    let file = std::fs::File::open(path).map_err(DaemonError::TlsMaterial)?;
    let metadata = file.metadata().map_err(DaemonError::TlsMaterial)?;
    if !metadata.is_file() || metadata.len() > 128 {
        return Err(DaemonError::Configuration(
            "public key file must be a regular file at most 128 bytes",
        ));
    }
    let mut bytes = Vec::with_capacity(128);
    file.take(129)
        .read_to_end(&mut bytes)
        .map_err(DaemonError::TlsMaterial)?;
    if bytes.len() > 128 {
        return Err(DaemonError::Configuration(
            "public key file must be at most 128 bytes",
        ));
    }
    let decoded = match bytes.len() {
        32 => bytes,
        _ => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| DaemonError::Configuration("public key file has invalid encoding"))?
                .trim();
            if text.len() != 64 {
                return Err(DaemonError::Configuration(
                    "public key must be 32 raw bytes or 64 hex digits",
                ));
            }
            let mut decoded = Vec::with_capacity(32);
            for pair in text.as_bytes().chunks(2) {
                let high = hex_nibble(pair.first().copied()).ok_or(DaemonError::Configuration(
                    "public key is not valid hexadecimal",
                ))?;
                let low = hex_nibble(pair.get(1).copied()).ok_or(DaemonError::Configuration(
                    "public key is not valid hexadecimal",
                ))?;
                decoded.push((high << 4) | low);
            }
            decoded
        }
    };
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| DaemonError::Configuration("public key must contain 32 bytes"))?;
    Ok(PublicKeyBytes::from_bytes(key)?)
}

fn hex_nibble(value: Option<u8>) -> Option<u8> {
    match value? {
        b'0'..=b'9' => value.map(|byte| byte - b'0'),
        b'a'..=b'f' => value.map(|byte| byte - b'a' + 10),
        b'A'..=b'F' => value.map(|byte| byte - b'A' + 10),
        _ => None,
    }
}

/// Run one real kernel instance until the provided shutdown future resolves, then drain RPC and
/// invoke the kernel's ordered teardown path.
/// # Errors
/// Kernel startup, listener/service setup, RPC serving or kernel shutdown failure.
pub async fn serve_daemon<F>(config: DaemonConfig, shutdown: F) -> Result<(), DaemonError>
where
    F: Future<Output = ()> + Send,
{
    serve_daemon_with_replies(config, None, shutdown).await
}

/// [`serve_daemon`], and with `replies` the daemon also answers every message typed into its
/// session with a read-only model turn ([`polaris_ui_session::responder`]); attached frontends
/// read the reply back as the step's record. Reply progress and failures go to standard error.
/// # Errors
/// Kernel startup, listener/service setup, RPC serving or kernel shutdown failure. A reply
/// failure (no provider, a failed turn) never stops the daemon.
pub async fn serve_daemon_with_replies<F>(
    config: DaemonConfig,
    replies: Option<ReplyConfig>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()> + Send,
{
    let config = config.validate()?;
    let settings = config.settings.validate()?;
    let session = config.kernel.session;
    let workspace = config.kernel.workspace.clone();
    let kernel = Kernel::start(config.kernel).await?;
    let readiness = DaemonReadiness::new();
    let resolver = DaemonResolver::new(session)?;
    let shutdown_handle = kernel.clone();
    let text: Arc<dyn SessionTextApi> = Arc::new(DaemonText::new(kernel.clone(), session));
    let stop_replies = tokio_util::sync::CancellationToken::new();
    let responder = replies.map(|reply| {
        let kernel = kernel.clone();
        let stop = stop_replies.clone();
        tokio::spawn(async move {
            let answered = polaris_ui_session::responder::answer_session_input(
                kernel,
                session,
                workspace,
                reply,
                stop,
                |line| report(&line),
            )
            .await;
            if let Err(error) = answered {
                report(&format!("model replies are off: {error}"));
            }
        })
    });
    readiness.set(true);
    let shutdown = close_readiness_when_signaled(shutdown, readiness.clone());
    let served = serve_kernel_api(
        kernel,
        Some(text),
        UnavailableTerminal,
        RpcAuthenticator::new(config.token_root, resolver),
        readiness.clone(),
        DaemonListenerConfig {
            endpoint: config.endpoint,
            settings,
        },
        shutdown,
        move || async move {
            // A running reply is cancelled and settles before the kernel stops.
            stop_replies.cancel();
            if let Some(responder) = responder {
                let _settled = tokio::time::timeout(Duration::from_secs(30), responder).await;
            }
            let _report = shutdown_handle.shutdown().await?;
            Ok(())
        },
    )
    .await;
    readiness.set(false);
    served
}

/// One line on standard error; a closed stream drops it.
fn report(line: &str) {
    use std::io::Write as _;
    let _written = writeln!(std::io::stderr().lock(), "polarisd: {line}");
}

/// Host an injected real SDK kernel API and authentication policy. This is the embedding seam for
/// future kernel profiles; the local CLI profile uses [`serve_daemon`].
/// # Errors
/// Service construction, listener/transport, shutdown callback, or kernel teardown failure.
#[allow(clippy::too_many_arguments)]
pub async fn serve_kernel_api<K, T, A, H, F, S, SH>(
    kernel: K,
    text: Option<Arc<dyn SessionTextApi>>,
    terminal: T,
    authenticator: RpcAuthenticator<A>,
    readiness: H,
    listener: DaemonListenerConfig,
    shutdown: F,
    shutdown_kernel: SH,
) -> Result<(), DaemonError>
where
    K: KernelApi + 'static,
    K::Events: EventSource + 'static,
    T: TerminalApi,
    A: TrustedFactsResolver,
    H: Readiness,
    F: Future<Output = ()> + Send,
    S: Future<Output = Result<(), DaemonError>> + Send,
    SH: FnOnce() -> S,
{
    let served: Result<(), DaemonError> = async {
        let settings = listener.settings.validate()?;
        let service = RpcService::new(
            kernel,
            terminal,
            authenticator,
            readiness,
            settings.rpc_limits(),
        )
        .map_err(|_| DaemonError::Configuration("invalid RPC service limits"))?;
        let service = match text {
            Some(text) => service.with_session_text(text),
            None => service,
        };
        let server = listener.endpoint.bind().await?;
        server
            .serve(service, settings.transport_limits(), shutdown)
            .await?;
        Ok(())
    }
    .await;
    let stopped = shutdown_kernel().await;
    served?;
    stopped
}

#[derive(Clone)]
struct DaemonReadiness(Arc<AtomicBool>);
impl DaemonReadiness {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    fn set(&self, ready: bool) {
        self.0.store(ready, Ordering::Release);
    }
}
impl Readiness for DaemonReadiness {
    fn is_ready(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

async fn close_readiness_when_signaled<F>(shutdown: F, readiness: DaemonReadiness)
where
    F: Future<Output = ()> + Send,
{
    shutdown.await;
    readiness.set(false);
}

#[cfg(test)]
#[path = "../../../tests/apps/polaris-daemon/src/lib.rs"]
mod tests;

struct DaemonResolver {
    session: SessionId,
    resource: ResourcePath,
}
impl DaemonResolver {
    fn new(session: SessionId) -> Result<Self, DaemonError> {
        let resource = ResourcePath::new(&format!("sessions/{session}/rpc"))?;
        Ok(Self { session, resource })
    }
}
impl DaemonResolver {
    fn check_session(&self, session: Option<&polaris_proto::pb::Uuid>) -> Result<(), Status> {
        let Some(session) = session else {
            return Err(Status::invalid_argument("session is required"));
        };
        let requested = (u128::from(session.hi) << 64) | u128::from(session.lo);
        if requested != self.session.as_uuid().as_u128() {
            return Err(Status::permission_denied("session is outside daemon scope"));
        }
        Ok(())
    }
}
impl TrustedFactsResolver for DaemonResolver {
    fn resolve(&self, _right: Right, subject: RequestSubject<'_>) -> Result<TrustedFacts, Status> {
        match subject {
            RequestSubject::Health | RequestSubject::AttachTerminal => {}
            RequestSubject::Subscribe(request) => {
                let Some(session) = &request.session else {
                    return Err(Status::invalid_argument("session is required"));
                };
                let requested = (u128::from(session.hi) << 64) | u128::from(session.lo);
                if requested != self.session.as_uuid().as_u128() {
                    return Err(Status::permission_denied("session is outside daemon scope"));
                }
            }
            RequestSubject::SubmitCommand(_) => {}
            RequestSubject::SubmitText(request) => self.check_session(request.session.as_ref())?,
            RequestSubject::ReadText(request) => self.check_session(request.session.as_ref())?,
        }
        Ok(TrustedFacts {
            resource: self.resource.clone(),
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
        })
    }
}

/// Attached-frontend text over the daemon's own kernel: typed lines go through the kernel's
/// control authority (the client never mints commands), and only text that a directive or
/// user-input event of this session references can be read back. For a step transcript a
/// `FrameStepRecorded` event of this session references, only the model's reply is served,
/// never the prompt or the tool results it carries.
pub struct DaemonText {
    kernel: KernelHandle,
    session: SessionId,
    /// Digests referenced by this session's events, kept current by a subscription.
    known: Arc<std::sync::Mutex<std::collections::BTreeMap<Cid, TextKind>>>,
}

/// What a referenced digest holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKind {
    /// Directive or typed user-input text, served verbatim.
    Text,
    /// A frame-step transcript; only its reply is served.
    Step,
}

impl DaemonText {
    /// Serve text for `session` of `kernel`. Must be called inside a tokio runtime.
    #[must_use]
    pub fn new(kernel: KernelHandle, session: SessionId) -> Self {
        let known = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let index = Arc::clone(&known);
        let mut events = kernel.context().subscribe();
        tokio::spawn(async move {
            while let Ok(record) = events.next().await {
                if record.session() == session
                    && let Some((cid, kind)) = text_reference(record.event())
                    && let Ok(mut known) = index.lock()
                {
                    known.insert(cid, kind);
                }
            }
        });
        Self {
            kernel,
            session,
            known,
        }
    }

    fn scope(&self, session: SessionId) -> Result<(), Status> {
        if session == self.session {
            Ok(())
        } else {
            Err(Status::permission_denied("session is outside daemon scope"))
        }
    }

    fn kind_of(&self, cid: &Cid) -> Option<TextKind> {
        self.known
            .lock()
            .ok()
            .and_then(|known| known.get(cid).copied())
    }

    /// Rebuild the index from the durable log (events committed before the subscription).
    async fn reindex(&self) -> Result<(), Status> {
        let context = Arc::clone(self.kernel.context());
        let session = self.session;
        let found = tokio::task::spawn_blocking(move || {
            context
                .records(Seq::new(1), Seq::new(u64::MAX))
                .map(|records| {
                    records
                        .iter()
                        .filter(|record| record.session() == session)
                        .filter_map(|record| text_reference(record.event()))
                        .collect::<Vec<_>>()
                })
        })
        .await
        .map_err(|_| Status::internal("text index task failed"))?
        .map_err(|_| Status::unavailable("event log unavailable"))?;
        if let Ok(mut known) = self.known.lock() {
            known.extend(found);
        }
        Ok(())
    }
}

fn text_reference(event: &Event) -> Option<(Cid, TextKind)> {
    use polaris_sdk::contracts::FrameStepOutcome;
    match event {
        Event::DirectiveApplied { directive } => Some((*directive, TextKind::Text)),
        Event::UserInputSubmitted { text } => Some((*text, TextKind::Text)),
        Event::FrameStepRecorded {
            transcript,
            outcome: FrameStepOutcome::Accepted | FrameStepOutcome::ClaimedDone,
            ..
        } => Some((*transcript, TextKind::Step)),
        _ => None,
    }
}

/// The model's reply in a stored step transcript: the final text of a step that answered, cut
/// to the RPC text bound on a character boundary. `None` for a tool round or an aborted step.
fn step_reply(bytes: &[u8]) -> Option<String> {
    use polaris_runtime::frames::{FrameStepRecord, StepOutcome};
    let record: FrameStepRecord = serde_json::from_slice(bytes).ok()?;
    let mut text = match record.result? {
        StepOutcome::Continue { text } => text,
        StepOutcome::ClaimedDone { claim } => claim,
        StepOutcome::ToolResults { .. } | StepOutcome::Aborted => return None,
    };
    if text.trim().is_empty() {
        return None;
    }
    if text.len() > MAX_SESSION_TEXT_BYTES {
        let mut end = MAX_SESSION_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Some(text)
}

fn text_status(error: &polaris_sdk::registry::KernelError) -> Status {
    use polaris_sdk::registry::KernelError as ApiError;
    match error {
        ApiError::Rejected(reason) => Status::invalid_argument(reason.clone()),
        ApiError::Busy => Status::unavailable("kernel is busy; try again"),
        ApiError::Closed => Status::unavailable("kernel is closed"),
        ApiError::Internal(_) => Status::internal("kernel could not record the text"),
    }
}

impl SessionTextApi for DaemonText {
    fn submit_text(
        &self,
        session: SessionId,
        text: String,
        directive: bool,
    ) -> TextFuture<'_, CommandReceipt> {
        Box::pin(async move {
            self.scope(session)?;
            if text.len() > MAX_SESSION_TEXT_BYTES || text.trim().is_empty() {
                return Err(Status::invalid_argument("text is empty or exceeds 64 KiB"));
            }
            let receipt = if directive {
                self.kernel.apply_directive(&text).await
            } else {
                self.kernel.submit_user_input(&text).await
            };
            receipt.map_err(|error| text_status(&error))
        })
    }

    fn read_text(&self, session: SessionId, cid: Cid) -> TextFuture<'_, String> {
        Box::pin(async move {
            use polaris_kernel::verifier::EvidenceStore;
            self.scope(session)?;
            if self.kind_of(&cid).is_none() {
                self.reindex().await?;
            }
            let Some(kind) = self.kind_of(&cid) else {
                return Err(Status::not_found(
                    "no text event of this session references that digest",
                ));
            };
            let store = polaris_kernel::KernelEvidence::new(Arc::clone(self.kernel.context()));
            let bytes = store
                .get(&cid)
                .map_err(|_| Status::unavailable("content store unavailable"))?
                .ok_or_else(|| Status::not_found("text is not in the content store"))?;
            if kind == TextKind::Step {
                return step_reply(&bytes)
                    .ok_or_else(|| Status::not_found("this step has no model reply"));
            }
            if bytes.len() > MAX_SESSION_TEXT_BYTES {
                return Err(Status::resource_exhausted("text exceeds 64 KiB"));
            }
            String::from_utf8(bytes).map_err(|_| Status::data_loss("stored text is not UTF-8"))
        })
    }
}

/// Explicit terminal unavailability until the kernel owns a real PTY backend.
/// This implementation never fabricates a terminal stream or executes a host shell.
#[derive(Debug, Clone, Copy)]
struct UnavailableTerminal;
impl TerminalApi for UnavailableTerminal {
    type Stream = futures::stream::Empty<Result<polaris_proto::pb::TerminalServerFrame, Status>>;

    async fn attach(
        &self,
        _inbound: Pin<
            Box<dyn Stream<Item = Result<polaris_proto::pb::TerminalClientFrame, Status>> + Send>,
        >,
        _cancellation: watch::Receiver<bool>,
    ) -> Result<Self::Stream, Status> {
        Err(Status::failed_precondition(
            "no kernel terminal backend is configured",
        ))
    }
}

fn read_pem(path: &Path) -> Result<Vec<u8>, DaemonError> {
    let file = std::fs::File::open(path).map_err(DaemonError::TlsMaterial)?;
    let metadata = file.metadata().map_err(DaemonError::TlsMaterial)?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return Err(DaemonError::Configuration(
            "TLS material must be a regular file at most 256 KiB",
        ));
    }
    let mut bytes = Vec::with_capacity(256 * 1024);
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(DaemonError::TlsMaterial)?;
    if bytes.is_empty() || bytes.len() > 256 * 1024 {
        return Err(DaemonError::Configuration(
            "TLS material must be nonempty and at most 256 KiB",
        ));
    }
    Ok(bytes)
}

fn polars_tls(
    certificate: Vec<u8>,
    key: Vec<u8>,
    client_ca: Vec<u8>,
) -> Result<polaris_rpc::TlsServerConfig, DaemonError> {
    let key = String::from_utf8(key)
        .map_err(|_| DaemonError::Configuration("TLS key PEM must be UTF-8"))?;
    Ok(polaris_rpc::TlsServerConfig::new(
        certificate,
        SecretString::from(key),
        client_ca,
    )?)
}
