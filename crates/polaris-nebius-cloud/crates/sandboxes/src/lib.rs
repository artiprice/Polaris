//! # polaris-nebius-cloud-sandboxes
//!
//! Stateless typed adapter for the official Nebius `contree-mcp` tool contract. Each command
//! creates a separate microVM and immutable image lineage is carried by image UUIDs; this crate
//! deliberately does not implement the SDK's persistent `SandboxBackend` lifecycle.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    path::Path,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use futures::future::{join_all, try_join_all};
use polaris_sdk::registry::{McpToolClient, McpToolClientError, McpToolDefinition};
use polaris_storage_sync::{EntryKind, Exclusion, IgnoreRules, Manifest, RelPath, SyncPlan};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Upstream source revision checked for the advertised tool inventory and Python call signatures.
pub const UPSTREAM_SOURCE_REVISION: &str = "c2f1541e3993bc77c3c7dbcf57b0edfe5b3634cc";

/// Exact tools registered by the upstream `create_mcp_app` at [`UPSTREAM_SOURCE_REVISION`].
pub const UPSTREAM_TOOL_NAMES: [&str; 19] = [
    "cancel_operation",
    "download",
    "get_guide",
    "get_image",
    "get_operation",
    "grep",
    "import_image",
    "list_files",
    "list_images",
    "list_operations",
    "read_file",
    "registry_auth",
    "registry_token_obtain",
    "rsync",
    "run",
    "set_tag",
    "upload",
    "wait_operations",
    "whoami",
];

/// Maximum encoded request or decoded result size accepted by this adapter.
pub const MAX_TOOL_JSON_BYTES: usize = 1_048_576;
/// Maximum number of independently branched commands in one call.
pub const MAX_BRANCHES: usize = 8;
/// Maximum command text accepted by the upstream `run` tool.
pub const MAX_COMMAND_BYTES: usize = 32_768;
/// Maximum combined stdout and stderr requested from one run.
pub const MAX_CAPTURE_BYTES: usize = 512 * 1024;
/// Maximum number of run environment entries.
pub const MAX_ENV_ENTRIES: usize = 128;
/// Maximum per-file payload encoded into an upstream MCP `upload` request.
pub const MAX_SYNC_FILE_BYTES: usize = 700_000;
/// Maximum total bytes in one plan-based upload batch.
pub const MAX_SYNC_BATCH_BYTES: usize = 16 * 1024 * 1024;
/// Maximum files in one plan-based upload batch.
pub const MAX_SYNC_FILES: usize = 256;
/// Maximum remote file size accepted by the upstream `download` local-file writer.
pub const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// Explicit consent marker for sending workspace content or commands to Contree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContreeEgressConsent(private::Consent);

impl ContreeEgressConsent {
    /// Records that the caller's policy layer granted egress to the configured Contree endpoint.
    #[must_use]
    pub const fn explicitly_granted() -> Self {
        Self(private::Consent)
    }
}

mod private {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Consent;
}

/// A validated loopback URL for an already supervised `contree-mcp --mode http` sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarEndpoint(Url);

impl SidecarEndpoint {
    /// Parses an explicit plain-HTTP loopback MCP endpoint.
    ///
    /// # Errors
    /// Returns [`ContreeError::Configuration`] for non-loopback, credential-bearing, or malformed URLs.
    pub fn parse(value: &str) -> Result<Self, ContreeError> {
        let url = Url::parse(value).map_err(|_| ContreeError::Configuration)?;
        let host = url.host_str().ok_or(ContreeError::Configuration)?;
        let loopback = host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if url.scheme() != "http"
            || !loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.path().ends_with("/mcp")
        {
            return Err(ContreeError::Configuration);
        }
        Ok(Self(url))
    }

    /// Returns the endpoint URL for supervised sidecar setup.
    #[must_use]
    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

/// Typed failures with no raw MCP payloads, credentials, paths, or server diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContreeError {
    /// The sidecar endpoint, consent, request, or tool contract is invalid.
    #[error("Contree configuration or request is invalid")]
    Configuration,
    /// The sidecar failed, timed out, or returned a malformed response.
    #[error("Contree MCP operation failed")]
    Transport,
    /// The local request was cancelled and any owned remote operations were cancelled.
    #[error("Contree operation cancelled")]
    Cancelled,
    /// The response exceeded a configured bound or an operation exceeded its limit.
    #[error("Contree operation exceeded a configured limit")]
    Limit,
    /// The sidecar does not expose the verified upstream tool contract.
    #[error("Contree tool inventory or schema does not match the pinned upstream contract")]
    Inventory,
    /// The remote operation failed or was not confirmed terminal.
    #[error("Contree remote operation failed")]
    Remote,
}

/// Immutable image identifier returned by Contree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImageId(String);

