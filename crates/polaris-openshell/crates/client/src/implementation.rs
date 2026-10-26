//! Bounded OpenShell gRPC adapter.

use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use openshell_sdk::raw::{self, proto};
use openshell_sdk::{DeletionOutcome, EdgeAuthInterceptor, OpenShellClient, SdkError};
use polaris_openshell_policy::Policy;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{
    CommitOutcome, ExecOutput, ExecRequest, SandboxBackend, SandboxCommitRequest, SandboxError,
    SandboxHandle, SandboxSnapshotPage, SandboxSpec,
};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

/// Maximum gRPC request payload used by OpenShell v0.1.4's gateway.
pub const MAX_REQUEST_CHUNK_BYTES: usize = 896 * 1024;
/// Maximum stdout/stderr bytes retained for a single execution.
pub const MAX_EXEC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum bytes returned by one download call.
pub const MAX_DOWNLOAD_BYTES: usize = MAX_EXEC_OUTPUT_BYTES;
/// Maximum paths returned from one recursive listing.
pub const MAX_LIST_ENTRIES: usize = 65_536;
/// Maximum bytes in one returned sandbox path.
pub const MAX_LIST_PATH_BYTES: usize = 4_096;
/// Maximum encoded request body for the in-sandbox atomic sync helper.
pub const MAX_COMMIT_REQUEST_BYTES: usize = 64 * 1024;
/// Maximum entries requested from one guest snapshot page.
pub const MAX_SNAPSHOT_PAGE_ENTRIES: u32 = 4096;
/// Maximum total entries accepted from one immutable guest snapshot.
pub const MAX_SNAPSHOT_TOTAL_ENTRIES: u64 = 100_000;
const GATEWAY_VERSION: &str = "0.1.4";
const CONNECT_STARTUP_DEADLINE: Duration = Duration::from_secs(10);
const CONNECT_RETRY_BACKOFF: Duration = Duration::from_millis(150);

/// Gateway-observed lifecycle state. Unknown values never imply successful cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPhase {
    /// The gateway is preparing the sandbox.
    Provisioning,
    /// Commands can be admitted.
    Ready,
    /// Creation or execution failed.
    Error,
    /// Deletion is pending; the resource is still present.
    Deleting,
    /// Stop is pending.
    Stopping,
    /// Runtime is stopped but the resource still exists.
    Stopped,
    /// Restart is pending.
    Starting,
    /// Main process completed; the sandbox record still exists.
    Completed,
    /// Unspecified or unrecognized gateway state.
    Unknown,
}

/// Nonsecret authoritative gateway identity for lifecycle diagnostics and recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxObservation {
    /// Immutable gateway resource UUID, distinct from its reusable name.
    pub id: Uuid,
    /// Validated gateway name.
    pub name: SandboxHandle,
    /// Current durable lifecycle phase.
    pub phase: SandboxPhase,
    /// Gateway optimistic concurrency revision.
    pub resource_version: u64,
}

/// TLS materials read from a per-machine OpenShell gateway certificate directory.
#[derive(Clone)]
pub struct GatewayTls {
    /// PEM encoded gateway CA certificate.
    pub ca_certificate: Vec<u8>,
    /// PEM encoded client certificate.
    pub client_certificate: Vec<u8>,
    /// PEM encoded client private key.
    pub client_key: Vec<u8>,
    /// Trusted DNS server name in the gateway certificate.
    pub server_name: String,
}

impl std::fmt::Debug for GatewayTls {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GatewayTls([REDACTED])")
    }
}

/// Gateway configuration. Remote URLs must use TLS; local WSL2 uses `https://localhost:17670`.
#[derive(Debug, Clone)]
pub struct OpenShellConfig {
    /// Gateway base URL.
    pub endpoint: String,
    /// Client and CA TLS material.
    pub tls: GatewayTls,
    /// Required gateway version.
    pub expected_version: String,
    /// OpenShell sandbox driver selected by the gateway installation.
    pub driver: DriverKind,
    /// Readiness timeout after creation.
    pub ready_timeout: Duration,
    /// Default per-command timeout.
    pub exec_timeout: Duration,
}

impl OpenShellConfig {
    /// Standard WSL2 endpoint and v0.1.4 docker gateway settings.
    pub fn wsl2(tls: GatewayTls) -> Self {
        Self {
            endpoint: "https://localhost:17670".to_owned(),
            tls,
            expected_version: GATEWAY_VERSION.to_owned(),
            driver: DriverKind::Docker,
            ready_timeout: Duration::from_secs(120),
            exec_timeout: Duration::from_secs(300),
        }
    }
}

/// Supported Polaris execution drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    /// Full OpenShell policy and exec support for local WSL2.
    Docker,
    /// OpenShell libkrun driver. Experimental and requires KVM on the gateway host.
    Vm,
}

/// Bounded ephemeral storage at a fixed guest path; never a host filesystem bind mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScratchQuota(u32);

impl ScratchQuota {
    /// The sole scratch mount target, inside the sandbox's existing policy boundary.
    pub const PATH: &str = "/sandbox/scratch";

    /// Validate a 1–64 MiB ephemeral scratch quota.
    /// # Errors
    /// Returns [`SandboxError::LimitExceeded`] outside the bounded capacity range.
    pub fn new(bytes: u32) -> Result<Self, SandboxError> {
        if !(1024 * 1024..=64 * 1024 * 1024).contains(&bytes) {
            return Err(SandboxError::LimitExceeded(
                "scratch quota must be 1–64 MiB".to_owned(),
            ));
        }
        Ok(Self(bytes))
    }

    /// Maximum guest tmpfs bytes.
    #[must_use]
    pub const fn bytes(self) -> u32 {
        self.0
    }

    fn driver_config(self) -> prost_types::Struct {
        use prost_types::{ListValue, Struct, Value, value::Kind};
        let field = |kind| Value { kind: Some(kind) };
        let mount = Struct {
            fields: BTreeMap::from([
                (
                    "type".to_owned(),
                    field(Kind::StringValue("tmpfs".to_owned())),
                ),
                (
                    "target".to_owned(),
                    field(Kind::StringValue(Self::PATH.to_owned())),
                ),
                (
                    "size_bytes".to_owned(),
                    field(Kind::NumberValue(f64::from(self.0))),
                ),
                // OpenShell exec uses the sandbox UID; the mount starts root-owned.
                // The existing sandbox policy continues to confine every guest access.
                (
                    "mode".to_owned(),
                    field(Kind::NumberValue(f64::from(0o777_u32))),
                ),
            ]),
        };
        let docker = Struct {
            fields: BTreeMap::from([(
                "mounts".to_owned(),
                field(Kind::ListValue(ListValue {
                    values: vec![field(Kind::StructValue(mount))],
                })),
            )]),
        };
        Struct {
            fields: BTreeMap::from([("docker".to_owned(), field(Kind::StructValue(docker)))]),
        }
    }
}

/// Nonsecret driver identity reported by the authenticated OpenShell gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayDriverInfo {
    /// Gateway-selected name used for routing and configuration.
    pub configured_name: String,
    /// Driver implementation name advertised by the selected driver.
    pub driver_name: String,
    /// Driver implementation version advertised during gateway startup.
    pub driver_version: String,
}

/// Resolve a `SandboxSpec.policy` CID to the exact canonical YAML bytes.
pub trait PolicySource: Send + Sync {
    /// Fetch policy bytes from a content addressed store.
    fn resolve(&self, cid: &Cid) -> Result<Bytes, SandboxError>;
}

/// Bounded in-memory policy catalog for callers that construct policy documents directly.
#[derive(Debug, Default)]
pub struct PolicyCatalog {
    policies: std::sync::RwLock<BTreeMap<Cid, Bytes>>,
}

/// Maximum number of policies kept in the local content-addressed catalog.
pub const MAX_CATALOG_POLICIES: usize = 256;
/// Maximum total policy bytes retained by one local catalog.
pub const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;

impl PolicyCatalog {
    /// Store a validated policy and return the CID computed from canonical YAML.
    pub fn register(&self, policy: &Policy) -> Result<Cid, SandboxError> {
        let bytes = Bytes::from(
            policy
                .canonical_yaml()
                .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?,
        );
        let cid = Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
        let mut policies = self
            .policies
            .write()
            .map_err(|_| SandboxError::Unavailable("policy catalog lock poisoned".to_owned()))?;
        let old_len = match policies.get(&cid) {
            Some(old) => old.len(),
            None => 0,
        };
        let total = policies
            .values()
            .try_fold(bytes.len().saturating_sub(old_len), |sum, value| {
                sum.checked_add(value.len())
            })
            .ok_or_else(|| {
                SandboxError::LimitExceeded("policy catalog size overflow".to_owned())
            })?;
        if total > MAX_CATALOG_BYTES || (old_len == 0 && policies.len() >= MAX_CATALOG_POLICIES) {
            return Err(SandboxError::LimitExceeded(
                "policy catalog capacity exceeded".to_owned(),
            ));
        }
        policies.insert(cid, bytes);
        Ok(cid)
    }
}

