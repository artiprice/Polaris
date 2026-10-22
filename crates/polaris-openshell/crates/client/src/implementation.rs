//! Bounded OpenShell gRPC adapter.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use openshell_sdk::{EdgeAuthInterceptor, OpenShellClient};
use openshell_sdk::raw::{self, proto};
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use polaris_openshell_policy::Policy;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

/// Maximum gRPC request payload used by OpenShell v0.1.4's gateway.
pub const MAX_REQUEST_CHUNK_BYTES: usize = 896 * 1024;
/// Maximum stdout/stderr bytes retained for a single execution.
pub const MAX_EXEC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum bytes returned by one download call.
pub const MAX_DOWNLOAD_BYTES: usize = MAX_EXEC_OUTPUT_BYTES;
const GATEWAY_VERSION: &str = "0.1.4";

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
        let bytes = Bytes::from(policy.canonical_yaml().map_err(|error| SandboxError::PolicyDenied(error.to_string()))?);
        let cid = Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
        let mut policies = self.policies.write().map_err(|_| SandboxError::Unavailable("policy catalog lock poisoned".to_owned()))?;
        let old_len = match policies.get(&cid) { Some(old) => old.len(), None => 0 };
        let total = policies.values().try_fold(bytes.len().saturating_sub(old_len), |sum, value| sum.checked_add(value.len()))
            .ok_or_else(|| SandboxError::LimitExceeded("policy catalog size overflow".to_owned()))?;
        if total > MAX_CATALOG_BYTES || (old_len == 0 && policies.len() >= MAX_CATALOG_POLICIES) {
            return Err(SandboxError::LimitExceeded("policy catalog capacity exceeded".to_owned()));
        }
        policies.insert(cid, bytes);
        Ok(cid)
    }
}

impl PolicySource for PolicyCatalog {
    fn resolve(&self, cid: &Cid) -> Result<Bytes, SandboxError> {
        self.policies.read()
            .map_err(|_| SandboxError::Unavailable("policy catalog lock poisoned".to_owned()))?
            .get(cid).cloned().ok_or_else(|| SandboxError::NotFound(format!("policy CID {cid}")))
    }
}

/// OpenShell backend. The policy resolver is generic for direct CAS injection by the composition root.
pub struct OpenShellBackend<P = PolicyCatalog> {
    client: Arc<OpenShellClient>,
    policies: P,
    config: OpenShellConfig,
}

impl<P: std::fmt::Debug> std::fmt::Debug for OpenShellBackend<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OpenShellBackend").field("config", &self.config).finish_non_exhaustive()
    }
}

impl OpenShellBackend<PolicyCatalog> {
    /// Connect to OpenShell with a local catalog for policies registered through this backend.
    pub async fn connect(config: OpenShellConfig) -> Result<Self, SandboxError> {
        Self::connect_with_policies(config, PolicyCatalog::default()).await
    }
}

