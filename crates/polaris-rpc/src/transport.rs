//! Owned, bounded pipe/UDS/TCP listeners for the authenticated kernel service.
use crate::{Readiness, RpcService, TerminalApi, TrustedFactsResolver};
use futures::Stream;
use polaris_sdk::registry::KernelApi;
use secrecy::{ExposeSecret, SecretString};
use std::{
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig, server::Connected};

const MAX_PEM_BYTES: usize = 256 * 1024;

/// Transport/configuration failures. No credential or PEM contents are included.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Invalid endpoint, bound or certificate configuration.
    #[error("invalid RPC transport configuration: {0}")]
    Configuration(&'static str),
    /// Public TCP listeners require client-authenticated TLS.
    #[error("non-loopback RPC listeners require mutual TLS")]
    TlsRequired,
    /// The requested transport is unsupported on this operating system.
    #[error("RPC transport is unavailable on this platform")]
    Unsupported,
    /// Listener/connector/socket cleanup failed.
    #[error("RPC transport I/O failed")]
    Io(#[from] io::Error),
    /// Tonic could not construct or run the transport.
    #[error("RPC transport failed")]
    Tonic(#[from] tonic::transport::Error),
    /// Connections failed to close after the graceful and forced deadlines.
    #[error("RPC shutdown deadline exceeded")]
    ShutdownDeadline,
}

/// Bounded PEM material. The private key has no printable/debug representation.
pub struct TlsServerConfig {
    certificate: Vec<u8>,
    key: SecretString,
    client_ca: Vec<u8>,
}
impl TlsServerConfig {
    /// Configure one server identity and the mandatory client-authentication trust anchor.
    /// # Errors
    /// Rejects empty or oversized material; certificate/key validity is checked by tonic on bind.
    pub fn new(
        certificate: Vec<u8>,
        key: SecretString,
        client_ca: Vec<u8>,
    ) -> Result<Self, TransportError> {
        for bytes in [
            certificate.as_slice(),
            key.expose_secret().as_bytes(),
            client_ca.as_slice(),
        ] {
            if bytes.is_empty() || bytes.len() > MAX_PEM_BYTES {
                return Err(TransportError::Configuration(
                    "empty or oversized TLS material",
                ));
            }
        }
        Ok(Self {
            certificate,
            key,
            client_ca,
        })
    }
    fn into_tonic(self) -> ServerTlsConfig {
        ServerTlsConfig::new()
            .identity(Identity::from_pem(
                self.certificate,
                self.key.expose_secret().as_bytes(),
            ))
            .client_ca_root(Certificate::from_pem(self.client_ca))
            .client_auth_optional(false)
    }
}

/// Validated local endpoint; constructors never accept a remote UNC pipe name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEndpoint {
    kind: LocalKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalKind {
    Pipe(Box<str>),
    Unix(PathBuf),
}
impl LocalEndpoint {
    /// A local `polaris-*` Windows pipe basename, without any path separators.
    /// # Errors
    /// Rejects empty, oversized, remote, reserved-prefix or ambiguous names.
    pub fn named_pipe(name: &str) -> Result<Self, TransportError> {
        if !name.starts_with("polaris-")
            || name.len() > 128
            || name.len() <= 8
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(TransportError::Configuration("invalid local pipe basename"));
        }
        Ok(Self {
            kind: LocalKind::Pipe(format!(r"\\.\pipe\{name}").into()),
        })
    }
    /// An absolute Unix socket path in an existing private parent directory.
    /// # Errors
    /// Rejects relative/dot/traversal paths or a path too long for portable UDS implementations.
    pub fn unix(path: PathBuf) -> Result<Self, TransportError> {
        use std::path::Component;
        if !path.is_absolute()
            || path.as_os_str().len() > 100
            || path.file_name().is_none()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(TransportError::Configuration("invalid Unix socket path"));
        }
        Ok(Self {
            kind: LocalKind::Unix(path),
        })
    }
    #[cfg(windows)]
    pub(crate) fn pipe_path(&self) -> Result<&str, TransportError> {
        match &self.kind {
            LocalKind::Pipe(name) => Ok(name),
            LocalKind::Unix(_) => Err(TransportError::Unsupported),
        }
    }
    #[cfg(unix)]
    pub(crate) fn unix_path(&self) -> Result<&std::path::Path, TransportError> {
        match &self.kind {
            LocalKind::Unix(path) => Ok(path),
            LocalKind::Pipe(_) => Err(TransportError::Unsupported),
        }
    }
}