impl PolicySource for PolicyCatalog {
    fn resolve(&self, cid: &Cid) -> Result<Bytes, SandboxError> {
        self.policies
            .read()
            .map_err(|_| SandboxError::Unavailable("policy catalog lock poisoned".to_owned()))?
            .get(cid)
            .cloned()
            .ok_or_else(|| SandboxError::NotFound(format!("policy CID {cid}")))
    }
}

/// OpenShell backend. The policy resolver is generic for direct CAS injection by the composition root.
pub struct OpenShellBackend<P = PolicyCatalog> {
    pub(crate) client: Arc<OpenShellClient>,
    policies: P,
    pub(crate) config: OpenShellConfig,
    gateway_driver: GatewayDriverInfo,
}

impl<P: std::fmt::Debug> std::fmt::Debug for OpenShellBackend<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenShellBackend")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OpenShellBackend<PolicyCatalog> {
    /// Connect to OpenShell with a local catalog for policies registered through this backend.
    pub async fn connect(config: OpenShellConfig) -> Result<Self, SandboxError> {
        Self::connect_with_policies(config, PolicyCatalog::default()).await
    }
}

impl<P: PolicySource> OpenShellBackend<P> {
    /// Read lifecycle identity without exposing upstream SDK types or credentials.
    /// Only an authoritative not-found response returns `None`; transport failures propagate.
    ///
    /// # Errors
    /// A timeout, unavailable gateway, invalid UUID, or unexpected returned name.
    pub async fn inspect(
        &self,
        sandbox: &SandboxHandle,
    ) -> Result<Option<SandboxObservation>, SandboxError> {
        let snapshot = match tokio::time::timeout(
            Duration::from_secs(10),
            self.client.get_sandbox(sandbox.as_str()),
        )
        .await
        {
            Err(_) => return Err(SandboxError::Timeout),
            Ok(Err(SdkError::NotFound { .. })) => return Ok(None),
            Ok(Err(error)) => return Err(SandboxError::Unavailable(error.to_string())),
            Ok(Ok(snapshot)) => snapshot,
        };
        if snapshot.name != sandbox.as_str() {
            return Err(SandboxError::Protocol(
                "gateway returned a different sandbox name".into(),
            ));
        }
        let phase = match snapshot.phase {
            openshell_sdk::SandboxPhase::Provisioning => SandboxPhase::Provisioning,
            openshell_sdk::SandboxPhase::Ready => SandboxPhase::Ready,
            openshell_sdk::SandboxPhase::Error => SandboxPhase::Error,
            openshell_sdk::SandboxPhase::Deleting => SandboxPhase::Deleting,
            openshell_sdk::SandboxPhase::Stopping => SandboxPhase::Stopping,
            openshell_sdk::SandboxPhase::Stopped => SandboxPhase::Stopped,
            openshell_sdk::SandboxPhase::Starting => SandboxPhase::Starting,
            openshell_sdk::SandboxPhase::Completed => SandboxPhase::Completed,
            _ => SandboxPhase::Unknown,
        };
        let id = Uuid::parse_str(&snapshot.id).map_err(|_| {
            SandboxError::Protocol("gateway returned an invalid resource UUID".into())
        })?;
        Ok(Some(SandboxObservation {
            id,
            name: sandbox.clone(),
            phase,
            resource_version: snapshot.resource_version,
        }))
    }

