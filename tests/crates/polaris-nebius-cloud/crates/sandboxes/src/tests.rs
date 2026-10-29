// Unit tests of crates/polaris-nebius-cloud/crates/sandboxes/src/lib.rs (`tests`); compiled into that module via #[path].
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use polaris_sdk::registry::{McpToolClient, McpToolClientError, McpToolDefinition, McpToolResult};
use polaris_storage_sync::{EntryKind, FileEntry, Manifest, RelPath, SyncPlan};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    ContreeBackend, ContreeEgressConsent, ContreeError, DirectoryStateId, ImageId, MAX_BRANCHES,
    PendingOperation, RunOutcome, RunRequest, SidecarEndpoint, UPSTREAM_TOOL_NAMES,
};

static TEMP_ROOT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
struct FixtureClient {
    names: Vec<String>,
    call_count: Arc<AtomicUsize>,
    cancel_seen: Arc<AtomicBool>,
    cancel_on_run: bool,
    run_schema_has_image: bool,
}

impl FixtureClient {
    fn upstream() -> Self {
        Self {
            names: UPSTREAM_TOOL_NAMES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            call_count: Arc::new(AtomicUsize::new(0)),
            cancel_seen: Arc::new(AtomicBool::new(false)),
            cancel_on_run: false,
            run_schema_has_image: true,
        }
    }

    fn definition(name: &str, include_run_image: bool) -> McpToolDefinition {
        let required: &[&str] = match name {
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
            _ => &[],
        };
        let mut properties: serde_json::Map<String, Value> = required
            .iter()
            .map(|name| {
                let schema_type = if *name == "operation_ids" {
                    "array"
                } else {
                    "string"
                };
                ((*name).to_owned(), json!({"type": schema_type}))
            })
            .collect();
        if name == "run" && !include_run_image {
            properties.remove("image");
        }
        let input_schema_json = serde_json::to_vec(&json!({
            "type": "object",
            "properties": properties,
            "required": required,
        }))
        .unwrap_or_default();
        McpToolDefinition {
            name: name.to_owned(),
            input_schema_json,
        }
    }

    fn response(value: Value) -> Result<McpToolResult, McpToolClientError> {
        let result_json = serde_json::to_vec(&json!({
            "content": [{"structured": value}],
            "is_error": false,
            "truncated": false,
        }))
        .map_err(|_| McpToolClientError::Protocol)?;
        Ok(McpToolResult {
            result_json,
            is_error: false,
        })
    }
}

impl McpToolClient for FixtureClient {
    async fn list_tools(&self, server: &str) -> Result<Vec<McpToolDefinition>, McpToolClientError> {
        if server != "contree" {
            return Err(McpToolClientError::Unavailable);
        }
        let mut definitions = Vec::with_capacity(self.names.len());
        for name in &self.names {
            definitions.push(Self::definition(name, self.run_schema_has_image));
        }
        Ok(definitions)
    }

    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments_json: &[u8],
        cancellation: &CancellationToken,
    ) -> Result<McpToolResult, McpToolClientError> {
        if server != "contree" {
            return Err(McpToolClientError::Unavailable);
        }
        let arguments: Value =
            serde_json::from_slice(arguments_json).map_err(|_| McpToolClientError::Protocol)?;
        match tool {
            "list_images" => Self::response(json!({"images": []})),
            "upload" => Self::response(json!({"uuid": "file-uuid", "sha256": "hash", "size": 4})),
            "run" => {
                let sequence = self.call_count.fetch_add(1, Ordering::SeqCst);
                if self.cancel_on_run {
                    cancellation.cancel();
                }
                Self::response(json!({"operation_id": format!("op-{sequence}")}))
            }
            "wait_operations" => {
                let ids = arguments
                    .get("operation_ids")
                    .and_then(Value::as_array)
                    .ok_or(McpToolClientError::Protocol)?;
                let mut results = serde_json::Map::new();
                for operation_id in ids {
                    let operation_id = operation_id.as_str().ok_or(McpToolClientError::Protocol)?;
                    results.insert(
                        operation_id.to_owned(),
                        json!({
                            "uuid": operation_id,
                            "status": "SUCCESS",
                            "result_image_uuid": format!("result-{operation_id}"),
                            "exit_code": 0,
                            "timed_out": false,
                            "stdout": "ok",
                            "stderr": "",
                        }),
                    );
                }
                Self::response(
                    json!({"results": results, "completed": ids, "cancelled": [], "timed_out": false}),
                )
            }
            "cancel_operation" => {
                self.cancel_seen.store(true, Ordering::SeqCst);
                let operation_id = arguments
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or(McpToolClientError::Protocol)?;
                Self::response(json!({"cancelled": true, "operation_id": operation_id}))
            }
            "get_operation" => {
                let operation_id = arguments
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .ok_or(McpToolClientError::Protocol)?;
                let status = if self.cancel_seen.load(Ordering::SeqCst) {
                    "CANCELLED"
                } else {
                    "PENDING"
                };
                Self::response(json!({"uuid": operation_id, "status": status}))
            }
            _ => Err(McpToolClientError::Unavailable),
        }
    }
}