/// Listener selection. Every listener is served only with an authenticated [`RpcService`].
pub enum Transport {
    /// Named pipe on Windows or UDS on Unix.
    Local(LocalEndpoint),
    /// Plain TCP is permitted only on a loopback address.
    Tcp {
        addr: SocketAddr,
        tls: Option<TlsServerConfig>,
    },
}
/// Public address of an already bound listener, including its actual ephemeral TCP port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundAddress {
    /// Actual TCP address.
    Tcp(SocketAddr),
    /// Actual local endpoint.
    Local(LocalEndpoint),
}
/// Connection and shutdown bounds. Allocation happens per connection, never per I/O poll.
#[derive(Debug, Clone, Copy)]
pub struct TransportLimits {
    /// Simultaneously accepted connections, 1..=1024.
    pub max_connections: usize,
    /// Concurrent HTTP/2 streams per connection, 1..=128.
    pub streams_per_connection: u32,
    /// Grace allowed for existing requests before their I/O is cancelled.
    pub shutdown_grace: Duration,
}
impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            streams_per_connection: 16,
            shutdown_grace: Duration::from_secs(10),
        }
    }
}
impl TransportLimits {
    fn validate(self) -> Result<Self, TransportError> {
        if !(1..=1024).contains(&self.max_connections)
            || !(1..=128).contains(&self.streams_per_connection)
            || self.shutdown_grace.is_zero()
            || self.shutdown_grace > Duration::from_secs(120)
        {
            return Err(TransportError::Configuration(
                "invalid connection or shutdown limits",
            ));
        }
        Ok(self)
    }
}

enum BoundKind {
    Tcp(TcpListener, Option<TlsServerConfig>),
    #[cfg(windows)]
    Pipe(tokio::net::windows::named_pipe::NamedPipeServer, Box<str>),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener, UnixSocketGuard),
}
/// An owned listening endpoint. Duplicate ownership is rejected by the operating system.
pub struct BoundServer {
    kind: BoundKind,
    address: BoundAddress,
}
impl fmt::Debug for BoundServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundServer")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}
impl BoundServer {
    /// Bind without replacing an existing pipe/socket or downgrading a TLS configuration.
    /// # Errors
    /// Invalid/private-directory/TLS policy, duplicate owner or actual bind failure.
    pub async fn bind(transport: Transport) -> Result<Self, TransportError> {
        match transport {
            Transport::Tcp { addr, tls } => {
                if !addr.ip().is_loopback() && tls.is_none() {
                    return Err(TransportError::TlsRequired);
                }
                // Parse TLS before opening the listener; bad credentials never expose a socket.
                if let Some(config) = &tls {
                    let _validated = Server::builder().tls_config(
                        ServerTlsConfig::new()
                            .identity(Identity::from_pem(
                                &config.certificate,
                                config.key.expose_secret().as_bytes(),
                            ))
                            .client_ca_root(Certificate::from_pem(&config.client_ca))
                            .client_auth_optional(false),
                    )?;
                }
                let listener = TcpListener::bind(addr).await?;
                let address = BoundAddress::Tcp(listener.local_addr()?);
                Ok(Self {
                    kind: BoundKind::Tcp(listener, tls),
                    address,
                })
            }
            Transport::Local(endpoint) => Self::bind_local(endpoint),
        }
    }
    /// The bound address; a TCP port of zero has already been resolved by the OS.
    #[must_use]
    pub fn address(&self) -> &BoundAddress {
        &self.address
    }