    /// Connect and fail closed unless the gateway reports the configured version.
    pub async fn connect_with_policies(
        config: OpenShellConfig,
        policies: P,
    ) -> Result<Self, SandboxError> {
        if !config.endpoint.starts_with("https://") {
            return Err(SandboxError::PolicyDenied(
                "OpenShell gateways require TLS".to_owned(),
            ));
        }
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(config.tls.ca_certificate.clone()))
            .identity(Identity::from_pem(
                config.tls.client_certificate.clone(),
                config.tls.client_key.clone(),
            ))
            .domain_name(config.tls.server_name.clone());
        let endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|error| SandboxError::Protocol(error.to_string()))?
            .connect_timeout(Duration::from_secs(10))
            .tls_config(tls)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let channel = match retry_transient_connect(
            CONNECT_STARTUP_DEADLINE,
            || endpoint.connect(),
            |error: &tonic::transport::Error| is_transient_connect_error(error),
        )
        .await
        {
            Ok(channel) => channel,
            Err(ConnectRetryError::Deadline) => {
                return Err(SandboxError::Unavailable(
                    "OpenShell gateway connection exceeded the 10 second startup deadline"
                        .to_owned(),
                ));
            }
            Err(ConnectRetryError::Failed(error)) => {
                return Err(SandboxError::Unavailable(error.to_string()));
            }
        };
        let client = OpenShellClient::from_parts(channel, EdgeAuthInterceptor::noop());
        let health = client
            .health()
            .await
            .map_err(|error| SandboxError::Unavailable(error.to_string()))?;
        if health.version != config.expected_version {
            return Err(SandboxError::Protocol(format!(
                "OpenShell version mismatch: expected {}, found {}",
                config.expected_version, health.version
            )));
        }
        let gateway_driver = read_gateway_driver(&client).await?;
        validate_gateway_driver_kind(&gateway_driver, config.driver)?;
        Ok(Self {
            client: Arc::new(client),
            policies,
            config,
            gateway_driver,
        })
    }

    /// Return the authenticated gateway's selected, nonsecret driver metadata.
    pub fn gateway_driver_info(&self) -> &GatewayDriverInfo {
        &self.gateway_driver
    }

    /// Return the configured gateway driver.
    pub fn driver(&self) -> DriverKind {
        self.config.driver
    }

    /// Register a policy in the local catalog and return the CID to place in `SandboxSpec`.
    pub fn register_policy(&self, policy: &Policy) -> Result<Cid, SandboxError>
    where
        P: PolicySource + CatalogAccess,
    {
        self.policies.register(policy)
    }

    /// Gateway health/version already checked at connect time.
    pub async fn health(&self) -> Result<String, SandboxError> {
        self.client
            .health()
            .await
            .map(|health| health.version)
            .map_err(|error| SandboxError::Unavailable(error.to_string()))
    }

    async fn create_raw(
        &self,
        spec: &SandboxSpec,
        bytes: Bytes,
        policy: Policy,
        providers: &[String],
        scratch: Option<ScratchQuota>,
    ) -> Result<SandboxHandle, SandboxError> {
        if let Some(name) = spec.name.as_deref() {
            validate_gateway_name(name)?;
        }
        let image = spec.image.as_deref().ok_or_else(|| {
            SandboxError::PolicyDenied(
                "OpenShell sandbox image must be pinned by digest".to_owned(),
            )
        })?;
        validate_image_reference(image)?;
        let content_hash = blake3::hash(&bytes);
        if spec.policy != Some(Cid::from_bytes(*content_hash.as_bytes())) {
            return Err(SandboxError::PolicyDenied(
                "policy CID does not match resolved policy bytes".to_owned(),
            ));
        }
        let wire_policy = to_wire_policy(policy.document())?;
        let resources = sandbox_resource_limits(spec.cpu_millis, spec.memory_mib);
        let template = Some(proto::SandboxTemplate {
            image: image.to_owned(),
            resources,
            driver_config: scratch.map(ScratchQuota::driver_config),
            ..Default::default()
        });
        let request = raw::CreateSandboxRequest {
            request_id: String::new(),
            spec: Some(raw::ProtoSandboxSpec {
                policy: Some(wire_policy),
                template,
                providers: providers.to_vec(),
                ..Default::default()
            }),
            name: match &spec.name {
                Some(name) => name.clone(),
                None => String::new(),
            },
            workspace_scope: Some(proto::workspace_selector("default")),
            ..Default::default()
        };
        let response = self
            .client
            .raw_grpc()
            .create_sandbox(request)
            .await
            .map_err(map_create_error)?
            .into_inner();
        let Some(sandbox) = response.sandbox else {
            return Err(SandboxError::Protocol(
                "create sandbox response omitted sandbox".to_owned(),
            ));
        };
        let Some(metadata) = sandbox.metadata else {
            return Err(SandboxError::Protocol(
                "create sandbox response omitted metadata".to_owned(),
            ));
        };
        let name = metadata.name;
        if name.is_empty() {
            return Err(SandboxError::Protocol(
                "gateway returned empty sandbox name".to_owned(),
            ));
        }
        let mut cleanup = SandboxCleanupGuard::new(Arc::clone(&self.client), name.clone());
        let ready = match self
            .client
            .wait_ready(&name, self.config.ready_timeout)
            .await
        {
            Ok(ready) => ready,
            Err(error) => {
                return Err(SandboxError::Unavailable(format!(
                    "sandbox readiness failed; cleanup scheduled: {error}"
                )));
            }
        };
        let Some(handle) = SandboxHandle::new(&ready.name).ok() else {
            return Err(SandboxError::Protocol(
                "gateway returned invalid sandbox name".to_owned(),
            ));
        };
        cleanup.disarm();
        Ok(handle)
    }

    /// Create a sandbox with explicit OpenShell provider profile references.
    pub async fn create_with_profiles(
        &self,
        spec: &SandboxSpec,
        providers: &[String],
    ) -> Result<SandboxHandle, SandboxError> {
        let Some(cid) = spec.policy else {
            return Err(SandboxError::PolicyDenied(
                "SandboxSpec.policy is required".to_owned(),
            ));
        };
        let bytes = self.policies.resolve(&cid)?;
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != cid {
            return Err(SandboxError::PolicyDenied(
                "resolved policy content does not match its CID".to_owned(),
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let policy = Policy::parse_yaml(text)
            .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        self.create_raw(spec, bytes, policy, providers, None).await
    }

    /// Create an explicitly requested sandbox with bounded ephemeral scratch capacity.
    ///
    /// Accepts no host source or arbitrary mount path. Image, CPU/memory limits, exact policy,
    /// readiness checks and cleanup ownership are identical to ordinary creation.
    /// # Errors
    /// Rejects non-Docker drivers, missing/invalid policy, or normal create/readiness failures.
    pub async fn create_with_scratch(
        &self,
        spec: &SandboxSpec,
        scratch: ScratchQuota,
    ) -> Result<SandboxHandle, SandboxError> {
        if self.config.driver != DriverKind::Docker {
            return Err(SandboxError::PolicyDenied(
                "bounded scratch currently requires the Docker driver".to_owned(),
            ));
        }
        let cid = spec.policy.ok_or_else(|| {
            SandboxError::PolicyDenied("SandboxSpec.policy is required".to_owned())
        })?;
        let bytes = self.policies.resolve(&cid)?;
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != cid {
            return Err(SandboxError::PolicyDenied(
                "resolved policy content does not match its CID".to_owned(),
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let policy = Policy::parse_yaml(text)
            .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        self.create_raw(spec, bytes, policy, &[], Some(scratch))
            .await
    }

    async fn exec_bounded(
        &self,
        sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        if request.output_limit_bytes == Some(0) {
            return Err(SandboxError::LimitExceeded(
                "exec output limit must be positive".to_owned(),
            ));
        }
        if request.argv.is_empty() {
            return Err(SandboxError::Protocol(
                "exec argv must not be empty".to_owned(),
            ));
        }
        let mut environment = std::collections::HashMap::new();
        for (key, value) in &request.env {
            if key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0') {
                return Err(SandboxError::Protocol(
                    "invalid environment entry".to_owned(),
                ));
            }
            environment.insert(key.clone(), value.clone());
        }
        let stdin = match request.stdin.as_ref() {
            Some(bytes) if bytes.len() <= MAX_REQUEST_CHUNK_BYTES => bytes.to_vec(),
            Some(_) => {
                return Err(SandboxError::LimitExceeded(format!(
                    "exec stdin exceeds {MAX_REQUEST_CHUNK_BYTES} bytes"
                )));
            }
            None => Vec::new(),
        };
        let execution_id = Uuid::now_v7().to_string();
        let pid_file = format!("/tmp/polaris-exec-{execution_id}.pid");
        let mut command = vec![
            "sh".to_owned(), "-c".to_owned(),
            "pid_file=$1; shift; setsid --wait sh -c 'printf \"%s\\n\" \"$$\" > \"$1\"; shift; exec \"$@\"' polaris-exec \"$pid_file\" \"$@\"; result=$?; rm -f -- \"$pid_file\"; exit \"$result\"".to_owned(),
            "polaris-exec".to_owned(), pid_file.clone(),
        ];
        command.extend(request.argv.iter().cloned());
        let rpc = raw::ExecSandboxRequest {
            request_id: String::new(),
            sandbox: String::from(sandbox.clone()),
            workspace_scope: Some(proto::workspace_selector("default")),
            command,
            workdir: match &request.workdir {
                Some(path) => path.clone(),
                None => String::new(),
            },
            environment,
            execution_timeout: None,
            stdin,
            tty: false,
            cols: 0,
            rows: 0,
            no_login_shell: true,
        };
        let mut cleanup = ExecCleanupGuard::new(
            Arc::clone(&self.client),
            String::from(sandbox.clone()),
            pid_file,
        );
        let started = Instant::now();
        let call = async {
            use futures::StreamExt;
            let mut stream = self
                .client
                .raw_grpc()
                .exec_sandbox(rpc)
                .await
                .map_err(|error| SandboxError::Unavailable(error.to_string()))?
                .into_inner();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_code = None;
            while let Some(event) = stream.next().await {
                let event = event.map_err(|error| SandboxError::Unavailable(error.to_string()))?;
                match event.payload {
                    Some(proto::exec_sandbox_event::Payload::Stdout(chunk)) => {
                        check_combined_output(
                            stdout.len(),
                            stderr.len(),
                            chunk.data.len(),
                            request.output_limit_bytes,
                        )?;
                        extend_bounded(&mut stdout, &chunk.data)?
                    }
                    Some(proto::exec_sandbox_event::Payload::Stderr(chunk)) => {
                        check_combined_output(
                            stdout.len(),
                            stderr.len(),
                            chunk.data.len(),
                            request.output_limit_bytes,
                        )?;
                        extend_bounded(&mut stderr, &chunk.data)?
                    }
                    Some(proto::exec_sandbox_event::Payload::Exit(exit)) => {
                        exit_code = Some(exit.exit_code)
                    }
                    None => {}
                }
            }
            let Some(exit_code) = exit_code else {
                return Err(SandboxError::Protocol(
                    "exec stream ended without an exit event".to_owned(),
                ));
            };
            Ok(ExecOutput {
                exit_code,
                stdout: Bytes::from(stdout),
                stderr: Bytes::from(stderr),
                duration: started.elapsed(),
            })
        };
        let timeout = match request.timeout {
            Some(value) => value,
            None => self.config.exec_timeout,
        };
        let result = match tokio::time::timeout(timeout, call).await {
            Ok(result) => result,
            Err(_) => Err(SandboxError::Timeout),
        };
        if result.is_ok() {
            cleanup.disarm();
        }
        result
    }

    async fn shell_transfer(
        &self,
        sandbox: &SandboxHandle,
        command: Vec<String>,
        stdin: Option<Bytes>,
        cap: usize,
    ) -> Result<Bytes, SandboxError> {
        let req = ExecRequest {
            argv: command,
            stdin,
            timeout: Some(self.config.exec_timeout),
            ..Default::default()
        };
        let out = self.exec_bounded(sandbox, &req).await?;
        if out.exit_code != 0 {
            return Err(SandboxError::Protocol(format!(
                "sandbox transfer command exited {}",
                out.exit_code
            )));
        }
        if out.stdout.len() > cap {
            return Err(SandboxError::LimitExceeded(format!(
                "sandbox transfer output exceeds {cap} bytes"
            )));
        }
        Ok(out.stdout)
    }
}

/// Marker for a policy source that also exposes a concrete catalog registration method.
pub trait CatalogAccess: PolicySource {
    /// Register canonical policy bytes and return their CID.
    fn register(&self, policy: &Policy) -> Result<Cid, SandboxError>;
}

impl CatalogAccess for PolicyCatalog {
    fn register(&self, policy: &Policy) -> Result<Cid, SandboxError> {
        PolicyCatalog::register(self, policy)
    }
}

impl<P: PolicySource> SandboxBackend for OpenShellBackend<P> {
    fn recovery_domain(&self) -> Option<Cid> {
        openshell_recovery_domain(&self.config, &self.gateway_driver)
    }

    async fn confirm_absence(&self, sandbox: &SandboxHandle) -> Result<bool, SandboxError> {
        Ok(self.inspect(sandbox).await?.is_none())
    }

    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let Some(cid) = spec.policy else {
            return Err(SandboxError::PolicyDenied(
                "SandboxSpec.policy is required".to_owned(),
            ));
        };
        let bytes = self.policies.resolve(&cid)?;
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != cid {
            return Err(SandboxError::PolicyDenied(
                "resolved policy content does not match its CID".to_owned(),
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let policy = Policy::parse_yaml(text)
            .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        self.create_raw(spec, bytes, policy, &[], None).await
    }

    async fn exec(
        &self,
        sandbox: &SandboxHandle,
        req: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        self.exec_bounded(sandbox, req).await
    }

    async fn upload(
        &self,
        sandbox: &SandboxHandle,
        path: &str,
        bytes: Bytes,
    ) -> Result<(), SandboxError> {
        validate_path(path)?;
        for (index, chunk) in bytes.chunks(MAX_REQUEST_CHUNK_BYTES).enumerate() {
            let append = if index == 0 { ">" } else { ">>" };
            let command = vec![
                "sh".to_owned(),
                "-c".to_owned(),
                format!("umask 077; cat {append} \"$1\"",),
                "polaris-upload".to_owned(),
                path.to_owned(),
            ];
            let request = ExecRequest {
                argv: command,
                stdin: Some(Bytes::copy_from_slice(chunk)),
                timeout: Some(self.config.exec_timeout),
                ..Default::default()
            };
            let result = self.exec_bounded(sandbox, &request).await?;
            if result.exit_code != 0 {
                return Err(SandboxError::Protocol(format!(
                    "upload command exited {}",
                    result.exit_code
                )));
            }
        }
        Ok(())
    }

    async fn download(&self, sandbox: &SandboxHandle, path: &str) -> Result<Bytes, SandboxError> {
        validate_path(path)?;
        let command = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "cat -- \"$1\"".to_owned(),
            "polaris-download".to_owned(),
            path.to_owned(),
        ];
        self.shell_transfer(sandbox, command, None, MAX_DOWNLOAD_BYTES)
            .await
    }

    async fn list_files(
        &self,
        sandbox: &SandboxHandle,
        dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        validate_path(dir)?;
        let command = vec![
            "find".to_owned(),
            dir.to_owned(),
            "-type".to_owned(),
            "f".to_owned(),
            "-print".to_owned(),
        ];
        let output = self
            .shell_transfer(sandbox, command, None, MAX_EXEC_OUTPUT_BYTES)
            .await?;
        let text = std::str::from_utf8(&output)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        parse_file_list(text)
    }

    async fn snapshot_page(
        &self,
        sandbox: &SandboxHandle,
        root: &str,
        generation: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<SandboxSnapshotPage, SandboxError> {
        validate_path(root)?;
        if let Some(generation) = generation {
            validate_generation(generation)?;
        }
        if !(1..=MAX_SNAPSHOT_PAGE_ENTRIES).contains(&limit)
            || (generation.is_none() && offset != 0)
        {
            return Err(SandboxError::PolicyDenied(
                "invalid snapshot page request".to_owned(),
            ));
        }
        let generation_arg = match generation {
            Some(token) => token.to_owned(),
            None => "-".to_owned(),
        };
        let output = self
            .exec_bounded(
                sandbox,
                &ExecRequest {
                    argv: vec![
                        "/usr/bin/polaris-guest".to_owned(),
                        "sync-snapshot".to_owned(),
                        root.to_owned(),
                        generation_arg,
                        offset.to_string(),
                        limit.to_string(),
                    ],
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(SandboxError::Protocol(format!(
                "sync-snapshot helper exited with code {}",
                output.exit_code
            )));
        }
        parse_snapshot_page(&output.stdout, generation, offset, limit)
    }

    async fn destroy(&self, sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        let sandbox_name = String::from(sandbox.clone());
        let result = match self
            .client
            .delete_sandbox(&sandbox_name, Default::default())
            .await
        {
            Ok(result) => result,
            Err(SdkError::NotFound { .. }) => return Ok(()),
            Err(error) => return Err(SandboxError::Unavailable(error.to_string())),
        };
        let expected_sandbox_id = validate_delete_ack(result)?;
        self.client
            .wait_deleted(
                &sandbox_name,
                Duration::from_secs(30),
                expected_sandbox_id.as_deref(),
            )
            .await
            .map_err(|error| SandboxError::Unavailable(error.to_string()))
    }

    async fn conditional_commit(
        &self,
        sandbox: &SandboxHandle,
        request: &SandboxCommitRequest,
    ) -> Result<CommitOutcome, SandboxError> {
        let payload = encode_commit_request(request)?;
        let output = self
            .exec_bounded(
                sandbox,
                &ExecRequest {
                    argv: vec!["/usr/bin/polaris-guest".to_owned(), "sync-cas".to_owned()],
                    stdin: Some(Bytes::from(payload)),
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;
        match output.exit_code {
            0 => Ok(CommitOutcome::Applied),
            3 => Ok(CommitOutcome::Conflict),
            code => Err(SandboxError::Protocol(format!(
                "sync-cas helper exited with code {code}"
            ))),
        }
    }

    async fn prepare_sync(&self, sandbox: &SandboxHandle, root: &str) -> Result<(), SandboxError> {
        validate_path(root)?;
        let output = self
            .exec_bounded(
                sandbox,
                &ExecRequest {
                    argv: vec![
                        "/usr/bin/polaris-guest".to_owned(),
                        "sync-init".to_owned(),
                        root.to_owned(),
                    ],
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(SandboxError::Protocol(format!(
                "sync-init helper exited with code {}: {}",
                output.exit_code,
                diagnostic_preview(&output.stderr, 2048)
            )))
        }
    }
}

pub(crate) struct SandboxCleanupGuard {
    client: Arc<OpenShellClient>,
    name: String,
    armed: bool,
}

impl SandboxCleanupGuard {
    pub(crate) fn new(client: Arc<OpenShellClient>, name: String) -> Self {
        Self {
            client,
            name,
            armed: true,
        }
    }
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SandboxCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let client = Arc::clone(&self.client);
        let name = self.name.clone();
        let cleanup = async move {
            if let Err(error) = client.delete_sandbox(&name, Default::default()).await {
                tracing::error!(sandbox = %name, %error, "failed to clean up sandbox after cancelled create");
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                drop(runtime.spawn(cleanup));
            }
            Err(error) => {
                tracing::error!(sandbox = %self.name, %error, "cannot schedule sandbox cleanup without a Tokio runtime")
            }
        }
    }
}

struct ExecCleanupGuard {
    client: Arc<OpenShellClient>,
    sandbox: String,
    pid_file: String,
    armed: bool,
}

impl ExecCleanupGuard {
    fn new(client: Arc<OpenShellClient>, sandbox: String, pid_file: String) -> Self {
        Self {
            client,
            sandbox,
            pid_file,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ExecCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let client = Arc::clone(&self.client);
        let sandbox = self.sandbox.clone();
        let pid_file = self.pid_file.clone();
        let cleanup = async move {
            let script = "i=0; while [ ! -s \"$1\" ] && [ $i -lt 20 ]; do sleep 0.05; i=$((i+1)); done; if [ -s \"$1\" ]; then read -r pid < \"$1\"; kill -TERM -- \"-$pid\"; rm -f -- \"$1\"; fi";
            let command = vec![
                "sh".to_owned(),
                "-c".to_owned(),
                script.to_owned(),
                "polaris-cancel".to_owned(),
                pid_file,
            ];
            let result = client
                .exec(
                    &sandbox,
                    &command,
                    openshell_sdk::ExecOptions {
                        no_login_shell: true,
                        ..Default::default()
                    },
                )
                .await;
            match result {
                Ok(output) if output.exit_code == 0 => {}
                Ok(output) => {
                    tracing::error!(sandbox = %sandbox, exit_code = output.exit_code, "remote exec cleanup exited unsuccessfully")
                }
                Err(error) => {
                    tracing::error!(sandbox = %sandbox, %error, "failed to stop remote exec after cancellation or output limit")
                }
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                drop(runtime.spawn(cleanup));
            }
            Err(error) => {
                tracing::error!(sandbox = %self.sandbox, %error, "cannot schedule remote exec cleanup without a Tokio runtime")
            }
        }
    }
}

enum ConnectRetryError<E> {
    Deadline,
    Failed(E),
}

async fn retry_transient_connect<T, E, F, Fut>(
    deadline: Duration,
    mut attempt: F,
    is_transient: impl Fn(&E) -> bool,
) -> Result<T, ConnectRetryError<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let deadline = tokio::time::Instant::now() + deadline;
    loop {
        match tokio::time::timeout_at(deadline, attempt()).await {
            Err(_) => return Err(ConnectRetryError::Deadline),
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) if !is_transient(&error) => {
                return Err(ConnectRetryError::Failed(error));
            }
            Ok(Err(_)) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ConnectRetryError::Deadline);
                }
                tokio::time::sleep_until(
                    (tokio::time::Instant::now() + CONNECT_RETRY_BACKOFF).min(deadline),
                )
                .await;
            }
        }
    }
}

fn is_transient_connect_error(error: &(dyn Error + 'static)) -> bool {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if let Some(io_error) = current.downcast_ref::<std::io::Error>() {
            return matches!(
                io_error.kind(),
                ErrorKind::ConnectionRefused | ErrorKind::NotConnected
            );
        }
        cause = current.source();
    }
    false
}

fn map_create_error(error: tonic::Status) -> SandboxError {
    if error.code() == tonic::Code::AlreadyExists {
        SandboxError::AlreadyExists(error.message().to_owned())
    } else {
        SandboxError::Unavailable(error.to_string())
    }
}

const fn expected_driver_name(kind: DriverKind) -> &'static str {
    match kind {
        DriverKind::Docker => "docker",
        DriverKind::Vm => "vm",
    }
}

fn validate_gateway_driver_kind(
    driver: &GatewayDriverInfo,
    expected: DriverKind,
) -> Result<(), SandboxError> {
    let expected_name = expected_driver_name(expected);
    if driver.driver_name.eq_ignore_ascii_case(expected_name) {
        Ok(())
    } else {
        Err(SandboxError::Protocol(format!(
            "OpenShell driver mismatch: configured {expected_name}, gateway reports {}",
            driver.driver_name
        )))
    }
}

async fn read_gateway_driver(client: &OpenShellClient) -> Result<GatewayDriverInfo, SandboxError> {
    let mut grpc = client
        .raw_grpc_fresh()
        .await
        .map_err(|error| SandboxError::Unavailable(error.to_string()))?;
    let response = grpc
        .get_gateway_info(proto::GetGatewayInfoRequest::default())
        .await
        .map_err(|error| SandboxError::Unavailable(error.to_string()))?
        .into_inner();
    let mut drivers = response.compute_drivers.into_iter();
    let driver = drivers.next().ok_or_else(|| {
        SandboxError::Protocol("OpenShell gateway reports no selected compute driver".to_owned())
    })?;
    if drivers.next().is_some() {
        return Err(SandboxError::Protocol(
            "OpenShell gateway reports multiple selected compute drivers".to_owned(),
        ));
    }
    let capabilities = driver.capabilities.ok_or_else(|| {
        SandboxError::Protocol("OpenShell compute driver omitted capabilities".to_owned())
    })?;
    if driver.name.trim().is_empty()
        || capabilities.driver_name.trim().is_empty()
        || capabilities.driver_version.trim().is_empty()
    {
        return Err(SandboxError::Protocol(
            "OpenShell gateway returned incomplete compute driver identity".to_owned(),
        ));
    }
    Ok(GatewayDriverInfo {
        configured_name: driver.name,
        driver_name: capabilities.driver_name,
        driver_version: capabilities.driver_version,
    })
}

fn validate_delete_ack(
    result: openshell_sdk::DeletionResult,
) -> Result<Option<String>, SandboxError> {
    match result.outcome {
        DeletionOutcome::Completed => Ok(result.sandbox_id),
        DeletionOutcome::AlreadyAbsent if result.sandbox_id.is_none() => Ok(None),
        DeletionOutcome::Accepted if result.sandbox_id.is_some() => Ok(result.sandbox_id),
        outcome => Err(SandboxError::Protocol(format!(
            "gateway returned invalid delete acknowledgement {outcome:?}"
        ))),
    }
}

fn check_combined_output(
    stdout: usize,
    stderr: usize,
    incoming: usize,
    limit: Option<usize>,
) -> Result<(), SandboxError> {
    let total = stdout
        .checked_add(stderr)
        .and_then(|bytes| bytes.checked_add(incoming))
        .ok_or_else(|| {
            SandboxError::LimitExceeded("combined exec output length overflow".to_owned())
        })?;
    if limit.is_some_and(|cap| total > cap) {
        return Err(SandboxError::LimitExceeded(
            "combined exec output exceeds requested limit".to_owned(),
        ));
    }
    Ok(())
}

fn extend_bounded(target: &mut Vec<u8>, chunk: &[u8]) -> Result<(), SandboxError> {
    let new_len = target
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| SandboxError::LimitExceeded("exec output length overflow".to_owned()))?;
    if new_len > MAX_EXEC_OUTPUT_BYTES {
        return Err(SandboxError::LimitExceeded(format!(
            "exec output exceeds {MAX_EXEC_OUTPUT_BYTES} bytes"
        )));
    }
    target.extend_from_slice(chunk);
    Ok(())
}

fn diagnostic_preview(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).into_owned()
}

fn validate_path(path: &str) -> Result<(), SandboxError> {
    if !path.starts_with('/')
        || path.len() > 4_096
        || path.contains('\0')
        || path.split('/').any(|part| part == "..")
    {
        return Err(SandboxError::PolicyDenied(format!(
            "invalid sandbox absolute path: {path}"
        )));
    }
    Ok(())
}

fn validate_gateway_name(name: &str) -> Result<(), SandboxError> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(SandboxError::Protocol(
            "OpenShell sandbox name must not be empty".to_owned(),
        ));
    };
    if name.len() > 19
        || !first.is_ascii_alphanumeric()
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(SandboxError::Protocol("OpenShell v0.1.4 sandbox names must be 1-19 ASCII characters, begin with an alphanumeric, and contain only alphanumerics, '-', '_' or '.'".to_owned()));
    }
    Ok(())
}