impl ImageId {
    /// Validates a bounded, nonempty image UUID or tag reference.
    ///
    /// # Errors
    /// Returns [`ContreeError::Configuration`] for malformed or oversized values.
    pub fn new(value: impl Into<String>) -> Result<Self, ContreeError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(ContreeError::Configuration);
        }
        Ok(Self(value))
    }

    /// Returns the exact upstream image reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `directory_state_id` produced by the verified upstream `rsync` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DirectoryStateId(u64);

impl DirectoryStateId {
    /// Creates a positive Contree directory state identifier.
    ///
    /// # Errors
    /// Returns [`ContreeError::Configuration`] for zero.
    pub fn new(value: u64) -> Result<Self, ContreeError> {
        if value == 0 {
            return Err(ContreeError::Configuration);
        }
        Ok(Self(value))
    }

    /// Returns the numeric identifier expected by upstream.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A run request using the actual `contree-mcp` source parameter names and defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    /// Shell command text, interpreted by upstream according to `shell`.
    pub command: String,
    /// Selected immutable image UUID or tag.
    pub image: ImageId,
    /// Optional directory state returned by `rsync`.
    pub directory_state_id: Option<DirectoryStateId>,
    /// Whether upstream passes command through a shell.
    pub shell: bool,
    /// Environment variables set or unset in the remote run.
    pub env: BTreeMap<String, Option<String>>,
    /// Whether upstream preserves environment metadata in a returned image.
    pub preserve_env: bool,
    /// Remote working directory; empty selects image default.
    pub cwd: String,
    /// Remote process user id.
    pub uid: u32,
    /// Remote process group id.
    pub gid: u32,
    /// Upstream command timeout in seconds.
    pub timeout_seconds: u32,
    /// Whether the command's result becomes a new immutable image.
    pub keep_result_image: bool,
    /// Optional text sent to remote stdin.
    pub stdin: Option<String>,
    /// Remote file path to uploaded file UUID injection map.
    pub files: BTreeMap<String, String>,
    /// Maximum captured output bytes requested from upstream.
    pub max_output_bytes: usize,
    /// Maximum writable layer size in bytes, if configured.
    pub max_layer_bytes: Option<u64>,
}

/// A completed, source-bound command result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    /// Source image used for this branch.
    pub source_image: ImageId,
    /// Directory state injected into the run, if any.
    pub directory_state_id: Option<DirectoryStateId>,
    /// Result image when upstream retained the writable layer.
    pub result_image: Option<ImageId>,
    /// Actual remote exit code, absent only if upstream omitted it.
    pub exit_code: Option<i32>,
    /// Whether upstream timed out the process.
    pub timed_out: bool,
    /// Bounded stdout captured by upstream.
    pub stdout: String,
    /// Bounded stderr captured by upstream.
    pub stderr: String,
    /// Upstream operation identifier.
    pub operation_id: String,
}

/// A pending upstream operation deliberately returned to the caller for explicit wait/cancel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    /// Operation UUID returned by the actual upstream server.
    pub operation_id: String,
    /// Source image of the run that created it.
    pub source_image: ImageId,
    /// Directory state injected into the run, if any.
    pub directory_state_id: Option<DirectoryStateId>,
}

/// Result of an explicit wait request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitResult {
    /// Operation identity.
    pub operation_id: String,
    /// Upstream status value.
    pub status: String,
    /// Result image if the remote operation produced one.
    pub result_image: Option<ImageId>,
    /// Command exit code if present.
    pub exit_code: Option<i32>,
    /// Whether the remote process timed out.
    pub timed_out: bool,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

/// One directory sync request that preserves the upstream rsync semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsyncRequest {
    /// Absolute local directory on the supervised MCP server host.
    pub source: String,
    /// Absolute destination inside the injected container filesystem.
    pub destination: String,
    /// Upstream exclude globs.
    pub exclude: Vec<String>,
}

/// Result of upstream's content-cached directory sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RsyncResult {
    /// Opaque positive directory state id produced by upstream.
    pub directory_state_id: DirectoryStateId,
}

/// One image item returned by `list_images`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRecord {
    /// Image UUID.
    pub uuid: ImageId,
    /// Optional tag.
    pub tag: Option<String>,
    /// ISO 8601 creation time.
    pub created_at: String,
    /// Operation that created the image, where present.
    pub operation_uuid: Option<String>,
}

/// Safe bounded image-list filter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListImagesRequest {
    /// Maximum page size.
    pub limit: Option<u32>,
    /// Page offset.
    pub offset: Option<u32>,
    /// Filter to tagged images only.
    pub tagged: bool,
    /// Optional tag prefix.
    pub tag_prefix: Option<String>,
}

/// One remote file entry returned by `list_files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteFileEntry {
    /// Base name.
    pub name: String,
    /// Absolute path within the image.
    pub path: String,
    /// Upstream kind: file, directory, or symlink.
    #[serde(rename = "type")]
    pub kind: String,
    /// Entry size in bytes.
    pub size: u64,
    /// Optional octal mode string.
    pub mode: Option<String>,
    /// Optional symlink target.
    pub target: Option<String>,
}

