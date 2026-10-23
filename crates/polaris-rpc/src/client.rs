//! Credential-bound clients for the published kernel RPC contract.
use crate::{LocalEndpoint, TransportError};
use futures::Stream;
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, limits, negotiate, pb};
use polaris_sdk::{
    contracts::{Command, EventRecord, TokenBytes, Unverified},
    ids::{CommandId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi, KernelError},
};
use prost::Message;
use secrecy::{ExposeSecret, SecretString};
use std::{fmt, net::SocketAddr, time::Duration};
use tonic::{
    Request, Status,
    metadata::{Ascii, MetadataValue},
    transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity},
};

type GeneratedClient = pb::polaris_kernel_service_client::PolarisKernelServiceClient<Channel>;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PEM_BYTES: usize = 256 * 1024;

/// Client transport errors, with no credential values in locally constructed messages.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Endpoint or connection failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The authenticated peer rejected the request or closed its stream.
    #[error("RPC peer rejected the operation: {0}")]
    Status(Box<Status>),
    /// A response violates the published protocol contract.
    #[error("RPC protocol violation: {0}")]
    Protocol(&'static str),
}
impl From<Status> for ClientError {
    fn from(status: Status) -> Self {
        Self::Status(Box::new(status))
    }
}

/// Retained terminal reason for a checked event stream.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RpcStreamError {
    /// The peer or transport ended the stream with an error status.
    #[error("RPC event stream failed: {0}")]
    Status(Box<Status>),
    /// A decoded envelope violates the bound session, version or sequence contract.
    #[error("RPC event stream protocol violation: {0}")]
    Protocol(&'static str),
}
impl From<Status> for RpcStreamError {
    fn from(status: Status) -> Self {
        Self::Status(Box::new(status))
    }
}

/// Pinned server trust and a mandatory client identity. No native-root fallback is enabled.
pub struct TlsClientConfig {
    server_ca: Vec<u8>,
    certificate: Vec<u8>,
    key: SecretString,
    server_name: Box<str>,
}
impl TlsClientConfig {
    /// Configure mutual TLS and the exact expected server certificate name.
    /// # Errors
    /// Rejects missing/oversized material and invalid DNS-name text; TLS parses keys on connect.
    pub fn new(
        server_ca: Vec<u8>,
        certificate: Vec<u8>,
        key: SecretString,
        server_name: &str,
    ) -> Result<Self, TransportError> {
        for bytes in [
            server_ca.as_slice(),
            certificate.as_slice(),
            key.expose_secret().as_bytes(),
        ] {
            if bytes.is_empty() || bytes.len() > MAX_PEM_BYTES {
                return Err(TransportError::Configuration(
                    "empty or oversized client TLS material",
                ));
            }
        }
        if server_name.is_empty()
            || server_name.len() > 253
            || !server_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
        {
            return Err(TransportError::Configuration(
                "invalid expected TLS server name",
            ));
        }
        Ok(Self {
            server_ca,
            certificate,
            key,
            server_name: server_name.into(),
        })
    }
    fn into_tonic(self) -> ClientTlsConfig {
        ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(self.server_ca))
            .identity(Identity::from_pem(
                self.certificate,
                self.key.expose_secret().as_bytes(),
            ))
            .domain_name(self.server_name.as_ref())
    }
}
/// Client endpoint. Off-loopback TCP cannot be selected without mutual TLS.
pub enum ClientTransport {
    /// Local named pipe or Unix-domain socket.
    Local(LocalEndpoint),
    /// TCP connection to one explicit address, optionally with pinned mutual TLS on loopback.
    Tcp {
        addr: SocketAddr,
        tls: Option<TlsClientConfig>,
    },
}

/// One session-bound client. Every request carries its capability and commands must match it.
#[derive(Clone)]
pub struct KernelClient {
    inner: GeneratedClient,
    session: SessionId,
    credential: TokenBytes,
}
impl fmt::Debug for KernelClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelClient")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}
impl KernelClient {
    /// Connect the actual transport. Readiness/version negotiation is available via [`Self::health`].
    /// Connecting itself never demands an additional `Read` permission from an execute-only token.
    /// # Errors
    /// Invalid/oversized credentials, unsupported endpoint, mandatory-TLS policy or connect failure.
    pub async fn connect(
        transport: ClientTransport,
        session: SessionId,
        credential: TokenBytes,
    ) -> Result<Self, ClientError> {
        let _validated_header = credential_header(&credential)?;
        let channel = match transport {
            ClientTransport::Tcp { addr, tls } => {
                if !addr.ip().is_loopback() && tls.is_none() {
                    return Err(TransportError::TlsRequired.into());
                }
                let scheme = if tls.is_some() { "https" } else { "http" };
                let endpoint = Endpoint::from_shared(format!("{scheme}://{addr}"))
                    .map_err(|_| TransportError::Configuration("invalid TCP URI"))?
                    .connect_timeout(CONNECT_TIMEOUT);
                let endpoint = match tls {
                    Some(tls) => endpoint
                        .tls_config(tls.into_tonic())
                        .map_err(TransportError::from)?,
                    None => endpoint,
                };
                endpoint.connect().await.map_err(TransportError::from)?
            }
            ClientTransport::Local(endpoint) => connect_local(endpoint).await?,
        };
        let inner = GeneratedClient::new(channel)
            .max_decoding_message_size(limits::MAX_MESSAGE_BYTES)
            .max_encoding_message_size(limits::MAX_MESSAGE_BYTES);
        Ok(Self {
            inner,
            session,
            credential,
        })
    }