/// Convert Polaris quota units to the pinned gateway's Kubernetes-style limit Struct.
/// OpenShell v0.1.4's curated `SandboxSpec` omits CPU/memory, so create uses the
/// SDK-authenticated raw RPC and this official `SandboxTemplate.resources` field.
fn sandbox_resource_limits(
    cpu_millis: Option<u32>,
    memory_mib: Option<u32>,
) -> Option<prost_types::Struct> {
    use prost_types::{Struct, Value, value::Kind};

    let mut limits = BTreeMap::new();
    if let Some(cpu_millis) = cpu_millis {
        limits.insert(
            "cpu".to_owned(),
            Value {
                kind: Some(Kind::StringValue(format!("{cpu_millis}m"))),
            },
        );
    }
    if let Some(memory_mib) = memory_mib {
        limits.insert(
            "memory".to_owned(),
            Value {
                kind: Some(Kind::StringValue(format!("{memory_mib}Mi"))),
            },
        );
    }
    if limits.is_empty() {
        return None;
    }
    Some(Struct {
        fields: BTreeMap::from([(
            "limits".to_owned(),
            Value {
                kind: Some(Kind::StructValue(Struct { fields: limits })),
            },
        )]),
    })
}

fn validate_image_reference(image: &str) -> Result<(), SandboxError> {
    let digest = if let Some(digest) = image.strip_prefix("sha256:") {
        digest
    } else if let Some((repository, digest)) = image.rsplit_once("@sha256:") {
        if repository.is_empty() {
            return Err(SandboxError::PolicyDenied(
                "OpenShell sandbox image must use an immutable sha256 reference".to_owned(),
            ));
        }
        digest
    } else {
        return Err(SandboxError::PolicyDenied(
            "OpenShell sandbox image must use an immutable sha256 reference".to_owned(),
        ));
    };
    if image.contains(char::is_whitespace)
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(SandboxError::PolicyDenied(
            "OpenShell sandbox image must use an immutable sha256 reference".to_owned(),
        ));
    }
    Ok(())
}