/// Typed adapter over one connected and supervised ConTree MCP sidecar.
pub struct ContreeBackend<C: McpToolClient> {
    client: C,
    endpoint: SidecarEndpoint,
    _egress: ContreeEgressConsent,
    max_result_bytes: usize,
}

impl<C: McpToolClient> std::fmt::Debug for ContreeBackend<C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContreeBackend")
            .field("endpoint", &self.endpoint)
            .field("max_result_bytes", &self.max_result_bytes)
            .finish_non_exhaustive()
    }
}

impl<C: McpToolClient> ContreeBackend<C> {
    /// Connects logically to a previously supervised sidecar and verifies its current MCP inventory.
    ///
    /// The transport, server authentication and process ownership are supplied by the composition
    /// root through [`McpToolClient`].
    ///
    /// # Errors
    /// Returns a sanitized configuration, transport, or inventory error.
    pub async fn connect(
        client: C,
        endpoint: SidecarEndpoint,
        egress: ContreeEgressConsent,
    ) -> Result<Self, ContreeError> {
        let backend = Self {
            client,
            endpoint,
            _egress: egress,
            max_result_bytes: MAX_TOOL_JSON_BYTES,
        };
        backend.verify_inventory().await?;
        Ok(backend)
    }

    /// Returns the validated loopback endpoint associated with this client.
    #[must_use]
    pub fn endpoint(&self) -> &SidecarEndpoint {
        &self.endpoint
    }

