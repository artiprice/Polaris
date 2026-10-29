//! A session served by `polarisd`, attached over its local endpoint (P9): the same ordered,
//! text-resolved records and the same submit routing as [`crate::OfflineSession`], with every
//! call authenticated by the credential the daemon issued.

use polaris_rpc::{ClientTransport, KernelClient, LocalEndpoint, RpcEvents};
use polaris_sdk::contracts::{EventRecord, TokenBytes};
use polaris_sdk::ids::{Seq, SessionId};
use polaris_sdk::registry::CommandReceipt;

use crate::{RecordTextKind, ResolvedRecord, SessionHostError, is_single_directive};

/// Where `polarisd` listens: a `polaris-*` named pipe on Windows, a Unix socket path, or
/// `tcp://host:port` for a daemon started with `--listen` (P9-X-2; off loopback only with
/// mutual TLS, [`AttachTls`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachTarget {
    /// Pipe name (`polaris-…`), absolute socket path or `tcp://host:port`.
    pub endpoint: String,
    /// The daemon's session.
    pub session: SessionId,
    /// Credential bytes (decoded from the hex file `polarisd --client-token-out` wrote).
    pub credential: Vec<u8>,
    /// Mutual TLS for a `tcp://` endpoint.
    pub tls: Option<AttachTls>,
}

/// The client side of `polarisd --tls-cert/--tls-key/--client-ca`: PEM files read at attach
/// time, so the key never sits in a long-lived value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachTls {
    /// CA that signed the daemon's certificate.
    pub server_ca: std::path::PathBuf,
    /// This client's certificate, signed by the daemon's `--client-ca`.
    pub certificate: std::path::PathBuf,
    /// This client's private key.
    pub key: std::path::PathBuf,
    /// Name the daemon's certificate must carry (defaults to the endpoint's host).
    pub server_name: Option<String>,
}

impl AttachTls {
    /// From `POLARIS_TLS_CA`, `POLARIS_TLS_CERT` and `POLARIS_TLS_KEY` (paths), with
    /// `POLARIS_TLS_SERVER_NAME` optional; `None` unless all three paths are set.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let path = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        };
        Some(Self {
            server_ca: path("POLARIS_TLS_CA")?,
            certificate: path("POLARIS_TLS_CERT")?,
            key: path("POLARIS_TLS_KEY")?,
            server_name: std::env::var("POLARIS_TLS_SERVER_NAME")
                .ok()
                .filter(|name| !name.is_empty()),
        })
    }
}

/// Largest PEM file read for [`AttachTls`].
const MAX_PEM_BYTES: u64 = 256 * 1024;

fn read_pem(path: &std::path::Path) -> Result<Vec<u8>, SessionHostError> {
    let size = std::fs::metadata(path)
        .map_err(|_| SessionHostError::Unavailable("a TLS file cannot be read"))?
        .len();
    if size == 0 || size > MAX_PEM_BYTES {
        return Err(SessionHostError::Unavailable(
            "a TLS file is empty or larger than 256 KiB",
        ));
    }
    std::fs::read(path).map_err(|_| SessionHostError::Unavailable("a TLS file cannot be read"))
}

impl AttachTarget {
    /// Parse the session (decimal, as `polarisd --session` takes it, or a UUID) and the hex
    /// credential.
    /// # Errors
    /// A malformed session id or credential.
    pub fn new(
        endpoint: &str,
        session: &str,
        credential_hex: &str,
    ) -> Result<Self, SessionHostError> {
        let session = match session.trim().parse::<u128>() {
            Ok(value) => SessionId::from_u128(value),
            Err(_) => session
                .trim()
                .parse::<SessionId>()
                .map_err(|_| SessionHostError::Unavailable("invalid daemon session id"))?,
        };
        let hex = credential_hex.trim();
        if hex.is_empty()
            || !hex.len().is_multiple_of(2)
            || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SessionHostError::Unavailable(
                "credential file is not hexadecimal",
            ));
        }
        let credential = (0..hex.len())
            .step_by(2)
            .map(|at| {
                hex.get(at..at.saturating_add(2))
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or(SessionHostError::Unavailable(
                        "credential file is not hexadecimal",
                    ))
            })
            .collect::<Result<Vec<u8>, _>>()?;
        Ok(Self {
            endpoint: endpoint.to_owned(),
            session,
            credential,
            tls: None,
        })
    }

    /// Use mutual TLS for a `tcp://` endpoint.
    #[must_use]
    pub fn with_tls(mut self, tls: AttachTls) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Use the mutual TLS files the environment names ([`AttachTls::from_env`]), if any.
    #[must_use]
    pub fn with_env_tls(self) -> Self {
        match AttachTls::from_env() {
            Some(tls) => self.with_tls(tls),
            None => self,
        }
    }

    async fn transport(&self) -> Result<ClientTransport, SessionHostError> {
        if let Some(address) = self.endpoint.strip_prefix("tcp://") {
            let resolved = tokio::net::lookup_host(address)
                .await
                .map_err(|_| SessionHostError::Unavailable("the daemon's host does not resolve"))?
                .next()
                .ok_or(SessionHostError::Unavailable(
                    "the daemon's host does not resolve",
                ))?;
            let tls = match &self.tls {
                Some(tls) => {
                    let host = address
                        .rsplit_once(':')
                        .map_or(address, |(host, _)| host)
                        .trim_matches(['[', ']']);
                    let key = String::from_utf8(read_pem(&tls.key)?)
                        .map_err(|_| SessionHostError::Unavailable("the TLS key is not PEM"))?;
                    Some(
                        polaris_rpc::TlsClientConfig::new(
                            read_pem(&tls.server_ca)?,
                            read_pem(&tls.certificate)?,
                            secrecy::SecretString::from(key),
                            tls.server_name.as_deref().unwrap_or(host),
                        )
                        .map_err(|_| SessionHostError::Unavailable("TLS files rejected"))?,
                    )
                }
                None if resolved.ip().is_loopback() => None,
                None => {
                    return Err(SessionHostError::Unavailable(
                        "a daemon off loopback needs --tls-ca, --tls-cert and --tls-key",
                    ));
                }
            };
            return Ok(ClientTransport::Tcp {
                addr: resolved,
                tls,
            });
        }
        let endpoint = if self.endpoint.starts_with("polaris-") {
            LocalEndpoint::named_pipe(&self.endpoint)
        } else {
            LocalEndpoint::unix(std::path::PathBuf::from(&self.endpoint))
        }
        .map_err(|_| SessionHostError::Unavailable("invalid daemon endpoint"))?;
        Ok(ClientTransport::Local(endpoint))
    }
}