fn endpoint() -> Result<SidecarEndpoint, ContreeError> {
    SidecarEndpoint::parse("http://127.0.0.1:9452/mcp")
}

async fn backend(client: FixtureClient) -> Result<ContreeBackend<FixtureClient>, ContreeError> {
    ContreeBackend::connect(
        client,
        endpoint()?,
        ContreeEgressConsent::explicitly_granted(),
    )
    .await
}

fn request(image: &str, directory_state_id: Option<u64>) -> Result<RunRequest, ContreeError> {
    Ok(RunRequest {
        command: "cargo test".to_owned(),
        image: ImageId::new(image.to_owned())?,
        directory_state_id: directory_state_id.map(DirectoryStateId::new).transpose()?,
        shell: true,
        env: Default::default(),
        preserve_env: false,
        cwd: "/workspace".to_owned(),
        uid: 0,
        gid: 0,
        timeout_seconds: 60,
        keep_result_image: true,
        stdin: None,
        files: BTreeMap::new(),
        max_output_bytes: 4096,
        max_layer_bytes: None,
    })
}

#[tokio::test]
async fn contree_checks_inventory_and_verified_tool_shapes()
-> Result<(), Box<dyn std::error::Error>> {
    let client = FixtureClient::upstream();
    let connected = backend(client.clone()).await?;
    assert_eq!(
        connected.endpoint().as_url().as_str(),
        "http://127.0.0.1:9452/mcp"
    );

    let mut drifted = client;
    drifted.names.retain(|name| name != "whoami");
    assert!(matches!(
        ContreeBackend::connect(
            drifted,
            endpoint()?,
            ContreeEgressConsent::explicitly_granted()
        )
        .await,
        Err(ContreeError::Inventory)
    ));
    let mut drifted_shape = FixtureClient::upstream();
    drifted_shape.run_schema_has_image = false;
    assert!(matches!(
        backend(drifted_shape).await,
        Err(ContreeError::Inventory)
    ));
    Ok(())
}

#[tokio::test]
async fn contree_bounds_operations_and_preserves_branch_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = backend(FixtureClient::upstream()).await?;
    let cancellation = CancellationToken::new();
    let outcomes = backend
        .branch_run(
            &[request("base-a", Some(31))?, request("base-b", Some(31))?],
            &cancellation,
        )
        .await?;
    assert_eq!(outcomes.len(), 2);
    match outcomes.first() {
        Some(RunOutcome::Complete(result)) => {
            assert_eq!(result.source_image.as_str(), "base-a");
            assert_eq!(result.directory_state_id, Some(DirectoryStateId::new(31)?));
            assert_eq!(
                result.result_image.as_ref().map(ImageId::as_str),
                Some("result-op-0")
            );
        }
        Some(RunOutcome::Pending(_)) | None => return Err("first branch must complete".into()),
    }
    match outcomes.get(1) {
        Some(RunOutcome::Complete(result)) => {
            assert_eq!(result.source_image.as_str(), "base-b");
            assert_eq!(
                result.result_image.as_ref().map(ImageId::as_str),
                Some("result-op-1")
            );
        }
        Some(RunOutcome::Pending(_)) | None => return Err("second branch must complete".into()),
    }
    assert!(MAX_BRANCHES >= outcomes.len());

    let pending = backend
        .run(&request("base-c", None)?, false, &cancellation)
        .await?;
    match pending {
        RunOutcome::Pending(PendingOperation {
            operation_id,
            source_image,
            ..
        }) => {
            assert_eq!(operation_id, "op-2");
            assert_eq!(source_image.as_str(), "base-c");
        }
        RunOutcome::Complete(_) => {
            return Err("non-waiting run must return its owned operation id".into());
        }
    }
    Ok(())
}