    /// Runs a command and either returns its owned operation id or waits for the remote result.
    ///
    /// For a waiting call, cancellation triggers an upstream `cancel_operation` for the operation
    /// id obtained from the initial `run(wait=false)` response before the call returns.
    ///
    /// # Errors
    /// Returns a sanitized validation, transport, cancellation, limit, or remote-operation error.
    pub async fn run(
        &self,
        request: &RunRequest,
        wait: bool,
        cancellation: &CancellationToken,
    ) -> Result<RunOutcome, ContreeError> {
        validate_run(request)?;
        let arguments = run_arguments(request, false)?;
        let launch = self.call("run", arguments, cancellation).await?;
        let operation_id = string_field(&launch, "operation_id")?;
        if !wait {
            return Ok(RunOutcome::Pending(PendingOperation {
                operation_id,
                source_image: request.image.clone(),
                directory_state_id: request.directory_state_id,
            }));
        }
        let waited = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                self.cancel_remote(&operation_id).await?;
                return Err(ContreeError::Cancelled);
            }
            result = self.wait_operations(std::slice::from_ref(&operation_id), request.timeout_seconds.min(300), cancellation) => match result {
                Ok(results) => results,
                Err(error) => {
                    self.cancel_remote(&operation_id).await?;
                    return Err(error);
                }
            }
        };
        let operation = waited.into_iter().next().ok_or(ContreeError::Remote)?;
        if !is_terminal_success(&operation.status) {
            let _cancelled = self.cancel_remote(&operation_id).await;
            return Err(ContreeError::Remote);
        }
        Ok(RunOutcome::Complete(RunResult {
            source_image: request.image.clone(),
            directory_state_id: request.directory_state_id,
            result_image: operation.result_image,
            exit_code: operation.exit_code,
            timed_out: operation.timed_out,
            stdout: operation.stdout,
            stderr: operation.stderr,
            operation_id,
        }))
    }

    /// Executes bounded independent branches from the same immutable image and directory state.
    pub async fn branch_run(
        &self,
        requests: &[RunRequest],
        cancellation: &CancellationToken,
    ) -> Result<Vec<RunOutcome>, ContreeError> {
        if requests.is_empty() || requests.len() > MAX_BRANCHES {
            return Err(ContreeError::Limit);
        }
        let launched_results = join_all(requests.iter().map(|request| async move {
            validate_run(request)?;
            let arguments = run_arguments(request, false)?;
            let launched = self.call("run", arguments, cancellation).await?;
            Ok::<_, ContreeError>((request, string_field(&launched, "operation_id")?))
        }))
        .await;
        let mut launches = Vec::with_capacity(launched_results.len());
        let mut launch_error = None;
        for result in launched_results {
            match result {
                Ok(value) => launches.push(value),
                Err(error) if launch_error.is_none() => launch_error = Some(error),
                Err(_) => {}
            }
        }
        if let Some(error) = launch_error {
            let ids: Vec<String> = launches.iter().map(|(_, id)| id.clone()).collect();
            self.cancel_many(&ids).await?;
            return Err(error);
        }
        let ids: Vec<String> = launches.iter().map(|(_, id)| id.clone()).collect();
        let timeout_seconds = requests
            .iter()
            .map(|request| request.timeout_seconds.min(300))
            .max()
            .ok_or(ContreeError::Limit)?;
        let waited = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                self.cancel_many(&ids).await?;
                return Err(ContreeError::Cancelled);
            }
            result = self.wait_operations(&ids, timeout_seconds, cancellation) => match result {
                Ok(results) => results,
                Err(error) => {
                    self.cancel_many(&ids).await?;
                    return Err(error);
                }
            }
        };
        let results_by_id: BTreeMap<String, WaitResult> = waited
            .into_iter()
            .map(|result| (result.operation_id.clone(), result))
            .collect();
        let mut outcomes = Vec::with_capacity(launches.len());
        for (request, operation_id) in launches {
            let result = results_by_id
                .get(&operation_id)
                .ok_or(ContreeError::Remote)?;
            if !is_terminal_success(&result.status) {
                let _cancelled = self.cancel_many(&ids).await;
                return Err(ContreeError::Remote);
            }
            outcomes.push(RunOutcome::Complete(RunResult {
                source_image: request.image.clone(),
                directory_state_id: request.directory_state_id,
                result_image: result.result_image.clone(),
                exit_code: result.exit_code,
                timed_out: result.timed_out,
                stdout: result.stdout.clone(),
                stderr: result.stderr.clone(),
                operation_id,
            }));
        }
        Ok(outcomes)
    }

    /// Syncs a directory through the upstream content-addressed `rsync` tool.
    ///
    /// `source` must be a caller-created staging directory containing only paths approved by the
    /// existing `polaris-storage-sync` plan and secret deny-list. This method never broadens that
    /// projection or invents a delete operation.
    pub async fn rsync(
        &self,
        request: &RsyncRequest,
        cancellation: &CancellationToken,
    ) -> Result<RsyncResult, ContreeError> {
        validate_rsync(request)?;
        let value = self
            .call(
                "rsync",
                json!({
                    "source": request.source,
                    "destination": request.destination,
                    "exclude": request.exclude,
                }),
                cancellation,
            )
            .await?;
        Ok(RsyncResult {
            directory_state_id: DirectoryStateId::new(number_field(&value, "directory_state_id")?)?,
        })
    }

    /// Uploads only the exact regular files selected by an existing storage `SyncPlan`.
    ///
    /// The caller supplies bytes read by the storage scanner. Every byte vector is checked against
    /// its manifest size and BLAKE3 hash, every path is reclassified through the storage ignore
    /// rules (including the non-overridable secret deny-list), and the key set must exactly equal
    /// `plan.upload`. Deletes and conflicts fail closed because Contree images are immutable.
    pub async fn sync_plan_uploads(
        &self,
        workspace_root: &Path,
        plan: &SyncPlan,
        manifest: &Manifest,
        file_contents: &BTreeMap<RelPath, Vec<u8>>,
        destination: &str,
        cancellation: &CancellationToken,
    ) -> Result<BTreeMap<String, String>, ContreeError> {
        if !plan.delete.is_empty()
            || !plan.conflicts.is_empty()
            || !plan.path_conflicts.is_empty()
            || plan.upload.is_empty()
            || plan.upload.len() > MAX_SYNC_FILES
            || plan.upload.len() != file_contents.len()
            || !destination.starts_with('/')
            || destination.contains("..")
            || destination.len() > 4096
        {
            return Err(ContreeError::Configuration);
        }
        let upload_paths: BTreeSet<RelPath> = plan.upload.iter().cloned().collect();
        let supplied_paths: BTreeSet<RelPath> = file_contents.keys().cloned().collect();
        if upload_paths != supplied_paths {
            return Err(ContreeError::Configuration);
        }
        let ignore =
            IgnoreRules::from_workspace(workspace_root).map_err(|_| ContreeError::Configuration)?;
        let mut total_bytes = 0_usize;
        let mut projected = BTreeMap::new();
        for path in &plan.upload {
            if ignore.classify(path, false) != Exclusion::Included
                || !plan.expected_remote.contains_key(path)
            {
                return Err(ContreeError::Configuration);
            }
            let entry = manifest
                .entries
                .get(path)
                .ok_or(ContreeError::Configuration)?;
            if entry.kind != EntryKind::File {
                return Err(ContreeError::Configuration);
            }
            let bytes = file_contents.get(path).ok_or(ContreeError::Configuration)?;
            let byte_count = u64::try_from(bytes.len()).map_err(|_| ContreeError::Limit)?;
            if byte_count != entry.size || bytes.len() > MAX_SYNC_FILE_BYTES {
                return Err(ContreeError::Limit);
            }
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or(ContreeError::Limit)?;
            if total_bytes > MAX_SYNC_BATCH_BYTES {
                return Err(ContreeError::Limit);
            }
            let digest = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(bytes).as_bytes());
            if digest != entry.hash {
                return Err(ContreeError::Configuration);
            }
            projected.insert(path.clone(), STANDARD.encode(bytes));
        }
        let mut uploaded = BTreeMap::new();
        for (path, content_base64) in projected {
            let file = self.upload_bytes(&content_base64, cancellation).await?;
            let remote_path = format!("{}/{}", destination.trim_end_matches('/'), path.as_str());
            uploaded.insert(remote_path, file.uuid);
        }
        Ok(uploaded)
    }

    /// Lists available image records using the actual upstream pagination/filter arguments.
    pub async fn list_images(
        &self,
        request: &ListImagesRequest,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ImageRecord>, ContreeError> {
        let limit = request.limit.unwrap_or(100);
        if limit == 0 || limit > 1000 || request.offset.is_some_and(|offset| offset > 1_000_000) {
            return Err(ContreeError::Limit);
        }
        let value = self
            .call(
                "list_images",
                json!({
                    "limit": limit,
                    "offset": request.offset.unwrap_or(0),
                    "tagged": request.tagged,
                    "tag_prefix": request.tag_prefix,
                }),
                cancellation,
            )
            .await?;
        let image_limit = usize::try_from(limit).map_err(|_| ContreeError::Limit)?;
        let images = value
            .get("images")
            .and_then(Value::as_array)
            .ok_or(ContreeError::Transport)?;
        if images.len() > image_limit {
            return Err(ContreeError::Limit);
        }
        serde_json::from_value(Value::Array(images.clone())).map_err(|_| ContreeError::Transport)
    }

    /// Inspects a remote image without spawning a microVM.
    pub async fn get_image(
        &self,
        image: &ImageId,
        cancellation: &CancellationToken,
    ) -> Result<ImageRecord, ContreeError> {
        let value = self
            .call("get_image", json!({"image": image.as_str()}), cancellation)
            .await?;
        serde_json::from_value(value).map_err(|_| ContreeError::Transport)
    }

    /// Lists image files using the upstream free inspection tool.
    pub async fn list_files(
        &self,
        image: &ImageId,
        path: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteFileEntry>, ContreeError> {
        validate_path(path)?;
        let value = self
            .call(
                "list_files",
                json!({"image": image.as_str(), "path": path}),
                cancellation,
            )
            .await?;
        let files = value
            .get("files")
            .and_then(Value::as_array)
            .ok_or(ContreeError::Transport)?;
        if files.len() > 100_000 {
            return Err(ContreeError::Limit);
        }
        serde_json::from_value(Value::Array(files.clone())).map_err(|_| ContreeError::Transport)
    }

    /// Reads one bounded remote image file.
    pub async fn read_file(
        &self,
        image: &ImageId,
        path: &str,
        cancellation: &CancellationToken,
    ) -> Result<ReadFileResult, ContreeError> {
        validate_path(path)?;
        let value = self
            .call(
                "read_file",
                json!({"image": image.as_str(), "path": path}),
                cancellation,
            )
            .await?;
        serde_json::from_value(value).map_err(|_| ContreeError::Transport)
    }

    /// Downloads one image file to an explicit absolute local destination.
    ///
    /// The image is immutable. Its file size is checked with upstream `list_files` before invoking
    /// the upstream writer, which has no independent byte-limit argument.
    pub async fn download(
        &self,
        image: &ImageId,
        path: &str,
        destination: &str,
        executable: bool,
        cancellation: &CancellationToken,
    ) -> Result<DownloadResult, ContreeError> {
        validate_path(path)?;
        let destination_path = Path::new(destination);
        if !destination_path.is_absolute() || destination.len() > 4096 {
            return Err(ContreeError::Configuration);
        }
        let (parent, _) = path.rsplit_once('/').ok_or(ContreeError::Configuration)?;
        let parent = if parent.is_empty() { "/" } else { parent };
        let entries = self.list_files(image, parent, cancellation).await?;
        let entry = entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or(ContreeError::Remote)?;
        if entry.kind != "file" || entry.size > MAX_DOWNLOAD_BYTES {
            return Err(ContreeError::Limit);
        }
        let value = self
            .call(
                "download",
                json!({
                    "image": image.as_str(),
                    "path": path,
                    "destination": destination,
                    "executable": executable,
                }),
                cancellation,
            )
            .await?;
        let result: DownloadResult =
            serde_json::from_value(value).map_err(|_| ContreeError::Transport)?;
        if !result.success
            || result.source.path != path
            || result.destination != destination
            || result.size != entry.size
            || result.executable != executable
        {
            return Err(ContreeError::Transport);
        }
        Ok(result)
    }

    /// Searches an image using the upstream bounded grep tool.
    pub async fn grep(
        &self,
        image: &ImageId,
        pattern: &str,
        path: Option<&str>,
        max_count: u32,
        max_total: u32,
        cancellation: &CancellationToken,
    ) -> Result<GrepResult, ContreeError> {
        if pattern.is_empty()
            || pattern.len() > 4096
            || max_count == 0
            || max_count > 10_000
            || max_total == 0
            || max_total > 100_000
        {
            return Err(ContreeError::Limit);
        }
        if let Some(path) = path {
            validate_path(path)?;
        }
        let value = self
            .call(
                "grep",
                json!({
                    "image": image.as_str(),
                    "pattern": pattern,
                    "path": path,
                    "max_count": max_count,
                    "max_total": max_total,
                }),
                cancellation,
            )
            .await?;
        serde_json::from_value(value).map_err(|_| ContreeError::Transport)
    }

    /// Uploads bounded bytes through the verified upstream `upload(content_base64=...)` surface.
    pub async fn upload_bytes(
        &self,
        content_base64: &str,
        cancellation: &CancellationToken,
    ) -> Result<UploadedFile, ContreeError> {
        if content_base64.is_empty() || content_base64.len() > MAX_TOOL_JSON_BYTES {
            return Err(ContreeError::Limit);
        }
        let value = self
            .call(
                "upload",
                json!({"content_base64": content_base64}),
                cancellation,
            )
            .await?;
        serde_json::from_value(value).map_err(|_| ContreeError::Transport)
    }

    /// Gets the status and result for an existing upstream operation.
    pub async fn get_operation(
        &self,
        operation_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<WaitResult, ContreeError> {
        validate_operation_id(operation_id)?;
        let value = self
            .call(
                "get_operation",
                json!({"operation_id": operation_id}),
                cancellation,
            )
            .await?;
        wait_result_from_value(value)
    }

    /// Waits for a bounded set of upstream operations and returns each result in input order.
    pub async fn wait_operations(
        &self,
        operation_ids: &[String],
        timeout_seconds: u32,
        cancellation: &CancellationToken,
    ) -> Result<Vec<WaitResult>, ContreeError> {
        if operation_ids.is_empty() || operation_ids.len() > MAX_BRANCHES || timeout_seconds == 0 {
            return Err(ContreeError::Limit);
        }
        for operation_id in operation_ids {
            validate_operation_id(operation_id)?;
        }
        let value = self
            .call(
                "wait_operations",
                json!({
                    "operation_ids": operation_ids,
                    "timeout": timeout_seconds.min(300),
                    "mode": "all",
                }),
                cancellation,
            )
            .await?;
        let results = value
            .get("results")
            .and_then(Value::as_object)
            .ok_or(ContreeError::Transport)?;
        let mut output = Vec::with_capacity(operation_ids.len());
        for operation_id in operation_ids {
            let result = results.get(operation_id).ok_or(ContreeError::Remote)?;
            output.push(wait_result_from_value(result.clone())?);
        }
        Ok(output)
    }

    /// Cancels one upstream operation and verifies the returned operation identity.
    pub async fn cancel_operation(
        &self,
        operation_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<bool, ContreeError> {
        validate_operation_id(operation_id)?;
        let value = self
            .call(
                "cancel_operation",
                json!({"operation_id": operation_id}),
                cancellation,
            )
            .await?;
        if string_field(&value, "operation_id")? != operation_id {
            return Err(ContreeError::Transport);
        }
        value
            .get("cancelled")
            .and_then(Value::as_bool)
            .ok_or(ContreeError::Transport)
    }

    async fn verify_inventory(&self) -> Result<(), ContreeError> {
        let mut tools = self
            .client
            .list_tools("contree")
            .await
            .map_err(map_client_error)?;
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        let mut expected = UPSTREAM_TOOL_NAMES.to_vec();
        expected.sort_unstable();
        if names != expected {
            return Err(ContreeError::Inventory);
        }
        for tool in &tools {
            validate_schema(tool)?;
        }
        Ok(())
    }

    async fn call(
        &self,
        tool: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ContreeError> {
        if cancellation.is_cancelled() {
            return Err(ContreeError::Cancelled);
        }
        let arguments_json =
            serde_json::to_vec(&arguments).map_err(|_| ContreeError::Configuration)?;
        if arguments_json.len() > MAX_TOOL_JSON_BYTES {
            return Err(ContreeError::Limit);
        }
        let result = self
            .client
            .call_tool("contree", tool, &arguments_json, cancellation)
            .await
            .map_err(map_client_error)?;
        if result.is_error || result.result_json.len() > self.max_result_bytes {
            return Err(ContreeError::Transport);
        }
        let envelope: PortEnvelope =
            serde_json::from_slice(&result.result_json).map_err(|_| ContreeError::Transport)?;
        if envelope.is_error || envelope.truncated || envelope.content.len() != 1 {
            return Err(ContreeError::Transport);
        }
        envelope
            .content
            .into_iter()
            .next()
            .ok_or(ContreeError::Transport)?
            .into_value()
    }

    async fn cancel_remote(&self, operation_id: &str) -> Result<(), ContreeError> {
        let token = CancellationToken::new();
        let _cancelled = self.cancel_operation(operation_id, &token).await?;
        let observed = self.get_operation(operation_id, &token).await?;
        if matches!(observed.status.as_str(), "CANCELLED" | "SUCCESS" | "FAILED") {
            Ok(())
        } else {
            Err(ContreeError::Remote)
        }
    }

    async fn cancel_many(&self, operation_ids: &[String]) -> Result<(), ContreeError> {
        let futures = operation_ids
            .iter()
            .map(|operation_id| self.cancel_remote(operation_id));
        let _outcomes = try_join_all(futures).await?;
        Ok(())
    }
}

/// A completed or explicitly owned pending operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// Operation has reached terminal status.
    Complete(RunResult),
    /// Caller owns the upstream operation and must wait for or cancel it explicitly.
    Pending(PendingOperation),
}

/// Bounded remote file content read from an image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileResult {
    /// Requested remote path.
    pub path: String,
    /// Text or base64-encoded binary contents according to `encoding`.
    pub content: String,
    /// Upstream encoding label.
    pub encoding: String,
    /// Original byte length.
    pub bytes_size: u64,
}

/// Bounded grep output returned by upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepResult {
    /// Search root path.
    pub path: String,
    /// Patterns searched.
    pub patterns: Vec<String>,
    /// Whether upstream stopped at its match or time limit.
    pub truncated: bool,
    /// Matches in upstream order.
    pub matches: Vec<GrepMatch>,
}

/// One upstream grep match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepMatch {
    /// Remote image path.
    pub path: String,
    /// One-based line number.
    pub line_number: u64,
    /// Matching line text.
    pub line_text: String,
}

