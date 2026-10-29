// Unit tests of apps/polaris-cli/src/task.rs (`tests`); compiled into that module via #[path].
use super::{MAX_PROFILE_BYTES, SemanticCoverage, TaskProfile, TaskSummary};
use std::{fs, io::Write, path::Path};
use tokio_util::sync::CancellationToken;

#[test]
fn task_profile_rejects_unknown_fields_before_host_start() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let profile_path = dir.path().join("profile.toml");
    let mut file = fs::File::create(&profile_path)?;
    file.write_all(b"version = 1\nunknown = true\n")?;
    assert!(TaskProfile::read(&profile_path, &CancellationToken::new()).is_err());
    Ok(())
}

#[test]
fn task_profile_rejects_oversized_toml_before_parse() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let profile_path = dir.path().join("oversized.toml");
    let mut file = fs::File::create(&profile_path)?;
    let bytes = vec![b' '; usize::try_from(MAX_PROFILE_BYTES.saturating_add(1))?];
    file.write_all(&bytes)?;
    assert!(TaskProfile::read(&profile_path, &CancellationToken::new()).is_err());
    Ok(())
}

#[test]
fn task_profile_accepts_existing_pinned_local_assets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let paths = create_assets(dir.path())?;
    let source = profile_source(&paths)?;
    let profile_path = dir.path().join("profile.toml");
    fs::write(&profile_path, source)?;
    let profile = TaskProfile::read(&profile_path, &CancellationToken::new())?;
    assert!(profile.llama_executable.is_absolute());
    assert!(profile.llama_model.is_file());
    Ok(())
}

#[test]
fn task_profile_rejects_changed_pinned_model_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let paths = create_assets(dir.path())?;
    let source = profile_source(&paths)?;
    let profile_path = dir.path().join("profile.toml");
    fs::write(&profile_path, source)?;
    fs::write(&paths.llama_model, b"changed weights")?;
    assert!(TaskProfile::read(&profile_path, &CancellationToken::new()).is_err());
    Ok(())
}

#[test]
fn task_profile_rejects_missing_host_paths() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let paths = create_assets(dir.path())?;
    let mut source = profile_source(&paths)?;
    let existing =
        toml::Value::String(paths.llama_model.to_string_lossy().into_owned()).to_string();
    let missing = toml::Value::String(
        dir.path()
            .join("missing.gguf")
            .to_string_lossy()
            .into_owned(),
    )
    .to_string();
    source = source.replace(&existing, &missing);
    let profile_path = dir.path().join("profile.toml");
    fs::write(&profile_path, source)?;
    assert!(TaskProfile::read(&profile_path, &CancellationToken::new()).is_err());
    Ok(())
}

#[test]
fn task_summary_serializes_only_sanitized_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let summary = TaskSummary {
        schema_version: 1,
        execution_status: "finished",
        proof_status: "unverified",
        session_persistence: "new_local_no_resume",
        attempts: 1,
        retries: 0,
        tool_rounds: 0,
        stop_reason: "ModelTurnComplete".to_owned(),
        semantic_privacy: Some(SemanticCoverage {
            ner_fields: 2,
            degraded_fields: 1,
        }),
        system_one: "degraded",
        provider: Some(super::ProviderSummary {
            provider: "openrouter",
            model: "nvidia/nemotron-3.5-lightning:free".to_owned(),
            skipped: vec![super::SkippedSummary {
                provider: "token_factory",
                reason: "API key is not set",
            }],
        }),
        rights: vec!["read"],
        answer: Some("synthetic model answer".to_owned()),
    };
    let value = serde_json::to_value(summary)?;
    assert!(value.get("assistant_text").is_none());
    assert!(value.get("tool_arguments").is_none());
    assert!(
        value.get("answer").is_none(),
        "answer text never enters the JSON"
    );
    assert!(!value.to_string().contains("synthetic model answer"));
    assert_eq!(
        value.get("system_one").and_then(serde_json::Value::as_str),
        Some("degraded")
    );
    assert_eq!(
        value
            .pointer("/provider/model")
            .and_then(serde_json::Value::as_str),
        Some("nvidia/nemotron-3.5-lightning:free")
    );
    assert_eq!(
        value
            .get("proof_status")
            .and_then(serde_json::Value::as_str),
        Some("unverified")
    );
    assert_eq!(
        value
            .get("session_persistence")
            .and_then(serde_json::Value::as_str),
        Some("new_local_no_resume")
    );
    Ok(())
}