impl<P: PolicySource> OpenShellBackend<P> {
    /// Connect and fail closed unless the gateway reports the configured version.
    pub async fn connect_with_policies(config: OpenShellConfig, policies: P) -> Result<Self, SandboxError> {
        if !config.endpoint.starts_with("https://") {
            return Err(SandboxError::PolicyDenied("OpenShell gateways require TLS".to_owned()));
        }
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(config.tls.ca_certificate.clone()))
            .identity(Identity::from_pem(config.tls.client_certificate.clone(), config.tls.client_key.clone()))
            .domain_name(config.tls.server_name.clone());
        let endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|error| SandboxError::Protocol(error.to_string()))?
            .connect_timeout(Duration::from_secs(10))
            .tls_config(tls)
            .map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let channel = endpoint.connect().await.map_err(|error| SandboxError::Unavailable(error.to_string()))?;
        let client = OpenShellClient::from_parts(channel, EdgeAuthInterceptor::noop());
        let health = client.health().await.map_err(|error| SandboxError::Unavailable(error.to_string()))?;
        if health.version != config.expected_version {
            return Err(SandboxError::Protocol(format!("OpenShell version mismatch: expected {}, found {}", config.expected_version, health.version)));
        }
        Ok(Self { client: Arc::new(client), policies, config })
    }

    /// Return the configured gateway driver.
    pub fn driver(&self) -> DriverKind { self.config.driver }

    /// Register a policy in the local catalog and return the CID to place in `SandboxSpec`.
    pub fn register_policy(&self, policy: &Policy) -> Result<Cid, SandboxError>
    where P: PolicySource + CatalogAccess {
        self.policies.register(policy)
    }

    /// Gateway health/version already checked at connect time.
    pub async fn health(&self) -> Result<String, SandboxError> {
        self.client.health().await.map(|health| health.version).map_err(|error| SandboxError::Unavailable(error.to_string()))
    }

    async fn create_raw(&self, spec: &SandboxSpec, bytes: Bytes, policy: Policy, providers: &[String]) -> Result<SandboxHandle, SandboxError> {
        let content_hash = blake3::hash(&bytes);
        if spec.policy != Some(Cid::from_bytes(*content_hash.as_bytes())) {
            return Err(SandboxError::PolicyDenied("policy CID does not match resolved policy bytes".to_owned()));
        }
        let wire_policy = to_wire_policy(policy.document())?;
        let template = spec.image.as_ref().map(|image| proto::SandboxTemplate { image: image.clone(), ..Default::default() });
        let request = raw::CreateSandboxRequest {
            request_id: String::new(),
            spec: Some(raw::ProtoSandboxSpec { policy: Some(wire_policy), template, providers: providers.to_vec(), ..Default::default() }),
            name: match &spec.name { Some(name) => name.clone(), None => String::new() },
            workspace_scope: Some(proto::workspace_selector("default")),
            ..Default::default()
        };
        let response = self.client.raw_grpc().create_sandbox(request).await
            .map_err(|error| SandboxError::Unavailable(error.to_string()))?.into_inner();
        let Some(sandbox) = response.sandbox else {
            return Err(SandboxError::Protocol("create sandbox response omitted sandbox".to_owned()));
        };
        let Some(metadata) = sandbox.metadata else {
            return Err(SandboxError::Protocol("create sandbox response omitted metadata".to_owned()));
        };
        let name = metadata.name;
        if name.is_empty() { return Err(SandboxError::Protocol("gateway returned empty sandbox name".to_owned())); }
        let mut cleanup = SandboxCleanupGuard::new(Arc::clone(&self.client), name.clone());
        let ready = match self.client.wait_ready(&name, self.config.ready_timeout).await {
            Ok(ready) => ready,
            Err(error) => return Err(SandboxError::Unavailable(format!("sandbox readiness failed; cleanup scheduled: {error}"))),
        };
        let Some(handle) = SandboxHandle::new(&ready.name).ok() else {
            return Err(SandboxError::Protocol("gateway returned invalid sandbox name".to_owned()));
        };
        cleanup.disarm();
        Ok(handle)
    }

    /// Create a sandbox with explicit OpenShell provider profile references.
    pub async fn create_with_profiles(&self, spec: &SandboxSpec, providers: &[String]) -> Result<SandboxHandle, SandboxError> {
        let Some(cid) = spec.policy else { return Err(SandboxError::PolicyDenied("SandboxSpec.policy is required".to_owned())); };
        let bytes = self.policies.resolve(&cid)?;
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != cid {
            return Err(SandboxError::PolicyDenied("resolved policy content does not match its CID".to_owned()));
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let policy = Policy::parse_yaml(text).map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        if spec.cpu_millis.is_some() || spec.memory_mib.is_some() {
            return Err(SandboxError::Protocol("OpenShell v0.1.4 SDK adapter cannot apply per-sandbox CPU/memory limits".to_owned()));
        }
        self.create_raw(spec, bytes, policy, providers).await
    }

    async fn exec_bounded(&self, sandbox: &SandboxHandle, request: &ExecRequest) -> Result<ExecOutput, SandboxError> {
        if request.argv.is_empty() { return Err(SandboxError::Protocol("exec argv must not be empty".to_owned())); }
        let mut environment = std::collections::HashMap::new();
        for (key, value) in &request.env {
            if key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0') {
                return Err(SandboxError::Protocol("invalid environment entry".to_owned()));
            }
            environment.insert(key.clone(), value.clone());
        }
        let stdin = match request.stdin.as_ref() {
            Some(bytes) if bytes.len() <= MAX_REQUEST_CHUNK_BYTES => bytes.to_vec(),
            Some(_) => return Err(SandboxError::LimitExceeded(format!("exec stdin exceeds {MAX_REQUEST_CHUNK_BYTES} bytes"))),
            None => Vec::new(),
        };
        let execution_id = Uuid::now_v7().to_string();
        let pid_file = format!("/tmp/polaris-exec-{execution_id}.pid");
        let mut command = vec![
            "sh".to_owned(), "-c".to_owned(),
            "pid_file=$1; shift; setsid -- \"$@\" & child=$!; printf '%s\\n' \"$child\" > \"$pid_file\"; wait \"$child\"; result=$?; rm -f -- \"$pid_file\"; exit \"$result\"".to_owned(),
            "polaris-exec".to_owned(), pid_file.clone(),
        ];
        command.extend(request.argv.iter().cloned());
        let rpc = raw::ExecSandboxRequest {
            request_id: String::new(), sandbox: String::from(sandbox.clone()),
            workspace_scope: Some(proto::workspace_selector("default")), command,
            workdir: match &request.workdir { Some(path) => path.clone(), None => String::new() }, environment,
            execution_timeout: None, stdin, tty: false, cols: 0, rows: 0, no_login_shell: true,
        };
        let mut cleanup = ExecCleanupGuard::new(Arc::clone(&self.client), String::from(sandbox.clone()), pid_file);
        let started = Instant::now();
        let call = async {
            use futures::StreamExt;
            let mut stream = self.client.raw_grpc().exec_sandbox(rpc).await
                .map_err(|error| SandboxError::Unavailable(error.to_string()))?.into_inner();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_code = None;
            while let Some(event) = stream.next().await {
                let event = event.map_err(|error| SandboxError::Unavailable(error.to_string()))?;
                match event.payload {
                    Some(proto::exec_sandbox_event::Payload::Stdout(chunk)) => extend_bounded(&mut stdout, &chunk.data)?,
                    Some(proto::exec_sandbox_event::Payload::Stderr(chunk)) => extend_bounded(&mut stderr, &chunk.data)?,
                    Some(proto::exec_sandbox_event::Payload::Exit(exit)) => exit_code = Some(exit.exit_code),
                    None => {}
                }
            }
            let Some(exit_code) = exit_code else { return Err(SandboxError::Protocol("exec stream ended without an exit event".to_owned())); };
            Ok(ExecOutput { exit_code, stdout: Bytes::from(stdout), stderr: Bytes::from(stderr), duration: started.elapsed() })
        };
        let timeout = match request.timeout { Some(value) => value, None => self.config.exec_timeout };
        let result = match tokio::time::timeout(timeout, call).await {
            Ok(result) => result,
            Err(_) => Err(SandboxError::Timeout),
        };
        if result.is_ok() { cleanup.disarm(); }
        result
    }

    async fn shell_transfer(&self, sandbox: &SandboxHandle, command: Vec<String>, stdin: Option<Bytes>, cap: usize) -> Result<Bytes, SandboxError> {
        let req = ExecRequest { argv: command, stdin, timeout: Some(self.config.exec_timeout), ..Default::default() };
        let out = self.exec_bounded(sandbox, &req).await?;
        if out.exit_code != 0 { return Err(SandboxError::Protocol(format!("sandbox transfer command exited {}", out.exit_code))); }
        if out.stdout.len() > cap { return Err(SandboxError::LimitExceeded(format!("sandbox transfer output exceeds {cap} bytes"))); }
        Ok(out.stdout)
    }
}

