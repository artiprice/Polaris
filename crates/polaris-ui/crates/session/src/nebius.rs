//! Nebius services for interactive sessions and the command line (P3, P5, P6). Every piece is
//! opt-in and needs the user's own Nebius access; without it the commands say what is missing.
//!
//! - **Token Factory** (`[nebius]` in `~/.polaris/config.toml`, key `NEBIUS_API_KEY`): chat is the
//!   first provider of the provider chain; [`rerank`] re-orders `/search-local` and `$rag`
//!   passages by Token Factory embeddings (masked text) when `embedding_model` is set.
//! - **Data Lab** (`/datalab-sync`, `/datalab-finetune [model]`): the session's trajectories with a
//!   passed Proof of Done are collected under an explicit consent record, masked, exported as
//!   JSONL, uploaded through the Files API and tuned through the fine-tuning API.
//! - **Contree** (`/contree-branches`, `polaris nebius contree-run|swe-run`): a user-run
//!   `contree-mcp --mode http` sidecar on loopback, reached through the MCP hub.
//! - **AI Cloud** (`polaris nebius cloud-up|cloud-status|cloud-down`): a remote OpenShell gateway
//!   VM created and removed through the official `nebius` CLI under the budget, lifetime and
//!   ownership policy of `polaris-nebius-cloud-ai-cloud`.
//!
//! `~/.polaris/nebius.json` (global layer only) holds the non-secret settings:
//! `embedding_model`, `contree` {`endpoint`, `egress_consent`}, `data_lab` {`base_model`,
//! `license`, `suffix`, `hyperparameters`}, `ai_cloud` {`cli`, `parent_id`, `subnet_id`,
//! `platform`, `preset`, `image_family`, `disk_gib`, `hourly_usd`, `budget_usd`, `max_hours`,
//! `idle_hours`, `operator_ip`}. None of these provider contracts has been exercised live from
//! this repository (the team has no Nebius access); field names follow the published
//! OpenAI-compatible, MCP and CLI documentation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use polaris_kernel::KernelHandle;
use polaris_nebius_cloud::ai_cloud::{
    BudgetGuard, IngressPolicy, OwnerMarker, ResourceLabels, ValidatedVmSpec,
    select_owned_resources,
};
use polaris_nebius_cloud::data_lab::{
    ConsentRecord, DataLabError, ExportConfig, ExportFormat, MessageInput, MessageRole, OptIn,
    ProofFacts, ProofResolver, ProofStatus, Provenance, RawContent, ResidualGuard, ToolCallInput,
    Trajectory, TrajectoryExporter, TrajectoryInput, TrajectoryLimits,
};
use polaris_nebius_cloud::sandboxes::{
    ContreeBackend, ContreeEgressConsent, DirectoryStateId, ImageId, ImageRanker, ImageRecord,
    ListImagesRequest, RepoManifest, ResolverError, RsyncRequest, RunOutcome, RunRequest,
    ScoredImage, SidecarEndpoint, TrustedImageMetadata, resolve_environment,
};
use polaris_nebius_cloud::token_factory::{
    ApiKey, BaseUrl, EmbeddingConfig, FineTuningRequest, TokenFactoryClient, hosted_client_config,
};
use polaris_sdk::contracts::{Event, Timestamp};
use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{Cid, FrameId, ModelId, Seq, SessionId};
use polaris_sdk::registry::ChatRole;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::SessionHostError;
use crate::local_search::SearchHit;

/// The settings file, relative to the user's Polaris directory.
pub const SETTINGS_FILE: &str = "nebius.json";
/// Where `contree-mcp --mode http` listens by default.
pub const DEFAULT_CONTREE_ENDPOINT: &str = "http://127.0.0.1:9452/mcp";
const DATA_LAB_DIR: &str = "data-lab";
const DATA_LAB_STATE: &str = "data-lab/state.json";
const BRANCHES_FILE: &str = ".polaris/contree/branches.json";
const CONTREE_IMAGES_FILE: &str = "contree-images.json";
const MAX_SETTINGS_BYTES: u64 = 256 * 1024;
const OWNER_LABEL: &str = "polaris-owner";
const MAX_PASSAGE_BYTES: usize = 2_000;
/// Excluded from a workspace synced to Contree: version control, build output and the usual
/// secret files (the sidecar uploads what it is pointed at).
const SYNC_EXCLUDES: [&str; 12] = [
    ".git",
    "target",
    "node_modules",
    ".polaris",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "id_rsa*",
    "id_ed25519*",
    ".aws",
];

// ------------------------------------------------------------------------------- settings

/// The non-secret Nebius settings of `~/.polaris/nebius.json`.
#[derive(Debug, Clone, Default)]
struct Settings {
    embedding_model: Option<String>,
    contree_endpoint: Option<String>,
    contree_consent: bool,
    base_model: Option<String>,
    license: Option<String>,
    suffix: Option<String>,
    hyperparameters: serde_json::Map<String, Value>,
    cloud: serde_json::Map<String, Value>,
}

fn read_settings(global: &Path) -> Result<Settings, String> {
    let path = global.join(SETTINGS_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(_) => return Err(format!("{} is unreadable", path.display())),
    };
    if !metadata.is_file() || metadata.len() > MAX_SETTINGS_BYTES {
        return Err(format!(
            "{} is not a file of at most 256 KiB",
            path.display()
        ));
    }
    let bytes = std::fs::read(&path).map_err(|_| format!("{} is unreadable", path.display()))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| format!("{} is not JSON", path.display()))?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    let object = |value: Option<&Value>| value.and_then(Value::as_object).cloned();
    let contree = object(value.get("contree")).unwrap_or_default();
    let data_lab = object(value.get("data_lab")).unwrap_or_default();
    Ok(Settings {
        embedding_model: text(value.get("embedding_model")),
        contree_endpoint: text(contree.get("endpoint")),
        contree_consent: contree
            .get("egress_consent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        base_model: text(data_lab.get("base_model")),
        license: text(data_lab.get("license")),
        suffix: text(data_lab.get("suffix")),
        hyperparameters: object(data_lab.get("hyperparameters")).unwrap_or_default(),
        cloud: object(value.get("ai_cloud")).unwrap_or_default(),
    })
}

/// The workspace's configuration, the user's Nebius settings and the Token Factory client.
struct Access {
    global: PathBuf,
    base_url: String,
    effort_models: Vec<(Effort, ModelId)>,
    settings: Result<Settings, String>,
    client: Result<TokenFactoryClient, String>,
}

impl Access {
    fn settings(&self) -> Settings {
        self.settings.clone().unwrap_or_default()
    }

    fn client(&self) -> Result<&TokenFactoryClient, SessionHostError> {
        self.client
            .as_ref()
            .map_err(|reason| SessionHostError::Ui(format!("Token Factory: {reason}")))
    }
}

/// The workspace's loaded configuration and the user's Polaris directory.
pub(crate) fn workspace_config(
    workspace: &Path,
) -> Result<(polaris_config::PolarisConfig, PathBuf), SessionHostError> {
    let invalid = |_| SessionHostError::Ui("configuration layers rejected".to_owned());
    let dirs = polaris_config::discover(workspace).map_err(invalid)?;
    let layers = polaris_config::read_layers(
        &dirs,
        &polaris_config::EnvSnapshot::capture(),
        &polaris_config::SessionOverrides::default(),
    )
    .map_err(invalid)?;
    let loaded =
        polaris_config::load(&layers, &polaris_config::TrustGrant::none()).map_err(invalid)?;
    Ok((loaded.config, dirs.global))
}