    fn request<T>(&self, message: T) -> Result<Request<T>, ClientError> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert("authorization", credential_header(&self.credential)?);
        Ok(request)
    }

    /// Query actual readiness and reject missing/incompatible protocol versions.
    /// # Errors
    /// Authentication/transport failure or invalid peer protocol version.
    pub async fn health(&self) -> Result<pb::HealthResponse, ClientError> {
        let mut request = self.request(pb::HealthRequest {})?;
        request.set_timeout(REQUEST_TIMEOUT);
        let response = self.inner.clone().health(request).await?.into_inner();
        let remote = ProtocolVersion::try_from(
            response
                .protocol_version
                .ok_or(ClientError::Protocol("health version missing"))?,
        )
        .map_err(|_| ClientError::Protocol("invalid health version"))?;
        let _agreed = negotiate(PROTOCOL_VERSION, remote)
            .map_err(|_| ClientError::Protocol("incompatible health version"))?;
        Ok(response)
    }

    /// Send an unverified command and retain the exact durable acceptance receipt.
    /// # Errors
    /// Token mismatch, protocol size/receipt violation, or peer rejection.
    pub async fn submit_checked(
        &self,
        command: Command<Unverified>,
    ) -> Result<CommandReceipt, ClientError> {
        if command.token().as_bytes() != self.credential.as_bytes() {
            return Err(ClientError::Protocol(
                "command credential differs from connection credential",
            ));
        }
        let command_id = command.id();
        let wire = pb::SubmitCommandRequest {
            command: Some(pb::Command::from(&command)),
        };
        if wire.encoded_len() > limits::MAX_MESSAGE_BYTES {
            return Err(ClientError::Protocol("command exceeds message limit"));
        }
        let mut request = self.request(wire)?;
        request.set_timeout(REQUEST_TIMEOUT);
        let response = self
            .inner
            .clone()
            .submit_command(request)
            .await?
            .into_inner();
        let received = CommandId::from(
            response
                .command_id
                .ok_or(ClientError::Protocol("receipt command missing"))?,
        );
        if received != command_id {
            return Err(ClientError::Protocol("receipt command mismatch"));
        }
        let accepted_at = response.accepted_at.ok_or(ClientError::Protocol(
            "peer did not provide an acceptance sequence",
        ))?;
        Ok(CommandReceipt {
            command: received,
            accepted_at: Seq::new(accepted_at),
        })
    }

    /// Subscribe to this exact session and preserve typed remote-stream failures.
    /// # Errors
    /// Authentication, peer rejection or transport failure.
    pub async fn subscribe_checked(&self, from: Seq) -> Result<RpcEvents, ClientError> {
        let wire = pb::Uuid {
            hi: u64::try_from(self.session.as_uuid().as_u128() >> 64)
                .map_err(|_| ClientError::Protocol("session overflow"))?,
            lo: u64::try_from(self.session.as_uuid().as_u128() & u128::from(u64::MAX))
                .map_err(|_| ClientError::Protocol("session overflow"))?,
        };
        let response = self
            .inner
            .clone()
            .subscribe(self.request(pb::SubscribeRequest {
                session: Some(wire),
                from_seq: from.get(),
            })?)
            .await?;
        Ok(RpcEvents {
            inner: response.into_inner(),
            session: self.session,
            from,
            last: None,
            terminal_error: None,
            closed: false,
        })
    }

    /// Attach the published bidirectional terminal protocol. Server frame/message limits apply.
    /// # Errors
    /// Authentication/terminal backend refusal or transport error. This never creates a local PTY.
    pub async fn attach_terminal<S>(
        &self,
        inbound: S,
    ) -> Result<tonic::Streaming<pb::TerminalServerFrame>, ClientError>
    where
        S: Stream<Item = pb::TerminalClientFrame> + Send + 'static,
    {
        Ok(self
            .inner
            .clone()
            .attach_terminal(self.request(inbound)?)
            .await?
            .into_inner())
    }
}

impl KernelApi for KernelClient {
    type Events = RpcEvents;
    async fn submit(&self, command: Command<Unverified>) -> Result<CommandReceipt, KernelError> {
        self.submit_checked(command).await.map_err(kernel_error)
    }
    async fn subscribe(&self, from: Seq) -> Result<Self::Events, KernelError> {
        self.subscribe_checked(from).await.map_err(kernel_error)
    }
}