/// File upload metadata returned by upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadedFile {
    /// Content-addressed file UUID.
    pub uuid: String,
    /// SHA-256 reported by upstream.
    pub sha256: String,
    /// Exact uploaded byte count.
    pub size: u64,
}

/// Result of the upstream local filesystem download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadResult {
    /// Whether the sidecar reports a completed write.
    pub success: bool,
    /// Source image and path actually downloaded.
    pub source: DownloadSource,
    /// Exact absolute local destination reported by upstream.
    pub destination: String,
    /// Exact output byte count.
    pub size: u64,
    /// Whether upstream applied executable mode.
    pub executable: bool,
}

/// Download origin details returned by upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadSource {
    /// Resolved source image UUID.
    pub image: String,
    /// Path within the image.
    pub path: String,
}

fn validate_run(request: &RunRequest) -> Result<(), ContreeError> {
    if request.command.is_empty()
        || request.command.len() > MAX_COMMAND_BYTES
        || request.cwd.len() > 4096
        || request.timeout_seconds == 0
        || request.timeout_seconds > 300
        || request.max_output_bytes == 0
        || request.max_output_bytes > MAX_CAPTURE_BYTES
        || request.env.len() > MAX_ENV_ENTRIES
        || request.files.len() > MAX_SYNC_FILES
        || request
            .max_layer_bytes
            .is_some_and(|value| value == 0 || value > 1_u64 << 40)
        || request
            .stdin
            .as_ref()
            .is_some_and(|value| value.len() > MAX_TOOL_JSON_BYTES / 2)
    {
        return Err(ContreeError::Limit);
    }
    for (name, value) in &request.env {
        validate_env_name(name)?;
        if name.to_ascii_uppercase().contains("SECRET")
            || name.to_ascii_uppercase().contains("TOKEN")
            || name.to_ascii_uppercase().contains("PASSWORD")
            || name.to_ascii_uppercase().contains("CREDENTIAL")
            || name.to_ascii_uppercase().ends_with("_KEY")
            || value.as_ref().is_some_and(|value| value.len() > 8192)
        {
            return Err(ContreeError::Configuration);
        }
    }
    for (path, file_id) in &request.files {
        if !path.starts_with('/')
            || path.contains("..")
            || path.len() > 4096
            || file_id.is_empty()
            || file_id.len() > 256
        {
            return Err(ContreeError::Configuration);
        }
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), ContreeError> {
    if name.is_empty()
        || name.len() > 256
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ContreeError::Configuration);
    }
    Ok(())
}