fn parse_file_list(text: &str) -> Result<Vec<String>, SandboxError> {
    let mut paths = Vec::new();
    for line in text.lines() {
        if paths.len() >= MAX_LIST_ENTRIES {
            return Err(SandboxError::LimitExceeded(format!(
                "file listing exceeds {MAX_LIST_ENTRIES} entries"
            )));
        }
        if line.len() > MAX_LIST_PATH_BYTES {
            return Err(SandboxError::LimitExceeded(format!(
                "listed path exceeds {MAX_LIST_PATH_BYTES} bytes"
            )));
        }
        validate_path(line)?;
        let mut path = String::new();
        path.try_reserve_exact(line.len()).map_err(|error| {
            SandboxError::LimitExceeded(format!("cannot allocate listed path: {error}"))
        })?;
        path.push_str(line);
        paths.try_reserve(1).map_err(|error| {
            SandboxError::LimitExceeded(format!("cannot grow file listing: {error}"))
        })?;
        paths.push(path);
    }
    Ok(paths)
}

fn validate_generation(generation: &str) -> Result<(), SandboxError> {
    if generation.len() != 64
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SandboxError::PolicyDenied(
            "invalid sync snapshot generation token".to_owned(),
        ));
    }
    Ok(())
}

fn parse_snapshot_page(
    bytes: &[u8],
    generation: Option<&str>,
    offset: u64,
    limit: u32,
) -> Result<SandboxSnapshotPage, SandboxError> {
    if bytes.len() > MAX_EXEC_OUTPUT_BYTES {
        return Err(SandboxError::LimitExceeded(
            "sync snapshot page exceeds the execution output limit".to_owned(),
        ));
    }
    let page: SandboxSnapshotPage = serde_json::from_slice(bytes)
        .map_err(|error| SandboxError::Protocol(format!("invalid sync snapshot JSON: {error}")))?;
    validate_snapshot_page(&page, generation, offset, limit)?;
    Ok(page)
}