/// One configured secret from the user's keys file (`~/.polaris/.env`) or the environment, never
/// from the repository (blocking). `Ok(None)` when it is not set; the error names `what`.
pub(crate) fn load_secret(
    reference: &polaris_config::SecretRef,
    global: &Path,
    what: &str,
) -> Result<Option<secrecy::SecretString>, String> {
    use polaris_sdk::secrets::SecretSource as _;
    let location = reference
        .location()
        .map_err(|_| format!("the {what} reference is invalid"))?;
    let system = polaris_security::secrets::SystemSecretSource;
    match crate::default_env_file(None, global) {
        Some(path) => polaris_security::secrets::DotenvSecretSource::open(&path, system)
            .map_err(|_| "the keys file cannot be read".to_owned())?
            .load(&location),
        None => system.load(&location),
    }
    .map_err(|_| format!("the {what} cannot be read"))
}

async fn access(workspace: &Path) -> Result<Access, SessionHostError> {
    let (config, global) = workspace_config(workspace)?;
    let reference = config.nebius.api_key.clone();
    let key_dir = global.clone();
    let key = tokio::task::spawn_blocking(move || resolve_key(reference, &key_dir))
        .await
        .map_err(|_| SessionHostError::Unavailable("credential resolver failed"))?;
    let base_url = config.nebius.base_url.clone();
    let client = key.and_then(|key| {
        let base =
            BaseUrl::parse(&base_url).map_err(|_| "nebius.base_url is invalid".to_owned())?;
        TokenFactoryClient::new(base, key, hosted_client_config())
            .map_err(|_| "the HTTP client could not be built".to_owned())
    });
    let effort_models = Effort::ALL
        .iter()
        .map(|effort| (*effort, config.nebius.effort.model(*effort).clone()))
        .collect();
    let settings = read_settings(&global);
    Ok(Access {
        global,
        base_url,
        effort_models,
        settings,
        client,
    })
}

fn resolve_key(
    reference: Option<polaris_config::SecretRef>,
    global: &Path,
) -> Result<ApiKey, String> {
    let reference = reference.ok_or_else(|| "nebius.api_key is not set".to_owned())?;
    let secret = load_secret(&reference, global, "Nebius key")?.ok_or_else(|| {
        "NEBIUS_API_KEY is not set (~/.polaris/.env or the environment)".to_owned()
    })?;
    ApiKey::new(secret).map_err(|_| "the Nebius key is invalid".to_owned())
}