fn validate_rsync(request: &RsyncRequest) -> Result<(), ContreeError> {
    if request.source.is_empty()
        || !std::path::Path::new(&request.source).is_absolute()
        || request.source.len() > 4096
        || request.destination.is_empty()
        || !request.destination.starts_with('/')
        || request.destination.contains("..")
        || request.destination.len() > 4096
        || request.exclude.len() > 1024
        || request
            .exclude
            .iter()
            .any(|item| item.is_empty() || item.len() > 1024)
    {
        return Err(ContreeError::Configuration);
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), ContreeError> {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(ContreeError::Configuration);
    }
    Ok(())
}

fn validate_operation_id(value: &str) -> Result<(), ContreeError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(ContreeError::Configuration);
    }
    Ok(())
}

fn run_arguments(request: &RunRequest, wait: bool) -> Result<Value, ContreeError> {
    Ok(json!({
        "command": &request.command,
        "image": request.image.as_str(),
        "shell": request.shell,
        "env": &request.env,
        "preserve_env": request.preserve_env,
        "cwd": &request.cwd,
        "uid": request.uid,
        "gid": request.gid,
        "timeout": request.timeout_seconds,
        "disposable": !request.keep_result_image,
        "stdin": &request.stdin,
        "files": &request.files,
        "directory_state_id": request.directory_state_id.map(DirectoryStateId::get),
        "wait": wait,
        "truncate_output_at": request.max_output_bytes,
        "max_layer_bytes": request.max_layer_bytes,
    }))
}