/// Checked event stream. SDK `EventSource` signals EOF on errors; inspect [`Self::terminal_error`]
/// or use [`Self::next_record`] to retain the typed reason rather than treating EOF as success.
pub struct RpcEvents {
    inner: tonic::Streaming<pb::EventRecord>,
    session: SessionId,
    from: Seq,
    last: Option<Seq>,
    terminal_error: Option<RpcStreamError>,
    closed: bool,
}
impl fmt::Debug for RpcEvents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcEvents")
            .field("session", &self.session)
            .field("last", &self.last)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
impl RpcEvents {
    /// The terminal error, if the wire stream failed or violated session/ordering invariants.
    #[must_use]
    pub fn terminal_error(&self) -> Option<&RpcStreamError> {
        self.terminal_error.as_ref()
    }
    /// Read the next verified envelope, preserving remote status errors.
    /// # Errors
    /// Session mismatch, invalid schema/record, nonmonotonic sequence, or transport status.
    pub async fn next_record(&mut self) -> Result<Option<EventRecord>, RpcStreamError> {
        if self.closed {
            return Ok(None);
        }
        let result = self.read_record().await;
        if let Err(error) = &result {
            self.terminal_error = Some(error.clone());
        }
        if result.is_err() || matches!(result, Ok(None)) {
            self.closed = true;
        }
        result
    }
    async fn read_record(&mut self) -> Result<Option<EventRecord>, RpcStreamError> {
        let Some(wire) = self.inner.message().await? else {
            return Ok(None);
        };
        let record = EventRecord::try_from(wire)
            .map_err(|_| RpcStreamError::Protocol("invalid event envelope"))?;
        if record.session() != self.session {
            return Err(RpcStreamError::Protocol("event session mismatch"));
        }
        if record.seq() < self.from || self.last.is_some_and(|last| record.seq() <= last) {
            return Err(RpcStreamError::Protocol("event sequence is not monotonic"));
        }
        self.last = Some(record.seq());
        Ok(Some(record))
    }
}
impl EventSource for RpcEvents {
    async fn next_event(&mut self) -> Option<EventRecord> {
        self.next_record().await.ok().flatten()
    }
}

fn credential_header(token: &TokenBytes) -> Result<MetadataValue<Ascii>, ClientError> {
    if token.is_empty() || token.len() > polaris_security_capability::MAX_TOKEN_BYTES {
        return Err(ClientError::Protocol("empty or oversized credential"));
    }
    let mut header = String::with_capacity(token.len().saturating_mul(2).saturating_add(7));
    header.push_str("Bearer ");
    for byte in token.as_bytes() {
        for nibble in [byte >> 4, byte & 15] {
            header.push(
                char::from_digit(u32::from(nibble), 16)
                    .ok_or(ClientError::Protocol("credential encoding failed"))?,
            );
        }
    }
    let mut value = MetadataValue::try_from(header)
        .map_err(|_| ClientError::Protocol("invalid credential metadata"))?;
    value.set_sensitive(true);
    Ok(value)
}
fn kernel_error(error: ClientError) -> KernelError {
    match error {
        ClientError::Status(status) => match status.code() {
            tonic::Code::ResourceExhausted => KernelError::Busy,
            tonic::Code::Unavailable | tonic::Code::Cancelled => KernelError::Closed,
            tonic::Code::Unauthenticated
            | tonic::Code::PermissionDenied
            | tonic::Code::InvalidArgument => {
                KernelError::Rejected("remote operation rejected".into())
            }
            _ => KernelError::Internal("remote RPC failed".into()),
        },
        ClientError::Transport(_) => KernelError::Closed,
        ClientError::Protocol(_) => KernelError::Internal("RPC protocol violation".into()),
    }
}

#[cfg(windows)]
async fn connect_local(endpoint: LocalEndpoint) -> Result<Channel, TransportError> {
    use hyper_util::rt::TokioIo;
    use tokio::net::windows::named_pipe::ClientOptions;
    let name = endpoint.pipe_path()?.to_owned();
    let connector = tower::service_fn(move |_| {
        let name = name.clone();
        async move {
            let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
            loop {
                match ClientOptions::new().open(&name) {
                    Ok(client) => return Ok::<_, io::Error>(TokioIo::new(client)),
                    Err(error) if error.raw_os_error() == Some(231) => {
                        if tokio::time::Instant::now() >= deadline {
                            return Err(io::Error::from(io::ErrorKind::TimedOut));
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    });
    Endpoint::from_static("http://localhost")
        .connect_timeout(CONNECT_TIMEOUT)
        .connect_with_connector(connector)
        .await
        .map_err(TransportError::from)
}
#[cfg(unix)]
async fn connect_local(endpoint: LocalEndpoint) -> Result<Channel, TransportError> {
    use hyper_util::rt::TokioIo;
    let path = endpoint.unix_path()?.to_owned();
    Endpoint::from_static("http://localhost")
        .connect_timeout(CONNECT_TIMEOUT)
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await
        .map_err(TransportError::from)
}
#[cfg(not(any(unix, windows)))]
async fn connect_local(_endpoint: LocalEndpoint) -> Result<Channel, TransportError> {
    Err(TransportError::Unsupported)
}

#[cfg(windows)]
use std::io;