fn masked(redactor: &impl polaris_sdk::registry::Redactor, text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    let _stats = redactor.redact_into(text.as_bytes(), &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

fn bounded(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}

fn now_micros() -> i64 {
    Timestamp::now().as_microsecond()
}

// --------------------------------------------------------------------------------- status

/// `/nebius-status`: what of Token Factory, Contree, Data Lab and AI Cloud this machine can
/// reach, and with what settings.
/// # Errors
/// The configuration layers are invalid.
pub async fn status_lines(workspace: &Path) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let mut lines = vec![format!("Token Factory  {}", access.base_url)];
    if let Err(reason) = &access.settings {
        lines.push(format!("  settings: {reason}"));
    }
    let settings = access.settings();
    match &access.client {
        Err(reason) => lines.push(format!("  key: {reason}")),
        Ok(client) => {
            lines.push("  key: configured".to_owned());
            match client.list_models().await {
                Ok(models) => {
                    let listed: BTreeSet<&str> = models.iter().map(|m| m.id.as_str()).collect();
                    lines.push(format!("  models: {} listed", models.len()));
                    for (effort, model) in &access.effort_models {
                        let mark = if listed.contains(model.as_str()) {
                            "listed"
                        } else {
                            "not listed"
                        };
                        lines.push(format!(
                            "    {:<7} {} ({mark})",
                            effort.name(),
                            model.as_str()
                        ));
                    }
                }
                Err(error) => lines.push(format!("  models: request failed ({error})")),
            }
            match client.list_fine_tuning_jobs(3).await {
                Ok(jobs) if jobs.is_empty() => lines.push("  fine-tuning: no jobs".to_owned()),
                Ok(jobs) => {
                    for job in jobs {
                        lines.push(format!(
                            "  fine-tuning: {} {} on {}{}",
                            job.id,
                            job.status,
                            job.model,
                            job.fine_tuned_model
                                .map(|model| format!(" → {model}"))
                                .unwrap_or_default()
                        ));
                    }
                }
                Err(error) => lines.push(format!("  fine-tuning: request failed ({error})")),
            }
        }
    }
    lines.push(match &settings.embedding_model {
        Some(model) => format!(
            "Embeddings  {model}: /search-local and $rag passages are re-ranked semantically (masked text)"
        ),
        None => format!(
            "Embeddings  off: set embedding_model in ~/.polaris/{SETTINGS_FILE} (for example BAAI/bge-en-icl)"
        ),
    });
    let endpoint = settings
        .contree_endpoint
        .clone()
        .unwrap_or_else(|| DEFAULT_CONTREE_ENDPOINT.to_owned());
    if settings.contree_consent {
        match contree(&settings).await {
            Ok(session) => {
                let images = session
                    .backend
                    .list_images(
                        &ListImagesRequest {
                            limit: Some(200),
                            ..ListImagesRequest::default()
                        },
                        &CancellationToken::new(),
                    )
                    .await;
                lines.push(match images {
                    Ok(images) => format!(
                        "Contree  {endpoint}: connected, {} images listed",
                        images.len()
                    ),
                    Err(error) => format!("Contree  {endpoint}: connected; images: {error}"),
                });
                session.close().await;
            }
            Err(reason) => lines.push(format!("Contree  {endpoint}: {reason}")),
        }
    } else {
        lines.push(format!(
            "Contree  off: run `contree-mcp --mode http` and set contree.egress_consent=true in ~/.polaris/{SETTINGS_FILE}"
        ));
    }
    let state = read_state(&access.global);
    lines.push(format!(
        "Data Lab  last upload {}, last job {}",
        state
            .get("file_id")
            .and_then(Value::as_str)
            .unwrap_or("none"),
        state
            .get("job_id")
            .and_then(Value::as_str)
            .unwrap_or("none")
    ));
    lines.push(match cloud_settings(&settings) {
        Ok(cloud) => format!(
            "AI Cloud  project {}, {} {} in subnet {}, budget ${} (`polaris nebius cloud-status`)",
            cloud.parent_id, cloud.platform, cloud.preset, cloud.subnet_id, cloud.budget_usd
        ),
        Err(reason) => format!("AI Cloud  off: {reason}"),
    });
    Ok(lines)
}

// ----------------------------------------------------------------------------- embeddings

/// Re-order lexical `hits` for `query` by the cosine similarity of Token Factory embeddings of
/// masked text and keep `keep` of them, with a note on how they were ranked. Without a key or an
/// `embedding_model` the lexical order is kept and there is no note.
pub async fn rerank(
    workspace: &Path,
    query: &str,
    mut hits: Vec<SearchHit>,
    keep: usize,
) -> (Vec<SearchHit>, Option<String>) {
    let lexical = |mut hits: Vec<SearchHit>, note: Option<String>| {
        hits.truncate(keep);
        (hits, note)
    };
    if hits.len() < 2 {
        return lexical(hits, None);
    }
    let Ok(access) = access(workspace).await else {
        return lexical(hits, None);
    };
    let settings = access.settings();
    let (Ok(client), Some(model)) = (&access.client, settings.embedding_model.as_deref()) else {
        return lexical(hits, None);
    };
    let Ok(model_id) = ModelId::new(model) else {
        return lexical(
            hits,
            Some("semantic re-rank off: invalid embedding_model".to_owned()),
        );
    };
    let Ok(redactor) = polaris_security::automata::DfaRedactor::default_redactor() else {
        return lexical(hits, None);
    };
    let mut texts = vec![masked(&redactor, bounded(query, MAX_PASSAGE_BYTES))];
    texts.extend(
        hits.iter()
            .map(|hit| masked(&redactor, bounded(&hit.text, MAX_PASSAGE_BYTES))),
    );
    let batch: Vec<&str> = texts.iter().map(String::as_str).collect();
    let config = EmbeddingConfig {
        max_batch_items: 64,
        max_input_bytes: 16 * 1024,
        max_dimensions: 16_384,
    };
    let vectors = match client.embeddings(&model_id, &batch, &config).await {
        Ok(result) => result.vectors,
        Err(error) => {
            return lexical(
                hits,
                Some(format!("semantic re-rank unavailable ({error})")),
            );
        }
    };
    let Some((query_vector, passages)) = vectors.split_first() else {
        return lexical(hits, None);
    };
    let mut scored: Vec<(f32, SearchHit)> = passages
        .iter()
        .map(|vector| cosine(query_vector, vector))
        .zip(hits.drain(..))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let reranked = scored.into_iter().map(|(_, hit)| hit).collect();
    lexical(reranked, Some(format!("semantic re-rank: {model}")))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut aa, mut bb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa <= f32::EPSILON || bb <= f32::EPSILON {
        return 0.0;
    }
    dot / (aa.sqrt() * bb.sqrt())
}

// ------------------------------------------------------------------------------- Data Lab

fn read_state(global: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(global.join(DATA_LAB_STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn write_state(
    global: &Path,
    state: &serde_json::Map<String, Value>,
) -> Result<(), SessionHostError> {
    let path = global.join(DATA_LAB_STATE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| SessionHostError::Unavailable("the Data Lab directory"))?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|_| SessionHostError::Unavailable("the Data Lab state"))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|_| SessionHostError::Unavailable("the Data Lab state cannot be written"))
}

/// Binds a kernel Proof of Done report to the diff and the sandbox run of the same frame that
/// the composition root read from the event log just before the proof.
struct EventBoundProof {
    diff: Cid,
    test_run: Cid,
}

impl ProofResolver for EventBoundProof {
    fn resolve(&self, proof: &[u8]) -> Result<ProofFacts, DataLabError> {
        let report: Value =
            serde_json::from_slice(proof).map_err(|_| DataLabError::Schema("proof report"))?;
        let field = |name: &'static str| {
            report
                .get(name)
                .cloned()
                .ok_or(DataLabError::Schema("proof report field"))
        };
        if report.get("schema").and_then(Value::as_u64) != Some(2) {
            return Err(DataLabError::Schema("proof report version"));
        }
        let passed = report.get("passed").and_then(Value::as_bool) == Some(true);
        let session: SessionId = serde_json::from_value(field("session")?)
            .map_err(|_| DataLabError::Schema("proof session"))?;
        let frame: FrameId = serde_json::from_value(field("frame")?)
            .map_err(|_| DataLabError::Schema("proof frame"))?;
        let tree: Cid = serde_json::from_value(field("tree")?)
            .map_err(|_| DataLabError::Schema("proof tree"))?;
        Ok(ProofFacts {
            session,
            frame,
            workspace_snapshot: tree,
            diff: self.diff,
            test_run: self.test_run,
            status: if passed {
                ProofStatus::Passed
            } else {
                ProofStatus::Failed
            },
        })
    }
}

/// One frame of the session whose tests passed in the sandbox after a reviewed change.
struct ProvedFrame {
    frame: FrameId,
    proof: Cid,
    tree: Cid,
    diff: Cid,
    test_run: Cid,
    transcript: Option<Cid>,
}

fn proved_frames(records: &[polaris_sdk::contracts::EventRecord]) -> Vec<ProvedFrame> {
    let mut diffs: HashMap<FrameId, Cid> = HashMap::new();
    let mut runs: HashMap<FrameId, Cid> = HashMap::new();
    let mut transcripts: HashMap<FrameId, Cid> = HashMap::new();
    let mut proved = Vec::new();
    for record in records {
        match record.event() {
            Event::DiffApproved { frame, diff } => {
                let _previous = diffs.insert(*frame, *diff);
            }
            Event::SandboxExecutionCompleted { frame, exec, .. } => {
                let _previous = runs.insert(*frame, *exec);
            }
            Event::FrameStepRecorded {
                frame, transcript, ..
            } => {
                let _previous = transcripts.insert(*frame, *transcript);
            }
            Event::ProofOfDonePassed { frame, report } => {
                if let (Some(diff), Some(test_run)) = (diffs.get(frame), runs.get(frame)) {
                    proved.push(ProvedFrame {
                        frame: *frame,
                        proof: *report,
                        tree: Cid::from_bytes([0; 32]),
                        diff: *diff,
                        test_run: *test_run,
                        transcript: None,
                    });
                }
            }
            _ => {}
        }
    }
    // The newest step transcript of each frame carries its whole conversation.
    for frame in &mut proved {
        frame.transcript = transcripts.get(&frame.frame).copied();
    }
    proved
}

fn messages_of(transcript: &[u8]) -> Option<(Vec<MessageInput>, String)> {
    use polaris_runtime::frames::{FrameStepRecord, StepOutcome};
    let record: FrameStepRecord = serde_json::from_slice(transcript).ok()?;
    let request = record.request?;
    let mut messages: Vec<MessageInput> = request
        .messages
        .into_iter()
        .map(|message| MessageInput {
            role: match message.role {
                ChatRole::System => MessageRole::System,
                ChatRole::User => MessageRole::User,
                ChatRole::Assistant => MessageRole::Assistant,
                ChatRole::Tool => MessageRole::Tool,
            },
            content: RawContent::text(message.content),
            tool_calls: message
                .tool_calls
                .into_iter()
                .map(|call| ToolCallInput {
                    id: call.id,
                    name: call.name,
                    arguments: RawContent::text(call.arguments_json),
                })
                .collect(),
            tool_call_id: message.tool_call_id,
        })
        .collect();
    let answer = match record.result {
        Some(StepOutcome::Continue { text }) => Some(text),
        Some(StepOutcome::ClaimedDone { claim }) => Some(claim),
        _ => None,
    };
    if let Some(answer) = answer.filter(|text| !text.trim().is_empty()) {
        messages.push(MessageInput {
            role: MessageRole::Assistant,
            content: RawContent::text(answer),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
    }
    Some((messages, request.model.as_str().to_owned()))
}

/// The workspace's licence as its LICENSE file names it, when it is a common one.
fn detect_license(workspace: &Path) -> Option<String> {
    for name in ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"] {
        let Ok(text) = std::fs::read_to_string(workspace.join(name)) else {
            continue;
        };
        let head: String = text
            .chars()
            .take(400)
            .collect::<String>()
            .to_ascii_lowercase();
        let known = [
            ("apache license", "Apache-2.0"),
            ("mit license", "MIT"),
            ("bsd 3-clause", "BSD-3-Clause"),
            ("bsd 2-clause", "BSD-2-Clause"),
            ("mozilla public license", "MPL-2.0"),
            ("gnu general public license", "GPL-3.0"),
        ];
        if let Some((_, spdx)) = known.iter().find(|(needle, _)| head.contains(needle)) {
            return Some((*spdx).to_owned());
        }
    }
    None
}

/// `/datalab-sync`: export this session's proved trajectories (consent recorded by this command,
/// content masked and rescanned) and upload them to Token Factory as a fine-tuning dataset.
/// # Errors
/// No key, no proved trajectory, a blocked export, or a failed upload.
pub async fn datalab_sync_lines(
    kernel: &KernelHandle,
    workspace: &Path,
) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let client = access.client()?.clone();
    let settings = access.settings();
    let license = settings
        .license
        .clone()
        .or_else(|| detect_license(workspace))
        .ok_or_else(|| {
            SessionHostError::Ui(format!(
                "Data Lab: the workspace licence is unknown; set data_lab.license in ~/.polaris/{SETTINGS_FILE}"
            ))
        })?;
    let context = Arc::clone(kernel.context());
    let root = context.root().to_owned();
    let output = access.global.join(DATA_LAB_DIR).join("exports");
    let repository = workspace
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let export = tokio::task::spawn_blocking(move || {
        export_trajectories(&context, &root, &output, repository, license)
    })
    .await
    .map_err(|_| SessionHostError::Unavailable("Data Lab export task failed"))??;
    let mut lines = vec![format!(
        "Exported {} proved trajectorie{} ({} bytes, masked) to {}",
        export.trajectories,
        if export.trajectories == 1 { "" } else { "s" },
        export.bytes,
        export.path.display()
    )];
    lines.extend(
        export
            .skipped
            .iter()
            .map(|reason| format!("  skipped: {reason}")),
    );
    let name = export
        .training
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "polaris-trajectories.jsonl".to_owned());
    let bytes = std::fs::read(&export.training)
        .map_err(|_| SessionHostError::Unavailable("the training file cannot be read"))?;
    let file = client
        .upload_file("fine-tune", &name, Bytes::from(bytes))
        .await
        .map_err(|error| SessionHostError::Ui(format!("Data Lab upload failed: {error}")))?;
    lines.push(format!(
        "Uploaded as {} (purpose fine-tune); start tuning with /datalab-finetune [base model]",
        file.id
    ));
    let mut state = read_state(&access.global);
    let _previous = state.insert("file_id".to_owned(), json!(file.id));
    let _previous = state.insert("export".to_owned(), json!(export.path));
    let _previous = state.insert("uploaded_at_us".to_owned(), json!(now_micros()));
    write_state(&access.global, &state)?;
    Ok(lines)
}

