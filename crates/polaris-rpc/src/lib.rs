//! Authenticated tonic adapters for the actual `polaris.v1` kernel contract.
//!
//! The P1 proto currently exposes `SubmitCommand`, `Subscribe`, `AttachTerminal`, and `Health`.
//! Command and event RPCs adapt the SDK [`polaris_sdk::registry::KernelApi`]; the terminal path
//! delegates to an injected [`TerminalApi`] because the SDK kernel trait does not own a PTY.
//! Every RPC requires an ASCII `authorization: Bearer <hex-token>` header, decoded with a hard
//! byte limit and authorized against trusted facts supplied by the composition root. A submitted
//! command's body token must match that header credential and is still passed unverified to the
//! kernel verifier.
//!
//! [`RpcService::into_grpc_service`] applies the protobuf message size bounds. Event streams use a
//! bounded queue and detach a slow consumer with `RESOURCE_EXHAUSTED`; sequence numbers are
//! monotonic and duplicate/older records are discarded. Stream leases reauthorize on each event
//! or terminal frame and at one-second idle intervals. The resolver's default revocation check
//! allows no revocations; an external revocation source must be supplied by the composition root.
//! [`BoundServer`] owns actual named-pipe/UDS/TCP listeners, including bounded connection admission
//! and cancellation after graceful shutdown. [`KernelClient`] uses these real transports and pins
//! its session and bearer credential; remote TCP requires mutual TLS. The published four-RPC
//! protocol is supported. Later research/audit/session RPC additions retain their own phase gates.

mod auth;
mod client;
mod server;
mod transport;

pub use client::{
    ClientError, ClientTransport, KernelClient, RpcEvents, RpcStreamError, TlsClientConfig,
};

pub use transport::{
    BoundAddress, BoundServer, LocalEndpoint, TlsServerConfig, Transport, TransportError,
    TransportLimits,
};

pub use auth::{
    AuthorizationLease, MAX_BEARER_HEADER_BYTES, MAX_METADATA_BYTES, MAX_METADATA_ENTRIES,
    RequestSubject, RpcAuthenticator, TrustedFacts, TrustedFactsResolver,
};
pub use server::{
    MAX_SESSION_TEXT_BYTES, Readiness, RpcError, RpcLimits, RpcService, SessionTextApi,
    TerminalApi, TextFuture,
};