fn string_field(value: &Value, field: &str) -> Result<String, ContreeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or(ContreeError::Transport)
}

fn number_field(value: &Value, field: &str) -> Result<u64, ContreeError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(ContreeError::Transport)
}

fn is_terminal_success(status: &str) -> bool {
    status == "SUCCESS"
}

fn wait_result_from_value(value: Value) -> Result<WaitResult, ContreeError> {
    let operation_id =
        string_field(&value, "uuid").or_else(|_| string_field(&value, "operation_id"))?;
    let status = string_field(&value, "status")?;
    let result_image = value
        .get("result_image_uuid")
        .and_then(Value::as_str)
        .map(|image| ImageId::new(image.to_owned()))
        .transpose()?;
    let exit_code = value
        .get("exit_code")
        .and_then(Value::as_i64)
        .map(i32::try_from)
        .transpose()
        .map_err(|_| ContreeError::Transport)?;
    Ok(WaitResult {
        operation_id,
        status,
        result_image,
        exit_code,
        timed_out: matches!(value.get("timed_out"), Some(Value::Bool(true))),
        stdout: bounded_text_field(&value, "stdout")?,
        stderr: bounded_text_field(&value, "stderr")?,
    })
}

fn bounded_text_field(value: &Value, field: &str) -> Result<String, ContreeError> {
    let Some(text) = value.get(field).and_then(Value::as_str) else {
        return Ok(String::new());
    };
    if text.len() > MAX_CAPTURE_BYTES {
        return Err(ContreeError::Limit);
    }
    Ok(text.to_owned())
}