/// One attached daemon session.
pub struct RemoteSession {
    /// The daemon's session.
    pub id: SessionId,
    endpoint: String,
    client: KernelClient,
    events: Option<RpcEvents>,
    /// A record taken off the stream whose text is still being read, kept so that a
    /// [`RemoteSession::next_record`] dropped mid-read (a `select!` branch) loses nothing.
    pending: Option<EventRecord>,
}

impl std::fmt::Debug for RemoteSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSession")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

fn remote(error: &polaris_rpc::ClientError) -> SessionHostError {
    match error {
        polaris_rpc::ClientError::Status(status) => {
            SessionHostError::Ui(format!("polarisd: {}", status.message()))
        }
        _ => SessionHostError::Ui("polarisd is not reachable".to_owned()),
    }
}

impl RemoteSession {
    /// Connect, authenticate and check that the daemon is ready.
    /// # Errors
    /// Unreachable endpoint, refused credential or an incompatible daemon.
    pub async fn attach(target: &AttachTarget) -> Result<Self, SessionHostError> {
        let client = KernelClient::connect(
            target.transport().await?,
            target.session,
            TokenBytes::from(target.credential.clone()),
        )
        .await
        .map_err(|error| remote(&error))?;
        let health = client.health().await.map_err(|error| remote(&error))?;
        if !health.ready {
            return Err(SessionHostError::Unavailable("polarisd is not ready"));
        }
        Ok(Self {
            id: target.session,
            endpoint: target.endpoint.clone(),
            client,
            events: None,
            pending: None,
        })
    }

    /// Where the daemon listens.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The next committed record of the session, from the first one on, with directive,
    /// user-input and model-reply text read back from the daemon. Cancel-safe: dropping the
    /// future before it resolves never skips a record.
    /// # Errors
    /// A refused or broken stream, or text the daemon will not serve.
    pub async fn next_record(&mut self) -> Result<Option<ResolvedRecord>, SessionHostError> {
        if self.pending.is_none() {
            let Some(record) = self.next_raw().await? else {
                return Ok(None);
            };
            self.pending = Some(record);
        }
        let target = match self.pending.as_ref().map(EventRecord::event) {
            Some(polaris_sdk::contracts::Event::DirectiveApplied { directive }) => {
                Some((RecordTextKind::Directive, *directive))
            }
            Some(polaris_sdk::contracts::Event::UserInputSubmitted { text }) => {
                Some((RecordTextKind::UserInput, *text))
            }
            // The daemon serves the reply of a step that ended with one; a tool round or a
            // failed step has none and answers NOT_FOUND.
            Some(polaris_sdk::contracts::Event::FrameStepRecorded {
                transcript,
                outcome:
                    polaris_sdk::contracts::FrameStepOutcome::Accepted
                    | polaris_sdk::contracts::FrameStepOutcome::ClaimedDone,
                ..
            }) => Some((RecordTextKind::Assistant, *transcript)),
            _ => None,
        };
        let text = match target {
            Some((kind, cid)) => match self.client.read_text(cid).await {
                Ok(text) => Some((kind, text)),
                Err(error) if kind == RecordTextKind::Assistant && error.is_not_found() => None,
                Err(error) => return Err(remote(&error)),
            },
            None => None,
        };
        let Some(record) = self.pending.take() else {
            return Ok(None);
        };
        Ok(Some(ResolvedRecord {
            record,
            text_kind: text.as_ref().map(|(kind, _)| *kind),
            text: text.map(|(_, text)| text),
        }))
    }

    /// The next envelope off the stream (subscribing first); each await is cancel-safe.
    async fn next_raw(&mut self) -> Result<Option<EventRecord>, SessionHostError> {
        if self.events.is_none() {
            self.events = Some(
                self.client
                    .subscribe_checked(Seq::new(0))
                    .await
                    .map_err(|error| remote(&error))?,
            );
        }
        let Some(events) = self.events.as_mut() else {
            return Ok(None);
        };
        events
            .next_record()
            .await
            .map_err(|_| SessionHostError::Ui("polarisd event stream failed".to_owned()))
    }

    /// Record one line: exactly one directive is applied, anything else is user input.
    /// # Errors
    /// Blank or oversized text, or the daemon's refusal.
    pub async fn submit(&self, text: &str) -> Result<CommandReceipt, SessionHostError> {
        if text.trim().is_empty() {
            return Err(SessionHostError::Unavailable("message is empty"));
        }
        self.client
            .submit_text(text, is_single_directive(text))
            .await
            .map_err(|error| remote(&error))
    }
}