fn validate_snapshot_page(
    page: &SandboxSnapshotPage,
    generation: Option<&str>,
    offset: u64,
    limit: u32,
) -> Result<(), SandboxError> {
    validate_generation(&page.generation)?;
    if generation.is_some_and(|expected| expected != page.generation)
        || page.offset != offset
        || page.total_entries > MAX_SNAPSHOT_TOTAL_ENTRIES
        || page.entries.len()
            > usize::try_from(limit).map_err(|_| {
                SandboxError::LimitExceeded("sync snapshot page size overflow".to_owned())
            })?
    {
        return Err(SandboxError::Protocol(
            "sync snapshot page metadata does not match its request".to_owned(),
        ));
    }
    let page_end = offset
        .checked_add(u64::try_from(page.entries.len()).map_err(|_| {
            SandboxError::LimitExceeded("sync snapshot page entry count overflow".to_owned())
        })?)
        .ok_or_else(|| {
            SandboxError::LimitExceeded("sync snapshot page offset overflow".to_owned())
        })?;
    if page_end > page.total_entries
        || page.next_offset != (page_end < page.total_entries).then_some(page_end)
    {
        return Err(SandboxError::Protocol(
            "sync snapshot page continuation is inconsistent".to_owned(),
        ));
    }

    let mut previous_path: Option<&str> = None;
    for entry in &page.entries {
        validate_snapshot_entry(entry)?;
        if previous_path.is_some_and(|previous| previous >= entry.relative_path.as_str()) {
            return Err(SandboxError::Protocol(
                "sync snapshot page paths are not strictly ordered".to_owned(),
            ));
        }
        previous_path = Some(&entry.relative_path);
    }
    Ok(())
}

fn validate_snapshot_entry(
    entry: &polaris_sdk::registry::SandboxSnapshotEntry,
) -> Result<(), SandboxError> {
    let path = entry.relative_path.as_str();
    if path.is_empty()
        || path.len() > MAX_LIST_PATH_BYTES
        || path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || matches!(part.to_ascii_lowercase().as_str(), ".git" | ".polaris")
        })
    {
        return Err(SandboxError::PolicyDenied(
            "sync snapshot contains an unsafe relative path".to_owned(),
        ));
    }
    if let Some(target) = &entry.link_target {
        if entry.executable
            || entry.size
                != u64::try_from(target.len()).map_err(|_| {
                    SandboxError::LimitExceeded("sync snapshot link size overflow".to_owned())
                })?
        {
            return Err(SandboxError::Protocol(
                "sync snapshot link metadata is inconsistent".to_owned(),
            ));
        }
        validate_link_target(path, target)?;
        if Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes()) != entry.hash {
            return Err(SandboxError::Protocol(
                "sync snapshot link hash does not match its target".to_owned(),
            ));
        }
    } else if entry.size > 256 * 1024 * 1024 {
        return Err(SandboxError::LimitExceeded(
            "sync snapshot file exceeds the guest file limit".to_owned(),
        ));
    }
    Ok(())
}