#[test]
fn contree_rejects_non_loopback_sidecar_and_missing_egress_consent() {
    assert!(matches!(
        SidecarEndpoint::parse("http://0.0.0.0:9452/mcp"),
        Err(ContreeError::Configuration)
    ));
    assert!(matches!(
        SidecarEndpoint::parse("https://example.test/mcp"),
        Err(ContreeError::Configuration)
    ));
}

#[tokio::test]
async fn contree_sanitizes_secrets_and_bounds_tool_responses()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = backend(FixtureClient::upstream()).await?;
    let mut request = request("base", None)?;
    request
        .env
        .insert("API_TOKEN".to_owned(), Some("secret-value".to_owned()));
    assert!(matches!(
        backend.run(&request, true, &CancellationToken::new()).await,
        Err(ContreeError::Configuration)
    ));
    request.env.clear();
    request.max_output_bytes = crate::MAX_CAPTURE_BYTES + 1;
    assert!(matches!(
        backend.run(&request, true, &CancellationToken::new()).await,
        Err(ContreeError::Limit)
    ));
    Ok(())
}

#[tokio::test]
async fn contree_cancellation_joins_local_calls_and_cancels_owned_operations()
-> Result<(), Box<dyn std::error::Error>> {
    let mut client = FixtureClient::upstream();
    client.cancel_on_run = true;
    let connected = backend(client.clone()).await?;
    let cancellation = CancellationToken::new();
    let result = connected
        .run(&request("base", None)?, true, &cancellation)
        .await;
    assert!(matches!(result, Err(ContreeError::Cancelled)));
    assert!(client.cancel_seen.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn contree_sync_projects_only_approved_sync_plan_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let backend = backend(FixtureClient::upstream()).await?;
    let root = temp_root()?;
    let path = RelPath::new("src/main.rs")?;
    let bytes = b"safe".to_vec();
    let digest = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(&bytes).as_bytes());
    let entry = FileEntry {
        size: 4,
        hash: digest,
        executable: false,
        kind: EntryKind::File,
    };
    let mut manifest = Manifest::default();
    manifest.entries.insert(path.clone(), entry);
    let mut plan = SyncPlan::default();
    plan.upload.push(path.clone());
    plan.expected_remote.insert(path.clone(), None);
    let mut contents = BTreeMap::new();
    contents.insert(path, bytes);
    let uploaded = backend
        .sync_plan_uploads(
            &root,
            &plan,
            &manifest,
            &contents,
            "/workspace",
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(
        uploaded.get("/workspace/src/main.rs"),
        Some(&"file-uuid".to_owned())
    );

    let secret_path = RelPath::new(".env")?;
    let secret_bytes = b"secret".to_vec();
    let secret_hash = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(&secret_bytes).as_bytes());
    manifest.entries.insert(
        secret_path.clone(),
        FileEntry {
            size: 6,
            hash: secret_hash,
            executable: false,
            kind: EntryKind::File,
        },
    );
    plan.upload.push(secret_path.clone());
    plan.expected_remote.insert(secret_path.clone(), None);
    contents.insert(secret_path, secret_bytes);
    assert!(matches!(
        backend
            .sync_plan_uploads(
                &root,
                &plan,
                &manifest,
                &contents,
                "/workspace",
                &CancellationToken::new(),
            )
            .await,
        Err(ContreeError::Configuration)
    ));
    drop(std::fs::remove_dir_all(root));
    Ok(())
}

fn temp_root() -> Result<PathBuf, std::io::Error> {
    let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "polaris-contree-fixture-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&path)?;
    Ok(path)
}