struct ExportSummary {
    path: PathBuf,
    training: PathBuf,
    trajectories: usize,
    bytes: usize,
    skipped: Vec<String>,
}

fn export_trajectories(
    context: &Arc<polaris_kernel::KernelCtx>,
    root: &Path,
    output: &Path,
    repository: Option<String>,
    license: String,
) -> Result<ExportSummary, SessionHostError> {
    use polaris_kernel::storage::cas::{Cas, CasConfig};
    use polaris_kernel::verifier::EvidenceStore;
    let records = context
        .records(Seq::new(1), Seq::new(u64::MAX))
        .map_err(|_| SessionHostError::Unavailable("the session log cannot be read"))?;
    let session = records
        .first()
        .map(polaris_sdk::contracts::EventRecord::session)
        .ok_or(SessionHostError::Unavailable("the session log is empty"))?;
    let mut frames = proved_frames(&records);
    if frames.is_empty() {
        return Err(SessionHostError::Ui(
            "Data Lab: no trajectory to export — a frame needs an applied change and a passed Proof of Done (run_tests)"
                .to_owned(),
        ));
    }
    let cas = Arc::new(
        Cas::open(CasConfig::new(root.join("cas")))
            .map_err(|error| SessionHostError::Ui(format!("CAS: {error}")))?,
    );
    let store = polaris_kernel::KernelEvidence::new(Arc::clone(context));
    // The user's command is the consent: a fresh record for this session only.
    let consent = ConsentRecord {
        schema: 1,
        session,
        enabled: true,
        revision: u64::try_from(now_micros()).unwrap_or_default(),
        recorded_at: Timestamp::now(),
    };
    let consent_bytes = serde_json::to_vec(&consent)
        .map_err(|_| SessionHostError::Unavailable("the consent record"))?;
    let consent_cid = cas
        .put(&consent_bytes)
        .map_err(|error| SessionHostError::Ui(format!("CAS: {error}")))?;
    let optin = OptIn::load(&cas, consent_cid, session)
        .map_err(|error| SessionHostError::Ui(format!("Data Lab consent: {error}")))?;
    let redactor = polaris_security::automata::DfaRedactor::default_redactor()
        .map_err(|_| SessionHostError::Unavailable("the redactor did not compile"))?;
    let guard = ResidualGuard::new()
        .map_err(|error| SessionHostError::Ui(format!("Data Lab scanner: {error}")))?;
    let mut masked_trajectories = Vec::new();
    let mut skipped = Vec::new();
    for frame in &mut frames {
        let Some(transcript) = frame.transcript else {
            skipped.push(format!("frame {}: no step transcript", frame.frame));
            continue;
        };
        let Some((messages, model)) = store
            .get(&transcript)
            .ok()
            .flatten()
            .and_then(|bytes| messages_of(&bytes))
        else {
            skipped.push(format!("frame {}: unreadable transcript", frame.frame));
            continue;
        };
        if let Some(tree) = store
            .get(&frame.proof)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|report| report.get("tree").cloned())
            .and_then(|tree| serde_json::from_value::<Cid>(tree).ok())
        {
            frame.tree = tree;
        }
        let input = TrajectoryInput {
            session,
            frame: frame.frame,
            workspace_snapshot: frame.tree,
            diff: frame.diff,
            test_run: frame.test_run,
            collected_at: Timestamp::now(),
            messages,
            provenance: Provenance {
                repository: repository.clone(),
                license: Some(license.clone()),
                excluded: false,
                model,
                tool_versions: vec![format!("polaris {}", env!("CARGO_PKG_VERSION"))],
            },
        };
        let resolver = EventBoundProof {
            diff: frame.diff,
            test_run: frame.test_run,
        };
        let outcome = Trajectory::collect(input, &optin, TrajectoryLimits::default())
            .and_then(|draft| draft.verified(&cas, &resolver, frame.proof, &optin))
            .and_then(|verified| verified.mask(&cas, &redactor, &guard, &optin));
        match outcome {
            Ok(trajectory) => masked_trajectories.push(trajectory),
            Err(error) => skipped.push(format!("frame {}: {error}", frame.frame)),
        }
    }
    if masked_trajectories.is_empty() {
        return Err(SessionHostError::Ui(format!(
            "Data Lab: every trajectory was blocked ({})",
            skipped.join("; ")
        )));
    }
    let exporter = TrajectoryExporter::new(
        ExportConfig {
            output_directory: output.to_owned(),
            max_line_bytes: 8 * 1024 * 1024,
            max_file_bytes: 64 * 1024 * 1024,
            max_batch: 256,
        },
        Arc::clone(&cas),
        optin,
    )
    .map_err(|error| SessionHostError::Ui(format!("Data Lab export: {error}")))?;
    let references: Vec<_> = masked_trajectories.iter().collect();
    let receipt = exporter
        .write_local(&references, ExportFormat::ChatMlJsonl)
        .map_err(|error| SessionHostError::Ui(format!("Data Lab export: {error}")))?;
    // The provider dataset carries the conversations only; provenance stays in the local export.
    let exported = std::fs::read_to_string(&receipt.path)
        .map_err(|_| SessionHostError::Unavailable("the export cannot be read"))?;
    let mut training = String::new();
    for line in exported.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(messages) = record.get("messages") {
            let _written = writeln!(training, "{}", json!({ "messages": messages }));
        }
    }
    let training_path = receipt.path.with_extension("train.jsonl");
    std::fs::write(&training_path, training.as_bytes())
        .map_err(|_| SessionHostError::Unavailable("the training file cannot be written"))?;
    Ok(ExportSummary {
        path: receipt.path,
        training: training_path,
        trajectories: receipt.trajectories,
        bytes: receipt.bytes,
        skipped,
    })
}