struct TestAssets {
    llama_executable: std::path::PathBuf,
    llama_model: std::path::PathBuf,
    s1_worker: std::path::PathBuf,
    decide_dir: std::path::PathBuf,
    pii_dir: std::path::PathBuf,
    ort: std::path::PathBuf,
    endpoint: std::path::PathBuf,
}

fn create_assets(root: &Path) -> Result<TestAssets, Box<dyn std::error::Error>> {
    let llama_executable = root.join("llama-server.exe");
    let llama_model = root.join("model.gguf");
    let s1_worker = root.join("s1-worker.exe");
    let decide_dir = root.join("decide");
    let pii_dir = root.join("pii");
    let ort = root.join("onnxruntime.dll");
    fs::write(&llama_executable, b"llama server")?;
    fs::write(&llama_model, b"model weights")?;
    fs::write(&s1_worker, b"s1 worker")?;
    fs::create_dir(&decide_dir)?;
    fs::create_dir(&pii_dir)?;
    fs::write(&ort, b"runtime")?;
    Ok(TestAssets {
        llama_executable,
        llama_model,
        s1_worker,
        decide_dir,
        pii_dir,
        ort,
        endpoint: root.join("s1-worker.sock"),
    })
}

fn profile_source(assets: &TestAssets) -> Result<String, Box<dyn std::error::Error>> {
    let exe_hash = blake3::hash(&fs::read(&assets.llama_executable)?)
        .to_hex()
        .to_string();
    let model_hash = blake3::hash(&fs::read(&assets.llama_model)?)
        .to_hex()
        .to_string();
    let field = |path: &Path| toml::Value::String(path.to_string_lossy().into_owned()).to_string();
    Ok(format!(
        "version = 1\nllama_executable = {}\nllama_executable_blake3 = \"{}\"\nllama_model = {}\nllama_model_blake3 = \"{}\"\ns1_worker = {}\ns1_decide_model_dir = {}\ns1_pii_model_dir = {}\nort_dylib = {}\ns1_endpoint = {}\nmodel_endpoint = \"127.0.0.1:19001\"\nmodel_alias = \"test-local\"\nllama_build = \"test-build.1\"\ncontext_tokens = 4096\neffort = \"medium\"\nmax_steps = 4\ntoken_budget = 2048\n",
        field(&assets.llama_executable),
        exe_hash,
        field(&assets.llama_model),
        model_hash,
        field(&assets.s1_worker),
        field(&assets.decide_dir),
        field(&assets.pii_dir),
        field(&assets.ort),
        field(&assets.endpoint),
    ))
}

#[tokio::test]
#[ignore = "requires a trusted profile with provenance-pinned local llama and S1 assets"]
async fn task_entry_uses_local_model_and_reports_independent_proof()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(profile) = std::env::var_os("POLARIS_TASK_PROFILE") else {
        return Err("POLARIS_TASK_PROFILE must name a trusted TOML profile".into());
    };
    let workspace = std::env::current_dir()?;
    let state = tempfile::tempdir()?;
    let config = polaris_config::load(&[], &polaris_config::TrustGrant::none())?.config;
    let summary = super::run(
        state.path(),
        &workspace,
        Some(Path::new(&profile)),
        "Inspect the current workspace and report a short summary.",
        &config,
        None,
    )
    .await?;
    assert_eq!(summary.execution_status, "finished");
    assert!(matches!(summary.proof_status, "verified" | "unverified"));
    assert_eq!(summary.session_persistence, "new_local_no_resume");
    assert_eq!(summary.rights, ["read"]);
    Ok(())
}

/// Live provider-chain run; reads `OPENROUTER_API_KEY` from the repository `.env`.
#[tokio::test]
#[ignore = "requires network access and a provider key in the repository .env"]
async fn hosted_task_runs_through_the_provider_chain_without_s1()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env_file = repo.join(".env");
    let state = tempfile::tempdir()?;
    let config = polaris_config::load(&[], &polaris_config::TrustGrant::none())?.config;
    let summary = super::run(
        state.path(),
        &repo.join("crates/polaris-tools"),
        None,
        "Read Cargo.toml in this workspace and state the package name in one sentence.",
        &config,
        Some(&env_file),
    )
    .await?;
    assert_eq!(summary.execution_status, "finished");
    assert_eq!(summary.system_one, "degraded");
    assert_eq!(summary.rights, ["read"]);
    let provider = summary.provider.ok_or("provider summary")?;
    assert!(matches!(
        provider.provider,
        "token_factory" | "openrouter" | "local"
    ));
    let answer = summary.answer.ok_or("final answer")?;
    assert!(
        answer.contains("polaris-tools"),
        "the model read the file through the read-only tools"
    );
    Ok(())
}