    #[cfg(windows)]
    fn bind_local(endpoint: LocalEndpoint) -> Result<Self, TransportError> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = endpoint.pipe_path()?.to_owned().into_boxed_str();
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(name.as_ref())?;
        Ok(Self {
            kind: BoundKind::Pipe(first, name),
            address: BoundAddress::Local(endpoint),
        })
    }
    #[cfg(unix)]
    fn bind_local(endpoint: LocalEndpoint) -> Result<Self, TransportError> {
        use std::os::unix::fs::PermissionsExt;
        let path = endpoint.unix_path()?;
        let parent = path
            .parent()
            .ok_or(TransportError::Configuration("socket has no parent"))?;
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(TransportError::Configuration(
                "Unix socket parent must be a private directory",
            ));
        }
        let listener = tokio::net::UnixListener::bind(path)?;
        let guard = UnixSocketGuard::new(path.to_owned())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            kind: BoundKind::Unix(listener, guard),
            address: BoundAddress::Local(endpoint),
        })
    }
    #[cfg(not(any(unix, windows)))]
    fn bind_local(_endpoint: LocalEndpoint) -> Result<Self, TransportError> {
        Err(TransportError::Unsupported)
    }

    /// Serve the authenticated service, stop admission on shutdown, then cancel lingering I/O.
    /// # Errors
    /// Invalid bounds, actual listener/TLS failure, cleanup failure or bounded shutdown failure.
    pub async fn serve<K, T, A, H, F>(
        self,
        service: RpcService<K, T, A, H>,
        limits: TransportLimits,
        shutdown: F,
    ) -> Result<(), TransportError>
    where
        K: KernelApi + 'static,
        K::Events: 'static,
        T: TerminalApi,
        A: TrustedFactsResolver,
        H: Readiness,
        F: Future<Output = ()> + Send,
    {
        let limits = limits.validate()?;
        let connections = CancellationToken::new();
        let _connection_guard = connections.clone().drop_guard();
        let graceful = CancellationToken::new();
        let permits = Arc::new(Semaphore::new(limits.max_connections));
        let max_headers = u32::try_from(crate::MAX_METADATA_BYTES)
            .map_err(|_| TransportError::Configuration("metadata bound exceeds HTTP/2 limit"))?;
        let mut server = Server::builder()
            .max_concurrent_streams(Some(limits.streams_per_connection))
            .http2_max_header_list_size(Some(max_headers))
            .http2_keepalive_interval(Some(Duration::from_secs(15)));
        match self.kind {
            BoundKind::Tcp(listener, tls) => {
                let mut server = match tls {
                    Some(config) => server.tls_config(config.into_tonic())?,
                    None => server,
                };
                let incoming = tcp_incoming(listener, permits, connections.clone());
                let serving = server
                    .add_service(service.into_grpc_service())
                    .serve_with_incoming_shutdown(incoming, graceful.clone().cancelled_owned());
                run_until_shutdown(
                    serving,
                    shutdown,
                    graceful,
                    connections,
                    limits.shutdown_grace,
                )
                .await
            }
            #[cfg(windows)]
            BoundKind::Pipe(first, name) => {
                let incoming = pipe_incoming(first, name, permits, connections.clone());
                let serving = server
                    .add_service(service.into_grpc_service())
                    .serve_with_incoming_shutdown(incoming, graceful.clone().cancelled_owned());
                run_until_shutdown(
                    serving,
                    shutdown,
                    graceful,
                    connections,
                    limits.shutdown_grace,
                )
                .await
            }
            #[cfg(unix)]
            BoundKind::Unix(listener, mut guard) => {
                let incoming = unix_incoming(listener, permits, connections.clone());
                let serving = server
                    .add_service(service.into_grpc_service())
                    .serve_with_incoming_shutdown(incoming, graceful.clone().cancelled_owned());
                let result = run_until_shutdown(
                    serving,
                    shutdown,
                    graceful,
                    connections,
                    limits.shutdown_grace,
                )
                .await;
                guard.cleanup()?;
                result
            }
        }
    }
}

async fn run_until_shutdown<S, F>(
    serving: S,
    shutdown: F,
    graceful: CancellationToken,
    connections: CancellationToken,
    grace: Duration,
) -> Result<(), TransportError>
where
    S: Future<Output = Result<(), tonic::transport::Error>>,
    F: Future<Output = ()>,
{
    tokio::pin!(serving, shutdown);
    tokio::select! {
        result = &mut serving => Ok(result?),
        () = &mut shutdown => {
            graceful.cancel();
            match tokio::time::timeout(grace, &mut serving).await {
                Ok(result) => Ok(result?),
                Err(_) => {
                    connections.cancel();
                    match tokio::time::timeout(Duration::from_secs(2), &mut serving).await {
                        Ok(result) => Ok(result?),
                        Err(_) => Err(TransportError::ShutdownDeadline),
                    }
                }
            }
        }
    }
}