/// `/datalab-finetune [model]`: start a fine-tuning job on the last uploaded dataset, or report
/// the job already started.
/// # Errors
/// No key, nothing uploaded yet, no base model, or a failed request.
pub async fn datalab_finetune_lines(
    workspace: &Path,
    model: Option<&str>,
) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let client = access.client()?;
    let settings = access.settings();
    let mut state = read_state(&access.global);
    let file = state
        .get("file_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            SessionHostError::Ui(
                "Data Lab: nothing uploaded yet; run /datalab-sync first".to_owned(),
            )
        })?;
    let base = model
        .map(str::to_owned)
        .or(settings.base_model)
        .ok_or_else(|| {
            SessionHostError::Ui(format!(
                "Data Lab: name a base model (/datalab-finetune <model>) or set data_lab.base_model in ~/.polaris/{SETTINGS_FILE}"
            ))
        })?;
    let request = FineTuningRequest {
        model: base.clone(),
        training_file: file.clone(),
        validation_file: None,
        suffix: Some(settings.suffix.unwrap_or_else(|| "polaris".to_owned())),
        hyperparameters: settings.hyperparameters,
    };
    let job = client
        .create_fine_tuning_job(&request)
        .await
        .map_err(|error| SessionHostError::Ui(format!("Data Lab fine-tuning failed: {error}")))?;
    let _previous = state.insert("job_id".to_owned(), json!(job.id));
    write_state(&access.global, &state)?;
    Ok(vec![
        format!(
            "Fine-tuning job {} started on {base} with {file}: {}",
            job.id, job.status
        ),
        "Its progress is shown by /nebius-status.".to_owned(),
    ])
}

// -------------------------------------------------------------------------------- Contree

/// The user configured the sidecar and granted egress in their own settings file.
struct ConfiguredContree;

impl polaris_ext::mcp::EffectAuthorizer for ConfiguredContree {
    fn authorize(
        &self,
        _tool: &polaris_ext::mcp::ToolSpec,
    ) -> polaris_ext::mcp::EffectAuthorization {
        polaris_ext::mcp::EffectAuthorization::Allow
    }
}

/// The sidecar is started by the user; the hub never launches a host process for it.
struct NoLauncher;

impl polaris_ext::mcp::ProcessLauncher for NoLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a polaris_ext::mcp::McpServerSpec,
    ) -> polaris_ext::mcp::LaunchFuture<'a> {
        Box::pin(async { Err(polaris_ext::mcp::McpError::InvalidSpec) })
    }
}

type ContreeClient = polaris_ext::mcp::McpHubToolClient<ConfiguredContree>;

struct ContreeSession {
    backend: ContreeBackend<ContreeClient>,
    hub: Arc<polaris_ext::mcp::McpHub>,
    server: polaris_ext::mcp::ServerName,
}

impl ContreeSession {
    async fn close(self) {
        let _closed = self.hub.disconnect(&self.server).await;
    }
}

async fn contree(settings: &Settings) -> Result<ContreeSession, String> {
    use polaris_ext::mcp::{McpHub, McpServerSpec, McpTransport, ServerName, TrustLevel};
    if !settings.contree_consent {
        return Err(format!(
            "egress is not granted (contree.egress_consent in ~/.polaris/{SETTINGS_FILE})"
        ));
    }
    let endpoint = settings
        .contree_endpoint
        .clone()
        .unwrap_or_else(|| DEFAULT_CONTREE_ENDPOINT.to_owned());
    let sidecar = SidecarEndpoint::parse(&endpoint)
        .map_err(|_| "the endpoint must be http://127.0.0.1:<port>/mcp".to_owned())?;
    let server = ServerName::new("contree").map_err(|_| "server name".to_owned())?;
    let hub = Arc::new(McpHub::new(
        Arc::new(NoLauncher),
        Arc::new(polaris_security::secrets::SystemSecretSource),
    ));
    let spec = McpServerSpec {
        name: server.clone(),
        transport: McpTransport::StreamableHttp {
            url: sidecar.as_url().clone(),
            headers: Vec::new(),
        },
        trust: TrustLevel::UserGlobal,
        startup_timeout: Duration::from_secs(20),
        call_timeout: Duration::from_secs(330),
        tool_allowlist: None,
        max_result_bytes: 4 * 1024 * 1024,
    };
    let client = polaris_ext::mcp::McpHubToolClient::connect(
        Arc::clone(&hub),
        spec,
        Arc::new(ConfiguredContree),
    )
    .await
    .map_err(|error| format!("the sidecar is not reachable ({error})"))?;
    let backend =
        ContreeBackend::connect(client, sidecar, ContreeEgressConsent::explicitly_granted())
            .await
            .map_err(|error| error.to_string())?;
    Ok(ContreeSession {
        backend,
        hub,
        server,
    })
}

fn branches_path(workspace: &Path) -> PathBuf {
    workspace.join(BRANCHES_FILE)
}

fn read_branches(workspace: &Path) -> Vec<Value> {
    std::fs::read(branches_path(workspace))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

/// `/contree-branches`: the branch runs this workspace made in Contree and the sidecar's images.
/// # Errors
/// The configuration layers are invalid.
pub async fn contree_branches_lines(workspace: &Path) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let settings = access.settings();
    let mut lines = Vec::new();
    let runs = read_branches(workspace);
    if runs.is_empty() {
        lines.push("No Contree branch run from this workspace yet (`polaris nebius contree-run <image> <command>…`).".to_owned());
    }
    for run in runs.iter().rev().take(10) {
        lines.push(format!(
            "{}  image {}  state {}",
            run.get("at").and_then(Value::as_str).unwrap_or("?"),
            run.get("image").and_then(Value::as_str).unwrap_or("?"),
            run.get("directory_state_id")
                .and_then(Value::as_u64)
                .map_or_else(|| "none".to_owned(), |id| id.to_string())
        ));
        for branch in run
            .get("branches")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            lines.push(format!(
                "  ├ exit {}  {}{}",
                branch
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .map_or_else(|| "?".to_owned(), |code| code.to_string()),
                branch.get("command").and_then(Value::as_str).unwrap_or(""),
                branch
                    .get("result_image")
                    .and_then(Value::as_str)
                    .map(|image| format!("  → {image}"))
                    .unwrap_or_default()
            ));
        }
    }
    match contree(&settings).await {
        Ok(session) => {
            match session
                .backend
                .list_images(
                    &ListImagesRequest {
                        limit: Some(20),
                        tagged: true,
                        ..ListImagesRequest::default()
                    },
                    &CancellationToken::new(),
                )
                .await
            {
                Ok(images) => {
                    lines.push(format!("Tagged images ({} shown):", images.len()));
                    for image in images {
                        lines.push(format!(
                            "  {}  {}",
                            image.uuid.as_str(),
                            image.tag.unwrap_or_default()
                        ));
                    }
                }
                Err(error) => lines.push(format!("Images: {error}")),
            }
            session.close().await;
        }
        Err(reason) => lines.push(format!("Contree: {reason}")),
    }
    Ok(lines)
}