/// Marker for a policy source that also exposes a concrete catalog registration method.
pub trait CatalogAccess: PolicySource {
    /// Register canonical policy bytes and return their CID.
    fn register(&self, policy: &Policy) -> Result<Cid, SandboxError>;
}

impl CatalogAccess for PolicyCatalog {
    fn register(&self, policy: &Policy) -> Result<Cid, SandboxError> { PolicyCatalog::register(self, policy) }
}

impl<P: PolicySource> SandboxBackend for OpenShellBackend<P> {
    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let Some(cid) = spec.policy else { return Err(SandboxError::PolicyDenied("SandboxSpec.policy is required".to_owned())); };
        let bytes = self.policies.resolve(&cid)?;
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != cid {
            return Err(SandboxError::PolicyDenied("resolved policy content does not match its CID".to_owned()));
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| SandboxError::Protocol(error.to_string()))?;
        let policy = Policy::parse_yaml(text).map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
        if spec.cpu_millis.is_some() || spec.memory_mib.is_some() {
            return Err(SandboxError::Protocol("OpenShell v0.1.4 SDK adapter cannot apply per-sandbox CPU/memory limits".to_owned()));
        }
        self.create_raw(spec, bytes, policy, &[]).await
    }

    async fn exec(&self, sandbox: &SandboxHandle, req: &ExecRequest) -> Result<ExecOutput, SandboxError> {
        self.exec_bounded(sandbox, req).await
    }

    async fn upload(&self, sandbox: &SandboxHandle, path: &str, bytes: Bytes) -> Result<(), SandboxError> {
        validate_path(path)?;
        for (index, chunk) in bytes.chunks(MAX_REQUEST_CHUNK_BYTES).enumerate() {
            let append = if index == 0 { ">" } else { ">>" };
            let command = vec!["sh".to_owned(), "-c".to_owned(), format!("umask 077; cat {append} \"$1\"",), "polaris-upload".to_owned(), path.to_owned()];
            let request = ExecRequest { argv: command, stdin: Some(Bytes::copy_from_slice(chunk)), timeout: Some(self.config.exec_timeout), ..Default::default() };
            let result = self.exec_bounded(sandbox, &request).await?;
            if result.exit_code != 0 { return Err(SandboxError::Protocol(format!("upload command exited {}", result.exit_code))); }
        }
        Ok(())
    }

    async fn download(&self, sandbox: &SandboxHandle, path: &str) -> Result<Bytes, SandboxError> {
        validate_path(path)?;
        let command = vec!["sh".to_owned(), "-c".to_owned(), "cat -- \"$1\"".to_owned(), "polaris-download".to_owned(), path.to_owned()];
        self.shell_transfer(sandbox, command, None, MAX_DOWNLOAD_BYTES).await
    }

    async fn list_files(&self, sandbox: &SandboxHandle, dir: &str) -> Result<Vec<String>, SandboxError> {
        validate_path(dir)?;
        let command = vec!["find".to_owned(), dir.to_owned(), "-type".to_owned(), "f".to_owned(), "-print".to_owned()];
        let output = self.shell_transfer(sandbox, command, None, MAX_EXEC_OUTPUT_BYTES).await?;
        let text = std::str::from_utf8(&output).map_err(|error| SandboxError::Protocol(error.to_string()))?;
        Ok(text.lines().map(str::to_owned).collect())
    }

    async fn destroy(&self, sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        let sandbox_name = String::from(sandbox.clone());
        match self.client.delete_sandbox(&sandbox_name, Default::default()).await {
            Ok(_) => Ok(()),
            Err(error) if error.to_string().to_ascii_lowercase().contains("not found") => Ok(()),
            Err(error) => Err(SandboxError::Unavailable(error.to_string())),
        }
    }
}