fn validate_link_target(path: &str, target: &str) -> Result<(), SandboxError> {
    if target.is_empty()
        || target.len() > MAX_LIST_PATH_BYTES
        || target.starts_with('/')
        || target.contains(['\\', '\0'])
    {
        return Err(SandboxError::PolicyDenied(
            "sync snapshot contains an unsafe link target".to_owned(),
        ));
    }
    let mut depth = path.split('/').count().saturating_sub(1);
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." if depth > 0 => depth -= 1,
            ".." => {
                return Err(SandboxError::PolicyDenied(
                    "sync snapshot link escapes its workspace".to_owned(),
                ));
            }
            _ if part.len() <= 255 => depth += 1,
            _ => {
                return Err(SandboxError::PolicyDenied(
                    "sync snapshot link target component is too long".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn encode_commit_request(request: &SandboxCommitRequest) -> Result<Vec<u8>, SandboxError> {
    if request.root.len() > MAX_LIST_PATH_BYTES || request.relative_path.len() > MAX_LIST_PATH_BYTES
    {
        return Err(SandboxError::LimitExceeded(
            "sync-cas root or destination exceeds the path bound".to_owned(),
        ));
    }
    let staged_path = match &request.action {
        polaris_sdk::registry::SandboxCommitAction::File { staged_path, .. } => {
            Some(staged_path.as_str())
        }
        polaris_sdk::registry::SandboxCommitAction::Delete
        | polaris_sdk::registry::SandboxCommitAction::Link { .. } => None,
    };
    if staged_path.is_some_and(|path| path.len() > MAX_LIST_PATH_BYTES) {
        return Err(SandboxError::LimitExceeded(
            "sync-cas staged path exceeds the path bound".to_owned(),
        ));
    }
    if let polaris_sdk::registry::SandboxCommitAction::Link { target, .. } = &request.action
        && target.len() > MAX_LIST_PATH_BYTES
    {
        return Err(SandboxError::LimitExceeded(
            "sync-cas link target exceeds the path bound".to_owned(),
        ));
    }
    let encoded =
        serde_json::to_vec(request).map_err(|error| SandboxError::Protocol(error.to_string()))?;
    if encoded.len() > MAX_COMMIT_REQUEST_BYTES {
        return Err(SandboxError::LimitExceeded(format!(
            "sync-cas request exceeds {MAX_COMMIT_REQUEST_BYTES} bytes"
        )));
    }
    Ok(encoded)
}

fn to_wire_policy(
    document: &openshell_policy_schema::PolicyDocument,
) -> Result<proto::sandbox::v1::SandboxPolicy, SandboxError> {
    if !document.network_middlewares.is_empty() {
        return Err(SandboxError::PolicyDenied(
            "network middleware policy cannot be represented by the OpenShell SDK adapter"
                .to_owned(),
        ));
    }
    let filesystem =
        document
            .filesystem_policy
            .as_ref()
            .map(|value| proto::sandbox::v1::FilesystemPolicy {
                include_workdir: value.include_workdir,
                read_only: value.read_only.clone(),
                read_write: value.read_write.clone(),
            });
    let landlock = document
        .landlock
        .as_ref()
        .map(|value| proto::sandbox::v1::LandlockPolicy {
            compatibility: match value.compatibility {
                openshell_policy_schema::LandlockCompatibility::BestEffort => "best_effort",
                openshell_policy_schema::LandlockCompatibility::HardRequirement => {
                    "hard_requirement"
                }
            }
            .to_owned(),
        });
    let process = document
        .process
        .as_ref()
        .map(|value| proto::sandbox::v1::ProcessPolicy {
            run_as_user: value.run_as_user.clone(),
            run_as_group: value.run_as_group.clone(),
        });
    let network_policies = document
        .network_policies
        .iter()
        .map(|(name, value)| {
            let endpoints = value
                .endpoints
                .iter()
                .map(|endpoint| {
                    if !endpoint.ports.is_empty()
                        || !endpoint.tls.is_empty()
                        || !endpoint.allowed_ips.is_empty()
                        || !endpoint.rules.is_empty()
                        || !endpoint.deny_rules.is_empty()
                        || endpoint.allow_encoded_slash
                        || endpoint.websocket_credential_rewrite
                        || endpoint.request_body_credential_rewrite
                        || endpoint.allow_uninspected_credentials
                        || !endpoint.persisted_queries.is_empty()
                        || !endpoint.graphql_persisted_queries.is_empty()
                        || endpoint.graphql_max_body_bytes > 0
                        || !endpoint.credential_signing.is_empty()
                        || !endpoint.signing_service.is_empty()
                        || !endpoint.signing_region.is_empty()
                        || endpoint.credential_binding.is_some()
                        || endpoint.json_rpc.is_some()
                        || endpoint.mcp.is_some()
                    {
                        return Err(SandboxError::PolicyDenied(format!(
                            "endpoint '{}' uses a policy feature not supported by the adapter",
                            endpoint.host
                        )));
                    }
                    let enforcement = match endpoint.enforcement.as_str() {
                        "" => proto::sandbox::v1::NetworkEnforcementMode::Unspecified as i32,
                        "enforce" => proto::sandbox::v1::NetworkEnforcementMode::Enforce as i32,
                        "audit" => proto::sandbox::v1::NetworkEnforcementMode::Audit as i32,
                        other => {
                            return Err(SandboxError::PolicyDenied(format!(
                                "unsupported OpenShell enforcement mode: {other}"
                            )));
                        }
                    };
                    let access = match endpoint.access.as_str() {
                        "" => proto::sandbox::v1::NetworkAccessPreset::Unspecified as i32,
                        "read-only" => proto::sandbox::v1::NetworkAccessPreset::ReadOnly as i32,
                        "read-write" => proto::sandbox::v1::NetworkAccessPreset::ReadWrite as i32,
                        other => {
                            return Err(SandboxError::PolicyDenied(format!(
                                "unsupported OpenShell access mode: {other}"
                            )));
                        }
                    };
                    Ok(proto::sandbox::v1::NetworkEndpoint {
                        host: endpoint.host.clone(),
                        port: u32::from(endpoint.port),
                        protocol: endpoint.protocol.clone(),
                        enforcement,
                        access,
                        path: endpoint.path.clone(),
                        ..Default::default()
                    })
                })
                .collect::<Result<Vec<_>, SandboxError>>()?;
            let binaries = value
                .binaries
                .iter()
                .map(|binary| proto::sandbox::v1::NetworkBinary {
                    path: binary.path.clone(),
                })
                .collect();
            Ok((
                name.clone(),
                proto::sandbox::v1::NetworkPolicyRule {
                    name: value.name.clone(),
                    endpoints,
                    binaries,
                },
            ))
        })
        .collect::<Result<std::collections::HashMap<_, _>, SandboxError>>()?;
    Ok(proto::sandbox::v1::SandboxPolicy {
        version: document.version,
        filesystem,
        landlock,
        process,
        network_policies,
        network_middlewares: std::collections::HashMap::new(),
    })
}

fn openshell_recovery_domain(config: &OpenShellConfig, driver: &GatewayDriverInfo) -> Option<Cid> {
    let mut endpoint = url::Url::parse(&config.endpoint).ok()?;
    if endpoint.scheme() != "https" {
        return None;
    }
    let _username_cleared = endpoint.set_username("");
    let _password_cleared = endpoint.set_password(None);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let normalized_endpoint = format!(
        "{}{}",
        endpoint.origin().ascii_serialization(),
        endpoint.path().trim_end_matches('/')
    );
    let ca_fingerprint = blake3::hash(&config.tls.ca_certificate);
    let mut hasher = blake3::Hasher::new_derive_key("Polaris OpenShell sandbox recovery domain v1");
    for value in [
        normalized_endpoint.as_bytes(),
        config.tls.server_name.as_bytes(),
        ca_fingerprint.as_bytes(),
        driver.configured_name.as_bytes(),
        driver.driver_name.as_bytes(),
        driver.driver_version.as_bytes(),
    ] {
        let length = u64::try_from(value.len()).ok()?;
        hasher.update(&length.to_le_bytes());
        hasher.update(value);
    }
    Some(Cid::from_bytes(*hasher.finalize().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use polaris_openshell_policy::{AgentRole, ExecTier, policy_for};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn requested_output_limit_counts_both_streams() {
        assert!(check_combined_output(5, 4, 1, Some(10)).is_ok());
        assert!(check_combined_output(5, 4, 2, Some(10)).is_err());
        assert!(check_combined_output(0, 0, 1, Some(0)).is_err());
        assert!(check_combined_output(usize::MAX, 0, 1, None).is_err());
        assert!(check_combined_output(0, usize::MAX, 1, None).is_err());
    }

    #[test]
    fn scratch_quota_is_bounded_and_has_no_host_source() -> Result<(), Box<dyn std::error::Error>> {
        use prost_types::value::Kind;
        for bytes in [0, 1, 1024 * 1024 - 1, 64 * 1024 * 1024 + 1, u32::MAX] {
            assert!(ScratchQuota::new(bytes).is_err());
        }
        for bytes in [1024 * 1024, 64 * 1024 * 1024] {
            let quota = ScratchQuota::new(bytes)?;
            assert_eq!(quota.bytes(), bytes);
            let config = quota.driver_config();
            assert_eq!(config.fields.len(), 1);
            let Some(Kind::StructValue(docker)) = config
                .fields
                .get("docker")
                .and_then(|value| value.kind.as_ref())
            else {
                return Err("missing Docker configuration".into());
            };
            assert_eq!(docker.fields.len(), 1);
            let Some(Kind::ListValue(mounts)) = docker
                .fields
                .get("mounts")
                .and_then(|value| value.kind.as_ref())
            else {
                return Err("missing mount list".into());
            };
            assert_eq!(mounts.values.len(), 1);
            let Some(Kind::StructValue(mount)) =
                mounts.values.first().and_then(|value| value.kind.as_ref())
            else {
                return Err("missing mount object".into());
            };
            assert_eq!(mount.fields.len(), 4);
            assert!(!mount.fields.contains_key("source"));
            assert_eq!(
                mount
                    .fields
                    .get("target")
                    .and_then(|value| value.kind.as_ref()),
                Some(&Kind::StringValue(ScratchQuota::PATH.to_owned()))
            );
            assert_eq!(
                mount
                    .fields
                    .get("type")
                    .and_then(|value| value.kind.as_ref()),
                Some(&Kind::StringValue("tmpfs".to_owned()))
            );
            assert_eq!(
                mount
                    .fields
                    .get("size_bytes")
                    .and_then(|value| value.kind.as_ref()),
                Some(&Kind::NumberValue(f64::from(bytes)))
            );
        }
        Ok(())
    }

    #[test]
    fn quota_limits_use_the_gateway_struct_shape_and_units()
    -> Result<(), Box<dyn std::error::Error>> {
        let resources = sandbox_resource_limits(Some(1_000), Some(512))
            .ok_or("requested limits should be present")?;
        let Some(prost_types::value::Kind::StructValue(limits)) = resources
            .fields
            .get("limits")
            .and_then(|value| value.kind.as_ref())
        else {
            return Err("limits object should be present".into());
        };
        assert_eq!(
            limits
                .fields
                .get("cpu")
                .and_then(|value| value.kind.as_ref()),
            Some(&prost_types::value::Kind::StringValue("1000m".to_owned()))
        );
        assert_eq!(
            limits
                .fields
                .get("memory")
                .and_then(|value| value.kind.as_ref()),
            Some(&prost_types::value::Kind::StringValue("512Mi".to_owned()))
        );
        assert!(sandbox_resource_limits(None, None).is_none());
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ConnectTestError {
        Transient,
        Permanent,
    }

    #[test]
    fn catalog_resolves_exact_content_and_missing_cids_fail()
    -> Result<(), Box<dyn std::error::Error>> {
        let catalog = PolicyCatalog::default();
        let policy = policy_for(AgentRole::Builder, ExecTier::Docker)
            .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        let cid = catalog.register(&policy)?;
        let bytes = catalog.resolve(&cid)?;
        assert_eq!(Cid::from_bytes(*blake3::hash(&bytes).as_bytes()), cid);
        assert!(catalog.resolve(&Cid::from_bytes([7; 32])).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn connection_retry_is_bounded_and_skips_permanent_errors() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempt_counter = Arc::clone(&attempts);
        let result = retry_transient_connect(
            Duration::from_millis(350),
            move || {
                attempt_counter.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(ConnectTestError::Transient) }
            },
            |error| *error == ConnectTestError::Transient,
        )
        .await;
        assert!(matches!(result, Err(ConnectRetryError::Deadline)));
        assert!(attempts.load(Ordering::SeqCst) >= 2);

        let permanent_attempts = Arc::new(AtomicUsize::new(0));
        let attempt_counter = Arc::clone(&permanent_attempts);
        let result = retry_transient_connect(
            Duration::from_secs(1),
            move || {
                attempt_counter.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(ConnectTestError::Permanent) }
            },
            |error| *error == ConnectTestError::Transient,
        )
        .await;
        assert!(matches!(
            result,
            Err(ConnectRetryError::Failed(ConnectTestError::Permanent))
        ));
        assert_eq!(permanent_attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dropping_connection_retry_cancels_the_inflight_attempt()
    -> Result<(), Box<dyn std::error::Error>> {
        struct DropMarker(Arc<AtomicBool>);
        impl Drop for DropMarker {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&dropped);
        let mut retry = Box::pin(retry_transient_connect(
            Duration::from_secs(5),
            move || {
                let marker = Arc::clone(&marker);
                async move {
                    let _drop_marker = DropMarker(marker);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Err::<(), _>(ConnectTestError::Transient)
                }
            },
            |_| true,
        ));
        tokio::select! {
            _ = &mut retry => return Err("in-flight connection attempt unexpectedly completed".into()),
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        drop(retry);
        assert!(dropped.load(Ordering::SeqCst));
        Ok(())
    }

    #[test]
    fn only_refused_or_disconnected_sockets_are_retryable() {
        let refused = std::io::Error::new(ErrorKind::ConnectionRefused, "gateway starting");
        let disconnected = std::io::Error::new(ErrorKind::NotConnected, "gateway disconnected");
        let invalid_certificate =
            std::io::Error::new(ErrorKind::InvalidData, "invalid certificate");
        assert!(is_transient_connect_error(&refused));
        assert!(is_transient_connect_error(&disconnected));
        assert!(!is_transient_connect_error(&invalid_certificate));
    }

    #[test]
    fn duplicate_sandbox_error_is_typed_for_cleanup_ownership() {
        assert_eq!(
            map_create_error(tonic::Status::already_exists("sandbox name already exists")),
            SandboxError::AlreadyExists("sandbox name already exists".to_owned())
        );
        assert!(matches!(
            map_create_error(tonic::Status::permission_denied("denied")),
            SandboxError::Unavailable(_)
        ));
    }

    #[test]
    fn gateway_driver_identity_must_match_the_selected_backend() {
        let docker = GatewayDriverInfo {
            configured_name: "default".to_owned(),
            driver_name: "docker".to_owned(),
            driver_version: "test".to_owned(),
        };
        assert!(validate_gateway_driver_kind(&docker, DriverKind::Docker).is_ok());
        assert!(validate_gateway_driver_kind(&docker, DriverKind::Vm).is_err());
    }

    #[test]
    fn recovery_domain_normalizes_and_binds_the_trusted_backend_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = OpenShellConfig::wsl2(GatewayTls {
            ca_certificate: b"ca-one".to_vec(),
            client_certificate: Vec::new(),
            client_key: Vec::new(),
            server_name: "gateway.local".to_owned(),
        });
        let driver = GatewayDriverInfo {
            configured_name: "default".to_owned(),
            driver_name: "docker".to_owned(),
            driver_version: "1".to_owned(),
        };
        let domain = openshell_recovery_domain(&config, &driver)
            .ok_or("valid TLS endpoint must have a recovery domain")?;
        let mut normalized = config.clone();
        normalized.endpoint =
            "https://user:secret@LOCALHOST:17670/?token=ignored#fragment".to_owned();
        assert_eq!(
            openshell_recovery_domain(&normalized, &driver),
            Some(domain)
        );

        let mut implicit_https_port = config.clone();
        implicit_https_port.endpoint = "https://localhost/".to_owned();
        let mut explicit_https_port = implicit_https_port.clone();
        explicit_https_port.endpoint = "HTTPS://user:secret@LOCALHOST:443".to_owned();
        let implicit_port_domain = openshell_recovery_domain(&implicit_https_port, &driver)
            .ok_or("valid HTTPS endpoint must have a recovery domain")?;
        assert_eq!(
            openshell_recovery_domain(&explicit_https_port, &driver),
            Some(implicit_port_domain)
        );
        assert_eq!(implicit_port_domain.to_hex().len(), 64);
        assert!(!implicit_port_domain.to_hex().contains("secret"));

        let mut other_endpoint = config.clone();
        other_endpoint.endpoint = "https://localhost:17671".to_owned();
        assert_ne!(
            openshell_recovery_domain(&other_endpoint, &driver),
            Some(domain)
        );
        let mut other_ca = config.clone();
        other_ca.tls.ca_certificate = b"ca-two".to_vec();
        assert_ne!(openshell_recovery_domain(&other_ca, &driver), Some(domain));
        let mut other_server_name = config.clone();
        other_server_name.tls.server_name = "other.local".to_owned();
        assert_ne!(
            openshell_recovery_domain(&other_server_name, &driver),
            Some(domain)
        );
        let mut other_driver = driver.clone();
        other_driver.driver_version = "2".to_owned();
        assert_ne!(
            openshell_recovery_domain(&config, &other_driver),
            Some(domain)
        );
        let mut insecure = config;
        insecure.endpoint = "http://localhost:17670".to_owned();
        assert!(openshell_recovery_domain(&insecure, &driver).is_none());
        Ok(())
    }

    #[test]
    fn delete_acknowledgements_require_a_terminal_or_pollable_target() {
        let original_id = "sandbox-id".to_owned();
        assert_eq!(
            validate_delete_ack(openshell_sdk::DeletionResult {
                outcome: DeletionOutcome::Accepted,
                sandbox_id: Some(original_id.clone()),
            })
            .ok(),
            Some(Some(original_id.clone()))
        );
        assert_eq!(
            validate_delete_ack(openshell_sdk::DeletionResult {
                outcome: DeletionOutcome::AlreadyAbsent,
                sandbox_id: None,
            })
            .ok(),
            Some(None)
        );
        assert!(
            validate_delete_ack(openshell_sdk::DeletionResult {
                outcome: DeletionOutcome::Accepted,
                sandbox_id: None,
            })
            .is_err()
        );
        assert!(
            validate_delete_ack(openshell_sdk::DeletionResult {
                outcome: DeletionOutcome::Unspecified,
                sandbox_id: Some(original_id),
            })
            .is_err()
        );
    }

    #[test]
    fn request_and_output_bounds_fail_closed() {
        assert!(extend_bounded(&mut vec![0; MAX_EXEC_OUTPUT_BYTES], &[1]).is_err());
        assert!(validate_path("/sandbox/../etc/passwd").is_err());
        assert!(validate_path("relative").is_err());
        assert!(validate_path("/sandbox/file").is_ok());
        const { assert!(MAX_REQUEST_CHUNK_BYTES < 1_048_576) };
    }

    #[test]
    fn file_listing_checks_each_path_before_allocation() {
        assert_eq!(
            parse_file_list("/sandbox/a\n/sandbox/b\n").ok(),
            Some(vec!["/sandbox/a".to_owned(), "/sandbox/b".to_owned()])
        );
        assert!(parse_file_list("relative\n").is_err());
        let long_path = format!("/{}", "x".repeat(MAX_LIST_PATH_BYTES));
        assert!(parse_file_list(&long_path).is_err());
    }

    #[test]
    fn sync_snapshot_response_is_bounded_and_validated() -> Result<(), Box<dyn std::error::Error>> {
        let page = SandboxSnapshotPage {
            generation: "a".repeat(64),
            manifest_id: Cid::from_bytes([3; 32]),
            total_entries: 2,
            offset: 0,
            entries: vec![polaris_sdk::registry::SandboxSnapshotEntry {
                relative_path: "src/main.rs".to_owned(),
                size: 7,
                hash: Cid::from_bytes(*blake3::hash(b"payload").as_bytes()),
                executable: false,
                link_target: None,
            }],
            next_offset: Some(1),
        };
        let encoded = serde_json::to_vec(&page)?;
        assert!(parse_snapshot_page(&encoded, None, 0, 1).is_ok());
        assert!(parse_snapshot_page(&encoded, Some(&"b".repeat(64)), 0, 1).is_err());
        assert!(parse_snapshot_page(&encoded, None, 0, 0).is_err());
        assert!(parse_snapshot_page(&vec![b' '; MAX_EXEC_OUTPUT_BYTES + 1], None, 0, 1).is_err());
        Ok(())
    }

    #[test]
    fn sync_snapshot_rejects_unsafe_paths_and_link_hashes() {
        let invalid_path = polaris_sdk::registry::SandboxSnapshotEntry {
            relative_path: "../outside".to_owned(),
            size: 0,
            hash: Cid::from_bytes([0; 32]),
            executable: false,
            link_target: None,
        };
        assert!(validate_snapshot_entry(&invalid_path).is_err());

        let escaping_link = polaris_sdk::registry::SandboxSnapshotEntry {
            relative_path: "src/link".to_owned(),
            size: 9,
            hash: Cid::from_bytes(*blake3::hash(b"../../out").as_bytes()),
            executable: false,
            link_target: Some("../../out".to_owned()),
        };
        assert!(validate_snapshot_entry(&escaping_link).is_err());

        let wrong_hash = polaris_sdk::registry::SandboxSnapshotEntry {
            relative_path: "src/link".to_owned(),
            size: 6,
            hash: Cid::from_bytes([7; 32]),
            executable: false,
            link_target: Some("target".to_owned()),
        };
        assert!(validate_snapshot_entry(&wrong_hash).is_err());
    }

    #[test]
    fn sync_commit_request_is_bounded_before_exec() {
        let request = SandboxCommitRequest {
            root: "/workspace".to_owned(),
            relative_path: "src/main.rs".to_owned(),
            expected: None,
            action: polaris_sdk::registry::SandboxCommitAction::Delete,
        };
        assert!(encode_commit_request(&request).is_ok());
        let oversized = SandboxCommitRequest {
            relative_path: "a".repeat(MAX_LIST_PATH_BYTES + 1),
            ..request
        };
        assert!(matches!(
            encode_commit_request(&oversized),
            Err(SandboxError::LimitExceeded(_))
        ));
    }

    #[test]
    fn gateway_names_fail_before_remote_create_when_invalid() {
        assert!(validate_gateway_name("p2-0123456789abcdef").is_ok());
        assert!(validate_gateway_name("p2-0123456789abcdef0").is_err());
        assert!(validate_gateway_name("bad/name").is_err());
    }

    #[test]
    fn image_references_require_immutable_sha256_digests() {
        assert!(validate_image_reference(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(
            validate_image_reference(&format!(
                "nvcr.io/polaris/sandbox@sha256:{}",
                "b".repeat(64)
            ))
            .is_ok()
        );
        assert!(validate_image_reference("polaris-sandbox:latest").is_err());
        assert!(validate_image_reference("sha256:abc").is_err());
    }
}