fn run_request(image: &ImageId, state: Option<DirectoryStateId>, command: &str) -> RunRequest {
    RunRequest {
        command: command.to_owned(),
        image: image.clone(),
        directory_state_id: state,
        shell: true,
        env: BTreeMap::new(),
        preserve_env: false,
        cwd: if state.is_some() {
            "/workspace".to_owned()
        } else {
            String::new()
        },
        uid: 0,
        gid: 0,
        timeout_seconds: 300,
        keep_result_image: true,
        stdin: None,
        files: BTreeMap::new(),
        max_output_bytes: 256 * 1024,
        max_layer_bytes: None,
    }
}

/// Run `commands` as independent branches from `image` (with the workspace synced to
/// `/workspace` when `sync`), record the branch tree in the workspace and report each branch.
/// # Errors
/// Contree is not configured or reachable, or a run failed.
pub async fn contree_run_lines(
    workspace: &Path,
    image: &str,
    commands: &[String],
    sync: bool,
) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let settings = access.settings();
    let session = contree(&settings)
        .await
        .map_err(|reason| SessionHostError::Ui(format!("Contree: {reason}")))?;
    let outcome = run_branches(&session, workspace, image, commands, sync).await;
    session.close().await;
    outcome
}

async fn run_branches(
    session: &ContreeSession,
    workspace: &Path,
    image: &str,
    commands: &[String],
    sync: bool,
) -> Result<Vec<String>, SessionHostError> {
    let cancellation = CancellationToken::new();
    let image = ImageId::new(image)
        .map_err(|_| SessionHostError::Ui("Contree: invalid image id".to_owned()))?;
    let state = if sync {
        let source = std::fs::canonicalize(workspace)
            .map_err(|_| SessionHostError::Unavailable("the workspace path"))?;
        let synced = session
            .backend
            .rsync(
                &RsyncRequest {
                    source: source.to_string_lossy().into_owned(),
                    destination: "/workspace".to_owned(),
                    exclude: SYNC_EXCLUDES
                        .iter()
                        .map(|glob| (*glob).to_owned())
                        .collect(),
                },
                &cancellation,
            )
            .await
            .map_err(|error| SessionHostError::Ui(format!("Contree sync: {error}")))?;
        Some(synced.directory_state_id)
    } else {
        None
    };
    let requests: Vec<RunRequest> = commands
        .iter()
        .map(|command| run_request(&image, state, command))
        .collect();
    let outcomes = session
        .backend
        .branch_run(&requests, &cancellation)
        .await
        .map_err(|error| SessionHostError::Ui(format!("Contree run: {error}")))?;
    let mut lines = vec![format!(
        "{} branch{} from {}{}",
        outcomes.len(),
        if outcomes.len() == 1 { "" } else { "es" },
        image.as_str(),
        state.map_or_else(String::new, |id| format!(
            " with the workspace (state {})",
            id.get()
        ))
    )];
    let mut branches = Vec::new();
    for (command, outcome) in commands.iter().zip(outcomes) {
        let RunOutcome::Complete(result) = outcome else {
            continue;
        };
        lines.push(format!(
            "├ exit {}{}  {command}",
            result
                .exit_code
                .map_or_else(|| "?".to_owned(), |code| code.to_string()),
            if result.timed_out { " (timed out)" } else { "" }
        ));
        for line in result
            .stdout
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            lines.push(format!("│   {line}"));
        }
        branches.push(json!({
            "command": command,
            "exit_code": result.exit_code,
            "timed_out": result.timed_out,
            "result_image": result.result_image.as_ref().map(ImageId::as_str),
            "operation_id": result.operation_id,
        }));
    }
    let mut runs = read_branches(workspace);
    runs.push(json!({
        "at": Timestamp::now().to_string(),
        "image": image.as_str(),
        "directory_state_id": state.map(DirectoryStateId::get),
        "branches": branches,
    }));
    let path = branches_path(workspace);
    if let Some(parent) = path.parent() {
        let _created = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&runs) {
        let _saved = std::fs::write(&path, bytes);
    }
    Ok(lines)
}

/// Ranks equally eligible images newest first (ISO 8601 creation times sort as text).
struct NewestFirst;

impl ImageRanker for NewestFirst {
    fn rank(
        &self,
        _manifest: &RepoManifest,
        candidates: &[polaris_nebius_cloud::sandboxes::ImageCandidate],
    ) -> Result<Vec<ScoredImage>, ResolverError> {
        let mut order: Vec<&polaris_nebius_cloud::sandboxes::ImageCandidate> =
            candidates.iter().collect();
        order.sort_by(|a, b| a.record().created_at.cmp(&b.record().created_at));
        Ok(order
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let score = u16::try_from(index.saturating_add(1))
                    .unwrap_or(u16::MAX)
                    .min(10_000);
                ScoredImage::new(candidate.image_id().clone(), score)
            })
            .collect())
    }
}