struct OwnedIo<I> {
    inner: I,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    _permit: OwnedSemaphorePermit,
}
impl<I> OwnedIo<I> {
    fn new(inner: I, permit: OwnedSemaphorePermit, cancellation: CancellationToken) -> Self {
        Self {
            inner,
            cancelled: Box::pin(cancellation.cancelled_owned()),
            _permit: permit,
        }
    }
    fn is_cancelled(&mut self, cx: &mut Context<'_>) -> bool {
        self.cancelled.as_mut().poll(cx).is_ready()
    }
}
impl<I: AsyncRead + Unpin> AsyncRead for OwnedIo<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buffer)
    }
}
impl<I: AsyncWrite + Unpin> AsyncWrite for OwnedIo<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::ConnectionAborted)));
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.is_cancelled(cx) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
impl<I: Connected> Connected for OwnedIo<I> {
    type ConnectInfo = I::ConnectInfo;
    fn connect_info(&self) -> Self::ConnectInfo {
        self.inner.connect_info()
    }
}

fn tcp_incoming(
    listener: TcpListener,
    permits: Arc<Semaphore>,
    cancellation: CancellationToken,
) -> impl Stream<Item = io::Result<OwnedIo<TcpStream>>> {
    futures::stream::unfold(
        (listener, permits, cancellation),
        |(listener, permits, cancellation)| async move {
            let permit = match permits.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return None,
            };
            let accepted = listener.accept().await.and_then(|(socket, _)| {
                socket.set_nodelay(true)?;
                Ok(OwnedIo::new(socket, permit, cancellation.clone()))
            });
            Some((accepted, (listener, permits, cancellation)))
        },
    )
}

#[cfg(windows)]
struct PipeIo(tokio::net::windows::named_pipe::NamedPipeServer);
#[cfg(windows)]
impl Connected for PipeIo {
    type ConnectInfo = ();
    fn connect_info(&self) {}
}
#[cfg(windows)]
impl AsyncRead for PipeIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buffer)
    }
}
#[cfg(windows)]
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
#[cfg(windows)]
fn pipe_incoming(
    first: tokio::net::windows::named_pipe::NamedPipeServer,
    name: Box<str>,
    permits: Arc<Semaphore>,
    cancellation: CancellationToken,
) -> impl Stream<Item = io::Result<OwnedIo<PipeIo>>> {
    use tokio::net::windows::named_pipe::ServerOptions;
    futures::stream::unfold(
        (Some(first), name, permits, cancellation),
        |(first, name, permits, cancellation)| async move {
            let first = first?;
            let permit = match permits.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return None,
            };
            if let Err(error) = first.connect().await {
                return Some((Err(error), (None, name, permits, cancellation)));
            }
            // Never expose a connected instance until its successor exists, avoiding a name gap.
            let next = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(name.as_ref())
            {
                Ok(next) => next,
                Err(error) => return Some((Err(error), (None, name, permits, cancellation))),
            };
            Some((
                Ok(OwnedIo::new(PipeIo(first), permit, cancellation.clone())),
                (Some(next), name, permits, cancellation),
            ))
        },
    )
}

#[cfg(unix)]
fn unix_incoming(
    listener: tokio::net::UnixListener,
    permits: Arc<Semaphore>,
    cancellation: CancellationToken,
) -> impl Stream<Item = io::Result<OwnedIo<tokio::net::UnixStream>>> {
    futures::stream::unfold(
        (listener, permits, cancellation),
        |(listener, permits, cancellation)| async move {
            let permit = match permits.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return None,
            };
            let accepted = listener
                .accept()
                .await
                .map(|(socket, _)| OwnedIo::new(socket, permit, cancellation.clone()));
            Some((accepted, (listener, permits, cancellation)))
        },
    )
}
#[cfg(unix)]
struct UnixSocketGuard {
    path: PathBuf,
    identity: (u64, u64),
    cleaned: bool,
}
#[cfg(unix)]
impl UnixSocketGuard {
    fn new(path: PathBuf) -> Result<Self, TransportError> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
            cleaned: false,
        })
    }
    fn cleanup(&mut self) -> Result<(), TransportError> {
        use std::os::unix::fs::MetadataExt;
        if self.cleaned {
            return Ok(());
        }
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        if (metadata.dev(), metadata.ino()) != self.identity {
            return Err(TransportError::Configuration(
                "socket path ownership changed",
            ));
        }
        std::fs::remove_file(&self.path)?;
        self.cleaned = true;
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for UnixSocketGuard {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            tracing::warn!(error=%error,"RPC socket cleanup failed; explicit serve cleanup remains required");
        }
    }
}
