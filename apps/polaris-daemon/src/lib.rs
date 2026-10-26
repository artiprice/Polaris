//! Headless local kernel daemon over the authenticated `polaris-rpc` service.
#![forbid(unsafe_code)]

use futures::Stream;
use polaris_kernel::{Kernel, KernelError, LocalKernelConfig};
use polaris_rpc::{
    BoundServer, LocalEndpoint, Readiness, RequestSubject, RpcAuthenticator, RpcLimits, RpcService,
    TerminalApi, Transport, TransportError, TransportLimits, TrustedFacts, TrustedFactsResolver,
};
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::{EventSource, KernelApi};
use polaris_security_capability::{PublicKeyBytes, ResourcePath, Right};
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
    let config = config.validate()?;
    let settings = config.settings.validate()?;
    let session = config.kernel.session;
    let kernel = Kernel::start(config.kernel).await?;
    let readiness = DaemonReadiness::new();
    let resolver = DaemonResolver::new(session)?;
    let shutdown_handle = kernel.clone();
    readiness.set(true);
    let shutdown = close_readiness_when_signaled(shutdown, readiness.clone());
    let served = serve_kernel_api(
        kernel,
        UnavailableTerminal,
        RpcAuthenticator::new(config.token_root, resolver),
        readiness.clone(),
        DaemonListenerConfig {
            endpoint: config.endpoint,
            settings,
        },
        shutdown,
        move || async move {
            let _report = shutdown_handle.shutdown().await?;
            Ok(())
        },
    )
    .await;
    readiness.set(false);
    served
}

/// Host an injected real SDK kernel API and authentication policy. This is the embedding seam for
/// future kernel profiles; the local CLI profile uses [`serve_daemon`].
/// # Errors
/// Service construction, listener/transport, shutdown callback, or kernel teardown failure.
pub async fn serve_kernel_api<K, T, A, H, F, S, SH>(
    kernel: K,
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
mod tests {
    use super::{DaemonReadiness, Readiness, close_readiness_when_signaled};

    #[tokio::test]
    async fn readiness_closes_when_shutdown_is_requested() -> Result<(), &'static str> {
        let readiness = DaemonReadiness::new();
        readiness.set(true);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let shutdown = close_readiness_when_signaled(
            async move {
                let _received = stop_rx.await;
            },
            readiness.clone(),
        );
        let serving = tokio::spawn(shutdown);
        if !readiness.is_ready() {
            return Err("readiness must remain true while the daemon serves");
        }
        stop_tx
            .send(())
            .map_err(|_| "shutdown listener disappeared")?;
        serving
            .await
            .map_err(|_| "readiness shutdown task failed")?;
        if readiness.is_ready() {
            return Err("readiness must close before transport draining");
        }
        Ok(())
    }
}

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