fn trusted_metadata(global: &Path) -> Result<Vec<TrustedImageMetadata>, String> {
    let path = global.join(CONTREE_IMAGES_FILE);
    let bytes = std::fs::read(&path).map_err(|_| {
        format!(
            "{} is missing: list the images you trust as [{{\"image\", \"repo\", \"commit\", \"languages\"}}]",
            path.display()
        )
    })?;
    let entries: Vec<Value> = serde_json::from_slice(&bytes)
        .map_err(|_| format!("{} is not a JSON list", path.display()))?;
    entries
        .iter()
        .map(|entry| {
            let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
            let image = ImageId::new(text("image").unwrap_or_default())
                .map_err(|_| "an entry has no valid image".to_owned())?;
            let languages = entry
                .get("languages")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let toolchains = entry
                .get("toolchains")
                .and_then(Value::as_object)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|(name, version)| {
                            version.as_str().map(|v| (name.clone(), v.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            TrustedImageMetadata::new(
                image,
                text("repo"),
                text("commit"),
                languages,
                toolchains,
                text("runtime"),
                text("architecture"),
            )
            .map_err(|error| error.to_string())
        })
        .collect()
}

/// `polaris nebius swe-run <owner/repo> <commit> <command>`: resolve the SWE environment image
/// for the repository and commit from the sidecar's catalog and the user's trusted annotations
/// (`~/.polaris/contree-images.json`), then run the command there with the workspace synced.
/// # Errors
/// Contree is not configured or reachable, no image matches, or the run failed.
pub async fn swe_run_lines(
    workspace: &Path,
    repo: &str,
    commit: &str,
    command: &str,
) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let settings = access.settings();
    let metadata = trusted_metadata(&access.global)
        .map_err(|reason| SessionHostError::Ui(format!("SWE run: {reason}")))?;
    let manifest = RepoManifest::new(repo, commit, BTreeSet::new(), BTreeMap::new(), None, None)
        .map_err(|error| SessionHostError::Ui(format!("SWE run: {error}")))?;
    let session = contree(&settings)
        .await
        .map_err(|reason| SessionHostError::Ui(format!("Contree: {reason}")))?;
    let images: Result<Vec<ImageRecord>, _> = session
        .backend
        .list_images(
            &ListImagesRequest {
                limit: Some(500),
                ..ListImagesRequest::default()
            },
            &CancellationToken::new(),
        )
        .await;
    let outcome = match images {
        Err(error) => Err(SessionHostError::Ui(format!("Contree images: {error}"))),
        Ok(images) => match resolve_environment(&manifest, &images, &metadata, &NewestFirst) {
            Err(error) => Err(SessionHostError::Ui(format!("SWE run: {error}"))),
            Ok(resolved) => {
                let mut lines = vec![format!(
                    "{repo}@{} → image {} ({:?})",
                    bounded(commit, 12),
                    resolved.image_id().as_str(),
                    resolved.reason()
                )];
                match run_branches(
                    &session,
                    workspace,
                    resolved.image_id().as_str(),
                    &[command.to_owned()],
                    true,
                )
                .await
                {
                    Ok(run) => {
                        lines.extend(run);
                        Ok(lines)
                    }
                    Err(error) => Err(error),
                }
            }
        },
    };
    session.close().await;
    outcome
}

// ------------------------------------------------------------------------------- AI Cloud

/// The remote gateway VM settings of `ai_cloud` in `~/.polaris/nebius.json`.
#[derive(Debug, Clone)]
struct CloudSettings {
    cli: String,
    parent_id: String,
    subnet_id: String,
    platform: String,
    preset: String,
    image_family: String,
    disk_gib: u32,
    hourly_usd: String,
    budget_usd: String,
    max_hours: u64,
    idle_hours: u64,
    operator_ip: Option<std::net::IpAddr>,
}

fn cloud_settings(settings: &Settings) -> Result<CloudSettings, String> {
    let cloud = &settings.cloud;
    let text = |key: &str| cloud.get(key).and_then(Value::as_str).map(str::to_owned);
    let number =
        |key: &str, default: u64| cloud.get(key).and_then(Value::as_u64).unwrap_or(default);
    let required = |key: &str| {
        text(key).ok_or_else(|| format!("set ai_cloud.{key} in ~/.polaris/{SETTINGS_FILE}"))
    };
    let money = |value: &Value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    };
    Ok(CloudSettings {
        cli: text("cli").unwrap_or_else(|| "nebius".to_owned()),
        parent_id: required("parent_id")?,
        subnet_id: required("subnet_id")?,
        platform: required("platform")?,
        preset: required("preset")?,
        image_family: text("image_family").unwrap_or_else(|| "ubuntu24.04-driverless".to_owned()),
        disk_gib: u32::try_from(number("disk_gib", 64)).unwrap_or(64),
        hourly_usd: cloud
            .get("hourly_usd")
            .and_then(money)
            .ok_or_else(|| format!("set ai_cloud.hourly_usd in ~/.polaris/{SETTINGS_FILE}"))?,
        budget_usd: cloud
            .get("budget_usd")
            .and_then(money)
            .ok_or_else(|| format!("set ai_cloud.budget_usd in ~/.polaris/{SETTINGS_FILE}"))?,
        max_hours: number("max_hours", 4).max(1),
        idle_hours: number("idle_hours", 1).max(1),
        operator_ip: text("operator_ip").and_then(|ip| ip.parse().ok()),
    })
}

/// US dollars as an exact decimal string → micro-dollars.
pub(crate) fn usd_micros(text: &str) -> Option<u64> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    if fraction.len() > 6 || whole.is_empty() {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<6}").parse().ok()?
    };
    whole.checked_mul(1_000_000)?.checked_add(fraction)
}

/// The machine's owner label: one per Polaris directory, so `cloud-down` removes only its own VMs.
fn owner_value(global: &Path) -> String {
    let digest = blake3::hash(global.to_string_lossy().as_bytes());
    digest.to_hex().get(..16).unwrap_or_default().to_owned()
}

/// cloud-init for the gateway VM: Docker Engine and the pinned OpenShell gateway, listening on
/// loopback only (reach it through an SSH tunnel; the client mTLS files are copied with scp).
fn cloud_init() -> String {
    String::from(
        "#cloud-config\n\
users:\n  - name: polaris\n    groups: [sudo, docker]\n    shell: /bin/bash\n    sudo: ALL=(ALL) NOPASSWD:ALL\n\
package_update: true\npackages: [docker.io, curl, ca-certificates]\n\
runcmd:\n\
  - systemctl enable --now docker\n\
  - usermod -aG docker polaris\n\
  - loginctl enable-linger polaris\n\
  - sudo -u polaris -H sh -c 'curl -fsSL https://raw.githubusercontent.com/NVIDIA/OpenShell/v0.1.4/install.sh -o /tmp/openshell-install.sh && OPENSHELL_VERSION=v0.1.4 sh /tmp/openshell-install.sh'\n\
  - sudo -u polaris -H sh -c 'mkdir -p ~/.config/openshell && echo OPENSHELL_COMPUTE_DRIVER=docker >> ~/.config/openshell/gateway.env'\n\
  - sudo -u polaris XDG_RUNTIME_DIR=/run/user/$(id -u polaris) systemctl --user restart openshell-gateway || true\n",
    )
}

async fn nebius_cli(cloud: &CloudSettings, args: &[String]) -> Result<Value, SessionHostError> {
    let mut command = tokio::process::Command::new(&cloud.cli);
    let _command = command
        .args(args)
        .arg("--format")
        .arg("json")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(600), command.output())
        .await
        .map_err(|_| SessionHostError::Unavailable("the nebius CLI timed out"))?
        .map_err(|_| {
            SessionHostError::Ui(format!(
                "the `{}` CLI was not found (install it and run `nebius profile create`)",
                cloud.cli
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SessionHostError::Ui(format!(
            "nebius {}: {}",
            args.first().map_or("", String::as_str),
            bounded(stderr.trim(), 600)
        )));
    }
    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| SessionHostError::Ui("the nebius CLI did not answer JSON".to_owned()))
}

fn instances_of(listing: &Value) -> Vec<Value> {
    listing
        .get("items")
        .or_else(|| listing.get("instances"))
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| listing.as_array().cloned())
        .unwrap_or_default()
}

fn instance_line(instance: &Value) -> String {
    let get = |path: &[&str]| {
        path.iter()
            .try_fold(instance, |value, key| value.get(*key))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned()
    };
    let address = instance
        .pointer("/status/network_interfaces/0/public_ip_address/address")
        .and_then(Value::as_str)
        .unwrap_or("no public address yet");
    format!(
        "{}  {}  {}  {}",
        get(&["metadata", "name"]),
        get(&["metadata", "id"]),
        get(&["status", "state"]),
        address
    )
}