struct SandboxCleanupGuard {
    client: Arc<OpenShellClient>,
    name: String,
    armed: bool,
}

impl SandboxCleanupGuard {
    fn new(client: Arc<OpenShellClient>, name: String) -> Self { Self { client, name, armed: true } }
    fn disarm(&mut self) { self.armed = false; }
}

impl Drop for SandboxCleanupGuard {
    fn drop(&mut self) {
        if !self.armed { return; }
        let client = Arc::clone(&self.client);
        let name = self.name.clone();
        let cleanup = async move {
            if let Err(error) = client.delete_sandbox(&name, Default::default()).await {
                tracing::error!(sandbox = %name, %error, "failed to clean up sandbox after cancelled create");
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => { drop(runtime.spawn(cleanup)); }
            Err(error) => tracing::error!(sandbox = %self.name, %error, "cannot schedule sandbox cleanup without a Tokio runtime"),
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
        Self { client, sandbox, pid_file, armed: true }
    }
    fn disarm(&mut self) { self.armed = false; }
}

impl Drop for ExecCleanupGuard {
    fn drop(&mut self) {
        if !self.armed { return; }
        let client = Arc::clone(&self.client);
        let sandbox = self.sandbox.clone();
        let pid_file = self.pid_file.clone();
        let cleanup = async move {
            let script = "i=0; while [ ! -s \"$1\" ] && [ $i -lt 20 ]; do sleep 0.05; i=$((i+1)); done; if [ -s \"$1\" ]; then read -r pid < \"$1\"; kill -TERM -- \"-$pid\"; rm -f -- \"$1\"; fi";
            let command = vec!["sh".to_owned(), "-c".to_owned(), script.to_owned(), "polaris-cancel".to_owned(), pid_file];
            let result = client.exec(&sandbox, &command, openshell_sdk::ExecOptions { no_login_shell: true, ..Default::default() }).await;
            match result {
                Ok(output) if output.exit_code == 0 => {}
                Ok(output) => tracing::error!(sandbox = %sandbox, exit_code = output.exit_code, "remote exec cleanup exited unsuccessfully"),
                Err(error) => tracing::error!(sandbox = %sandbox, %error, "failed to stop remote exec after cancellation or output limit"),
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => { drop(runtime.spawn(cleanup)); }
            Err(error) => tracing::error!(sandbox = %self.sandbox, %error, "cannot schedule remote exec cleanup without a Tokio runtime"),
        }
    }
}

fn extend_bounded(target: &mut Vec<u8>, chunk: &[u8]) -> Result<(), SandboxError> {
    let new_len = target.len().checked_add(chunk.len()).ok_or_else(|| SandboxError::LimitExceeded("exec output length overflow".to_owned()))?;
    if new_len > MAX_EXEC_OUTPUT_BYTES { return Err(SandboxError::LimitExceeded(format!("exec output exceeds {MAX_EXEC_OUTPUT_BYTES} bytes"))); }
    target.extend_from_slice(chunk);
    Ok(())
}

fn validate_path(path: &str) -> Result<(), SandboxError> {
    if !path.starts_with('/') || path.len() > 4_096 || path.contains('\0') || path.split('/').any(|part| part == "..") {
        return Err(SandboxError::PolicyDenied(format!("invalid sandbox absolute path: {path}")));
    }
    Ok(())
}

fn to_wire_policy(document: &openshell_policy_schema::PolicyDocument) -> Result<proto::sandbox::v1::SandboxPolicy, SandboxError> {
    if !document.network_middlewares.is_empty() {
        return Err(SandboxError::PolicyDenied("network middleware policy cannot be represented by the OpenShell SDK adapter".to_owned()));
    }
    let filesystem = document.filesystem_policy.as_ref().map(|value| proto::sandbox::v1::FilesystemPolicy {
        include_workdir: value.include_workdir, read_only: value.read_only.clone(), read_write: value.read_write.clone(),
    });
    let landlock = document.landlock.as_ref().map(|value| proto::sandbox::v1::LandlockPolicy {
        compatibility: match value.compatibility { openshell_policy_schema::LandlockCompatibility::BestEffort => "best_effort", openshell_policy_schema::LandlockCompatibility::HardRequirement => "hard_requirement" }.to_owned(),
    });
    let process = document.process.as_ref().map(|value| proto::sandbox::v1::ProcessPolicy {
        run_as_user: value.run_as_user.clone(), run_as_group: value.run_as_group.clone(),
    });
    let network_policies = document.network_policies.iter().map(|(name, value)| {
        let endpoints = value.endpoints.iter().map(|endpoint| {
            if !endpoint.ports.is_empty() || !endpoint.tls.is_empty() || !endpoint.allowed_ips.is_empty()
                || !endpoint.rules.is_empty() || !endpoint.deny_rules.is_empty()
                || endpoint.allow_encoded_slash || endpoint.websocket_credential_rewrite
                || endpoint.request_body_credential_rewrite || endpoint.allow_uninspected_credentials
                || !endpoint.persisted_queries.is_empty() || !endpoint.graphql_persisted_queries.is_empty()
                || endpoint.graphql_max_body_bytes > 0 || !endpoint.credential_signing.is_empty()
                || !endpoint.signing_service.is_empty() || !endpoint.signing_region.is_empty()
                || endpoint.credential_binding.is_some() || endpoint.json_rpc.is_some() || endpoint.mcp.is_some()
            {
                return Err(SandboxError::PolicyDenied(format!("endpoint '{}' uses a policy feature not supported by the adapter", endpoint.host)));
            }
            let enforcement = match endpoint.enforcement.as_str() {
                "" => proto::sandbox::v1::NetworkEnforcementMode::Unspecified as i32,
                "enforce" => proto::sandbox::v1::NetworkEnforcementMode::Enforce as i32,
                "audit" => proto::sandbox::v1::NetworkEnforcementMode::Audit as i32,
                other => return Err(SandboxError::PolicyDenied(format!("unsupported OpenShell enforcement mode: {other}"))),
            };
            let access = match endpoint.access.as_str() {
                "" => proto::sandbox::v1::NetworkAccessPreset::Unspecified as i32,
                "read-only" => proto::sandbox::v1::NetworkAccessPreset::ReadOnly as i32,
                "read-write" => proto::sandbox::v1::NetworkAccessPreset::ReadWrite as i32,
                other => return Err(SandboxError::PolicyDenied(format!("unsupported OpenShell access mode: {other}"))),
            };
            Ok(proto::sandbox::v1::NetworkEndpoint {
                host: endpoint.host.clone(), port: u32::from(endpoint.port), protocol: endpoint.protocol.clone(),
                enforcement, access, path: endpoint.path.clone(), ..Default::default()
            })
        }).collect::<Result<Vec<_>, SandboxError>>()?;
        let binaries = value.binaries.iter().map(|binary| proto::sandbox::v1::NetworkBinary { path: binary.path.clone() }).collect();
        Ok((name.clone(), proto::sandbox::v1::NetworkPolicyRule { name: value.name.clone(), endpoints, binaries }))
    }).collect::<Result<std::collections::HashMap<_, _>, SandboxError>>()?;
    Ok(proto::sandbox::v1::SandboxPolicy { version: document.version, filesystem, landlock, process, network_policies, network_middlewares: std::collections::HashMap::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use polaris_openshell_policy::{AgentRole, ExecTier, policy_for};

    #[test]
    fn catalog_resolves_exact_content_and_missing_cids_fail() {
        let catalog = PolicyCatalog::default();
        let Some(policy) = policy_for(AgentRole::Builder, ExecTier::Docker).ok() else { panic!("policy expected") };
        let Some(cid) = catalog.register(&policy).ok() else { panic!("catalog registration expected") };
        let Some(bytes) = catalog.resolve(&cid).ok() else { panic!("CID must resolve") };
        assert_eq!(Cid::from_bytes(*blake3::hash(&bytes).as_bytes()), cid);
        assert!(catalog.resolve(&Cid::from_bytes([7; 32])).is_err());
    }

    #[test]
    fn request_and_output_bounds_fail_closed() {
        assert!(extend_bounded(&mut vec![0; MAX_EXEC_OUTPUT_BYTES], &[1]).is_err());
        assert!(validate_path("/sandbox/../etc/passwd").is_err());
        assert!(validate_path("relative").is_err());
        assert!(validate_path("/sandbox/file").is_ok());
        assert!(MAX_REQUEST_CHUNK_BYTES < 1_048_576);
    }
}