fn map_client_error(error: McpToolClientError) -> ContreeError {
    match error {
        McpToolClientError::Unavailable => ContreeError::Transport,
        McpToolClientError::Timeout => ContreeError::Transport,
        McpToolClientError::Cancelled => ContreeError::Cancelled,
        McpToolClientError::Denied => ContreeError::Configuration,
        McpToolClientError::Protocol => ContreeError::Transport,
    }
}

fn validate_schema(tool: &McpToolDefinition) -> Result<(), ContreeError> {
    let schema: Value =
        serde_json::from_slice(&tool.input_schema_json).map_err(|_| ContreeError::Inventory)?;
    let required: &[&str] = match tool.name.as_str() {
        "cancel_operation" | "get_operation" => &["operation_id"],
        "download" => &["image", "path", "destination"],
        "get_guide" => &["section"],
        "get_image" => &["image"],
        "grep" => &["image", "pattern"],
        "import_image" => &["registry_url"],
        "list_files" => &["image"],
        "read_file" => &["image", "path"],
        "registry_auth" => &["registry_url", "username", "token"],
        "registry_token_obtain" => &["registry_url"],
        "rsync" => &["source", "destination"],
        "run" => &["command", "image"],
        "set_tag" => &["image_uuid"],
        "wait_operations" => &["operation_ids"],
        "list_images" | "list_operations" | "upload" | "whoami" => &[],
        _ => return Err(ContreeError::Inventory),
    };
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ContreeError::Inventory)?;
    let required_actual = schema
        .get("required")
        .and_then(Value::as_array)
        .ok_or(ContreeError::Inventory)?;
    let mut actual_names = Vec::with_capacity(required_actual.len());
    for value in required_actual {
        actual_names.push(value.as_str().ok_or(ContreeError::Inventory)?);
    }
    if actual_names.len() != required.len()
        || required.iter().any(|name| !actual_names.contains(name))
    {
        return Err(ContreeError::Inventory);
    }
    for name in required {
        let property = properties.get(*name).ok_or(ContreeError::Inventory)?;
        let expected_type = if *name == "operation_ids" {
            "array"
        } else {
            "string"
        };
        if property.get("type").and_then(Value::as_str) != Some(expected_type) {
            return Err(ContreeError::Inventory);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortEnvelope {
    content: Vec<PortContent>,
    is_error: bool,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PortContent {
    Structured { structured: Value },
    Text { text: String },
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-nebius-cloud/crates/sandboxes/src/tests.rs"]
mod tests;

mod env_resolver;
pub use env_resolver::{
    ImageCandidate, ImageRanker, MAX_RESOLVER_IMAGES, MatchReason, RepoManifest, ResolvedImage,
    ResolverError, ScoredImage, TrustedImageMetadata, resolve_environment,
};

impl PortContent {
    fn into_value(self) -> Result<Value, ContreeError> {
        match self {
            Self::Structured { structured } => Ok(structured),
            Self::Text { text } => serde_json::from_str(&text).map_err(|_| ContreeError::Transport),
        }
    }
}