/// `polaris nebius cloud-up|cloud-status|cloud-down`: the remote OpenShell gateway VM (E4).
/// # Errors
/// AI Cloud is not configured, the budget or policy refuses the VM, or the CLI failed.
pub async fn cloud_lines(workspace: &Path, action: &str) -> Result<Vec<String>, SessionHostError> {
    let access = access(workspace).await?;
    let cloud = cloud_settings(&access.settings())
        .map_err(|reason| SessionHostError::Ui(format!("AI Cloud: {reason}")))?;
    let owner = owner_value(&access.global);
    let marker = OwnerMarker::new(OWNER_LABEL.to_owned(), owner.clone())
        .map_err(|_| SessionHostError::Unavailable("the owner label"))?;
    let listing = nebius_cli(
        &cloud,
        &[
            "compute".to_owned(),
            "instance".to_owned(),
            "list".to_owned(),
            "--parent-id".to_owned(),
            cloud.parent_id.clone(),
        ],
    )
    .await?;
    let instances = instances_of(&listing);
    let labelled = instances.iter().map(|instance| {
        let id = instance
            .pointer("/metadata/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let labels = instance
            .pointer("/metadata/labels")
            .and_then(Value::as_object)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        ResourceLabels::new(id, labels)
    });
    let owned: Vec<String> = select_owned_resources(labelled, &marker)
        .into_iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    let owned_instances: Vec<&Value> = instances
        .iter()
        .filter(|instance| {
            instance
                .pointer("/metadata/id")
                .and_then(Value::as_str)
                .is_some_and(|id| owned.iter().any(|own| own == id))
        })
        .collect();
    match action {
        "cloud-status" => {
            let mut lines = vec![format!(
                "{} gateway VM{} of this machine in {}:",
                owned_instances.len(),
                if owned_instances.len() == 1 { "" } else { "s" },
                cloud.parent_id
            )];
            lines.extend(
                owned_instances
                    .iter()
                    .map(|instance| instance_line(instance)),
            );
            Ok(lines)
        }
        "cloud-down" => {
            let mut lines = Vec::new();
            for id in &owned {
                let _deleted = nebius_cli(
                    &cloud,
                    &[
                        "compute".to_owned(),
                        "instance".to_owned(),
                        "delete".to_owned(),
                        "--id".to_owned(),
                        id.clone(),
                    ],
                )
                .await?;
                lines.push(format!("Deleted {id}"));
            }
            if lines.is_empty() {
                lines.push("No gateway VM of this machine to delete.".to_owned());
            }
            Ok(lines)
        }
        "cloud-up" => {
            if let Some(existing) = owned_instances.first() {
                return Ok(vec![
                    "A gateway VM of this machine already exists:".to_owned(),
                    instance_line(existing),
                ]);
            }
            let hourly = usd_micros(&cloud.hourly_usd)
                .ok_or_else(|| SessionHostError::Ui("AI Cloud: invalid hourly_usd".to_owned()))?;
            let limit = usd_micros(&cloud.budget_usd)
                .ok_or_else(|| SessionHostError::Ui("AI Cloud: invalid budget_usd".to_owned()))?;
            let max = Duration::from_secs(cloud.max_hours.saturating_mul(3600));
            let idle = Duration::from_secs(cloud.idle_hours.saturating_mul(3600));
            let ingress = IngressPolicy::new(cloud.operator_ip.into_iter().collect(), true)
                .map_err(|error| SessionHostError::Ui(format!("AI Cloud ingress: {error}")))?;
            let spec = ValidatedVmSpec::new(cloud.disk_gib, max, idle.min(max), ingress)
                .map_err(|error| SessionHostError::Ui(format!("AI Cloud VM spec: {error}")))?;
            let budget = BudgetGuard::new(limit);
            let reservation = budget.reserve(hourly, spec.hard_ttl()).map_err(|error| {
                SessionHostError::Ui(format!(
                    "AI Cloud: {error} (${}/h for {} h against ${})",
                    cloud.hourly_usd, cloud.max_hours, cloud.budget_usd
                ))
            })?;
            let name = format!("polaris-gateway-{}", bounded(&owner, 8));
            let interfaces = json!([{
                "name": "eth0",
                "subnet_id": cloud.subnet_id,
                "ip_address": {},
                "public_ip_address": {}
            }]);
            let created = nebius_cli(
                &cloud,
                &[
                    "compute".to_owned(),
                    "instance".to_owned(),
                    "create".to_owned(),
                    "--parent-id".to_owned(),
                    cloud.parent_id.clone(),
                    "--name".to_owned(),
                    name.clone(),
                    "--labels".to_owned(),
                    format!("{OWNER_LABEL}={owner}"),
                    "--resources-platform".to_owned(),
                    cloud.platform.clone(),
                    "--resources-preset".to_owned(),
                    cloud.preset.clone(),
                    "--boot-disk-attach-mode".to_owned(),
                    "READ_WRITE".to_owned(),
                    "--boot-disk-managed-disk-name".to_owned(),
                    format!("{name}-boot"),
                    "--boot-disk-managed-disk-type".to_owned(),
                    "NETWORK_SSD".to_owned(),
                    "--boot-disk-managed-disk-size-gibibytes".to_owned(),
                    spec.disk_gib().to_string(),
                    "--boot-disk-managed-disk-source-image-family-image-family".to_owned(),
                    cloud.image_family.clone(),
                    "--network-interfaces".to_owned(),
                    interfaces.to_string(),
                    "--cloud-init-user-data".to_owned(),
                    cloud_init(),
                ],
            )
            .await?;
            // The reservation covers the whole hard lifetime; `cloud-down` ends it earlier.
            let reserved = reservation.reserved_micros();
            let _settled = reservation.settle(reserved);
            let id = created
                .pointer("/metadata/id")
                .or_else(|| created.pointer("/resource_id"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            Ok(vec![
                format!(
                    "Gateway VM {name} ({id}) is starting: {} {}, {} GiB, at most {} h (${} reserved of ${})",
                    cloud.platform,
                    cloud.preset,
                    spec.disk_gib(),
                    cloud.max_hours,
                    cloud.hourly_usd,
                    cloud.budget_usd
                ),
                "When `polaris nebius cloud-status` shows its address:".to_owned(),
                "  ssh -N -L 17670:127.0.0.1:17670 polaris@<address>   (the gateway stays on the VM's loopback)".to_owned(),
                "  scp -r polaris@<address>:.config/openshell/gateways/openshell/mtls ~/.polaris/remote-mtls".to_owned(),
                "  then point ~/.polaris/sandbox.json at https://127.0.0.1:17670 with mtls_dir ~/.polaris/remote-mtls and no wsl_distro.".to_owned(),
                format!(
                    "Delete it with `polaris nebius cloud-down` (idle limit {} h is yours to keep).",
                    cloud.idle_hours
                ),
            ])
        }
        _ => Err(SessionHostError::Ui(format!(
            "unknown AI Cloud action {action}"
        ))),
    }
}

// ------------------------------------------------------------------------------------ CLI

/// `polaris nebius <command> …`: every Nebius action from the command line.
/// # Errors
/// An unknown command, missing arguments, or the command's own failure.
pub async fn cli_lines(
    workspace: &Path,
    words: &[String],
) -> Result<Vec<String>, SessionHostError> {
    let usage = || {
        SessionHostError::Ui(
            "usage: polaris nebius status | datalab-finetune [model] | contree-branches | contree-run [--sync] <image> <command>… | swe-run <owner/repo> <commit> <command> | cloud-up | cloud-status | cloud-down"
                .to_owned(),
        )
    };
    let (command, rest) = words.split_first().ok_or_else(usage)?;
    match command.as_str() {
        "status" => status_lines(workspace).await,
        "datalab-finetune" => {
            datalab_finetune_lines(workspace, rest.first().map(String::as_str)).await
        }
        "contree-branches" => contree_branches_lines(workspace).await,
        "contree-run" => {
            let (sync, rest) = match rest.split_first() {
                Some((flag, tail)) if flag == "--sync" => (true, tail),
                _ => (false, rest),
            };
            let (image, commands) = rest.split_first().ok_or_else(usage)?;
            if commands.is_empty() {
                return Err(usage());
            }
            contree_run_lines(workspace, image, commands, sync).await
        }
        "swe-run" => match rest {
            [repo, commit, command, ..] => swe_run_lines(workspace, repo, commit, command).await,
            _ => Err(usage()),
        },
        "cloud-up" | "cloud-status" | "cloud-down" => cloud_lines(workspace, command).await,
        _ => Err(usage()),
    }
}
