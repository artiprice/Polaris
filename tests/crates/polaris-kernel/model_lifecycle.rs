//! Synthetic local-server contracts for the production model process lifecycle.
use std::{
    error::Error,
    ffi::OsString,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::Duration,
};

use polaris_kernel::model_lifecycle::{
    LocalModelLifecycleConfig, LocalModelProcessConfig, LocalModelRuntime,
};
use polaris_models::{
    Availability, CapabilitySupport, CatalogProvenance, ModelEffort, ModelKind, ModelRegistry,
    ModelSpec, PriceTable, ProviderKind, ReasoningBudget, RegistryConfig, SlotConfig,
};
use polaris_nebius_cloud::token_factory::LlamaCppExpectedProfile;
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, PromptTokenizer},
};
use serde_json::json;

const PORT_ENV: &str = "POLARIS_SYNTHETIC_MODEL_PORT";
const ALIAS_ENV: &str = "POLARIS_SYNTHETIC_MODEL_ALIAS";
const BUILD_ENV: &str = "POLARIS_SYNTHETIC_MODEL_BUILD";
const CONTEXT_ENV: &str = "POLARIS_SYNTHETIC_MODEL_CONTEXT";
const STATUS_ENV: &str = "POLARIS_SYNTHETIC_MODEL_STATUS";
const HELPER_ENV: &str = "POLARIS_SYNTHETIC_MODEL_HELPER";
const STALL_ENV: &str = "POLARIS_SYNTHETIC_MODEL_STALL_FIRST";

/// Synthetic HTTP fixture used only when spawned by a lifecycle integration test.
#[test]
fn synthetic_llama_server_fixture() -> Result<(), Box<dyn Error>> {
    let Some(port) = std::env::var_os(PORT_ENV) else {
        return Ok(());
    };
    let alias = std::env::var(ALIAS_ENV)?;
    let build = std::env::var(BUILD_ENV)?;
    let context = std::env::var(CONTEXT_ENV)?.parse::<u32>()?;
    let status = std::env::var_os(STATUS_ENV).ok_or("fixture status path missing")?;
    let port = port.to_string_lossy().parse::<u16>()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    if std::env::var_os(HELPER_ENV).is_some() {
        let executable = std::env::current_exe()?;
        let mut helper = Command::new(executable);
        helper
            .args(["--exact", "synthetic_llama_server_fixture", "--nocapture"])
            .env(PORT_ENV, port.to_string())
            .env(ALIAS_ENV, &alias)
            .env(BUILD_ENV, &build)
            .env(CONTEXT_ENV, context.to_string())
            .env(STATUS_ENV, &status)
            .env_remove(HELPER_ENV);
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            helper.env("SystemRoot", system_root);
        }
        let child = match helper.spawn() {
            Ok(child) => child,
            Err(error) => {
                std::fs::write(&status, format!("helper-spawn-failed:{error}"))?;
                return Err(Box::new(error));
            }
        };
        let startup_deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(6))
            .ok_or("fixture deadline overflow")?;
        loop {
            if std::fs::read_to_string(&status).is_ok_and(|value| value.starts_with("bound:")) {
                break;
            }
            if std::time::Instant::now() >= startup_deadline {
                return Err("synthetic helper listener did not bind".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::write(
            status,
            format!("helper-listener:{}:{}", std::process::id(), child.id()),
        )?;
        loop {
            std::thread::park_timeout(Duration::from_secs(60));
        }
    }
    let listener = match TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(error) => {
            std::fs::write(status, error.to_string())?;
            return Err(Box::new(error));
        }
    };
    std::fs::write(&status, format!("bound:{}\n", std::process::id()))?;
    let mut stall_first = std::env::var_os(STALL_ENV).is_some();
    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request)?;
        let first_line = std::str::from_utf8(request.get(..read).ok_or("invalid request length")?)?
            .lines()
            .next()
            .ok_or("missing request line")?;
        let route = first_line
            .split_whitespace()
            .nth(1)
            .ok_or("missing route")?;
        let mut audit = std::fs::OpenOptions::new().append(true).open(&status)?;
        writeln!(audit, "route:{route}")?;
        if stall_first && route == "/props" {
            stall_first = false;
            // A deliberately stalled socket, with a real bound listener that continues to serve
            // new probes. Its thread belongs to this retained synthetic process tree.
            let _stalled = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                drop(stream);
            });
            continue;
        }
        let body = match route {
            "/props" => json!({
                "build_info": build,
                "model_alias": alias,
                "default_generation_settings": {"n_ctx": context},
                "chat_template_caps": {"supports_tools": true, "supports_tool_calls": true}
            })
            .to_string(),
            "/v1/models" => json!({
                "object": "list",
                "data": [{
                    "id": alias,
                    "object": "model",
                    "owned_by": "synthetic",
                    "meta": {"n_ctx": context},
                    "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}
                }]
            })
            .to_string(),
            "/v1/chat/completions/input_tokens" => {
                json!({"object": "response.input_tokens", "input_tokens": 42}).to_string()
            }
            "/v1/chat/completions" => concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"synthetic\"}}]}\n\n",
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":42,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
            _ => return Err(format!("unexpected synthetic route {route}").into()),
        };
        let content_type = if route == "/v1/chat/completions" {
            "text/event-stream"
        } else {
            "application/json"
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            content_type,
            body
        )?;
        stream.flush()?;
    }
    Ok(())
}

#[tokio::test]
async fn stalled_profile_probe_retries_within_probe_timeout() -> Result<(), Box<dyn Error>> {
    let alias = "synthetic-probe-model";
    let model = ModelId::new(alias)?;
    let port = reserve_loopback_port()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let directory = tempfile::tempdir()?;
    let status = directory.path().join("probe_status.txt");
    let mut spec = spawn_spec(
        std::env::current_exe()?,
        port,
        alias,
        "synthetic-build",
        4096,
        Some(&status),
    );
    spec.env
        .push((OsString::from(STALL_ENV), OsString::from("1")));
    let process = LocalModelProcessConfig::new(
        model.clone(),
        spec,
        address,
        format!("http://{address}/v1"),
        profile(alias)?,
    )?;
    let mut bounds = lifecycle_config();
    // The real Windows ownership probe launches netstat. Its budget must permit that process
    // under parallel tests, while remaining strictly below the fixture's four-second HTTP stall.
    // The three-second acceptance bound below is unchanged and still rejects the old loader.
    bounds.probe_timeout = Duration::from_secs(1);
    bounds.startup_timeout = Duration::from_secs(6);
    let runtime = LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry(model.clone())?,
        &[],
        vec![process],
        bounds,
    )?;
    let started = tokio::time::Instant::now();
    let selected = runtime
        .acquire_for_effort(Effort::Low, &[Effort::Low], std::slice::from_ref(&model))
        .await;
    let elapsed = started.elapsed();
    drop(selected?);
    let outcomes = runtime.close(Duration::from_secs(2)).await?;
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, polaris_models::EvictOutcome::Evicted)),
        "probe test process must be observed stopped"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "healthy retry was delayed by the stalled metadata response: {elapsed:?}"
    );
    let audit = std::fs::read_to_string(status)?;
    assert!(
        audit.matches("route:/props").count() >= 2,
        "a later profile probe must replace the stalled first response"
    );
    Ok(())
}

#[tokio::test]
async fn loader_rejects_non_loopback_and_mismatched_url_authority() -> Result<(), Box<dyn Error>> {
    let model = ModelId::new("synthetic-model")?;
    let profile = profile("synthetic-model")?;
    let spec = spawn_spec(
        std::env::current_exe()?,
        19001,
        "synthetic-model",
        "fixture",
        4096,
        None,
    );
    if LocalModelProcessConfig::new(
        model.clone(),
        spec.clone(),
        SocketAddr::from(([127, 0, 0, 1], 19001)),
        "http://127.0.0.2:19001/v1".to_owned(),
        profile.clone(),
    )
    .is_ok()
    {
        return Err("mismatched literal endpoint host was accepted".into());
    }
    if LocalModelProcessConfig::new(
        model.clone(),
        spec.clone(),
        SocketAddr::from(([127, 0, 0, 1], 19001)),
        "http://localhost:19001/v1".to_owned(),
        profile.clone(),
    )
    .is_ok()
    {
        return Err("DNS hostname endpoint was accepted".into());
    }
    for authority in [
        "http://user:secret@127.0.0.1:19001/v1",
        "http://127.0.0.1:19001/v1?route=other",
        "http://127.0.0.1:19002/v1",
    ] {
        if LocalModelProcessConfig::new(
            model.clone(),
            spec.clone(),
            SocketAddr::from(([127, 0, 0, 1], 19001)),
            authority.to_owned(),
            profile.clone(),
        )
        .is_ok()
        {
            return Err(format!("unsafe endpoint authority was accepted: {authority}").into());
        }
    }
    if LocalModelProcessConfig::new(
        model,
        spec,
        SocketAddr::from(([0, 0, 0, 0], 19001)),
        "http://0.0.0.0:19001/v1".to_owned(),
        profile,
    )
    .is_ok()
    {
        return Err("wildcard listener was accepted".into());
    }
    Ok(())
}

#[tokio::test]
async fn load_keeps_one_exact_adapter_and_lease_through_count_and_stream()
-> Result<(), Box<dyn Error>> {
    let alias = "synthetic-model";
    let port = reserve_loopback_port()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let directory = tempfile::tempdir()?;
    let status_path = directory.path().join("model_status.txt");
    let executable = std::env::current_exe()?;
    let model = ModelId::new(alias)?;
    let profile = profile(alias)?;
    let spec = spawn_spec(
        executable,
        port,
        alias,
        "synthetic-build",
        4096,
        Some(status_path.as_path()),
    );
    let process = LocalModelProcessConfig::new(
        model.clone(),
        spec,
        address,
        format!("http://{address}/v1"),
        profile,
    )?;
    let registry = registry(model.clone())?;
    let runtime = Arc::new(LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry,
        &[],
        vec![process],
        lifecycle_config(),
    )?);
    let waiter_runtime = Arc::clone(&runtime);
    let waiter_model = model.clone();
    let waiter = tokio::spawn(async move {
        waiter_runtime
            .acquire_for_effort(
                Effort::Low,
                &[Effort::Low],
                std::slice::from_ref(&waiter_model),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::read_to_string(status_path.as_path())
                .is_ok_and(|status| status.starts_with("bound:"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    waiter.abort();
    let _aborted_waiter = waiter.await;
    let selected = runtime
        .acquire_for_effort(Effort::Low, &[Effort::Low], std::slice::from_ref(&model))
        .await?;
    let mut wrong_model = request(ModelId::new("foreign-synthetic-model")?);
    if selected.count_prompt(&wrong_model).await.is_ok() {
        return Err("tokenizer accepted a different request model id".into());
    }
    if selected.stream(wrong_model.clone()).await.is_ok() {
        return Err("chat accepted a different request model id".into());
    }
    wrong_model.model = model.clone();
    wrong_model.effort = Effort::High;
    if selected.count_prompt(&wrong_model).await.is_ok() {
        return Err("tokenizer accepted a different request effort".into());
    }
    if selected.stream(wrong_model).await.is_ok() {
        return Err("chat accepted a different request effort".into());
    }
    let routes = std::fs::read_to_string(status_path.as_path())?;
    if routes.contains("/v1/chat/completions/input_tokens")
        || routes.contains("route:/v1/chat/completions\n")
    {
        return Err("mismatched model requests reached the HTTP adapter".into());
    }
    let request = request(model.clone());
    let count = selected.count_prompt(&request).await?;
    if count.input_tokens != TokenCount::new(42)
        || count.max_context_tokens != TokenCount::new(4096)
    {
        return Err("synthetic tokenizer response was not preserved".into());
    }
    let mut stream = selected.stream(request).await?;
    drop(selected);
    if !matches!(
        runtime.slot_state(&model)?,
        polaris_models::SlotState::Active { leases: 1 }
    ) {
        return Err("stream did not retain its model lease".into());
    }
    let mut saw_text = false;
    while let Some(chunk) = stream.next_chunk().await {
        if let polaris_sdk::registry::ChatChunk::Text(text) = chunk?
            && text == "synthetic"
        {
            saw_text = true;
        }
    }
    if !saw_text {
        return Err("synthetic chat response was not streamed".into());
    }
    if !matches!(
        runtime.slot_state(&model)?,
        polaris_models::SlotState::Active { leases: 0 }
    ) {
        return Err("clean EOF retained the model lease in the still-live stream value".into());
    }
    let closed = runtime.close(Duration::from_secs(2)).await?;
    if closed
        .iter()
        .any(|outcome| !matches!(outcome, polaris_models::EvictOutcome::Evicted))
    {
        return Err("local model process did not report observed eviction".into());
    }
    drop(stream);
    Ok(())
}

#[tokio::test]
async fn failed_profile_startup_stops_the_owned_process_tree() -> Result<(), Box<dyn Error>> {
    let alias = "expected-synthetic-model";
    let port = reserve_loopback_port()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let directory = tempfile::tempdir()?;
    let status_path = directory.path().join("failed_model_status.txt");
    let model = ModelId::new(alias)?;
    let config = LocalModelProcessConfig::new(
        model.clone(),
        spawn_spec(
            std::env::current_exe()?,
            port,
            "wrong-advertised-alias",
            "synthetic-build",
            4096,
            Some(status_path.as_path()),
        ),
        address,
        format!("http://{address}/v1"),
        profile(alias)?,
    )?;
    let mut bounds = lifecycle_config();
    bounds.startup_timeout = Duration::from_secs(2);
    bounds.slot.load_timeout = Duration::from_secs(3);
    let runtime = LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry(model.clone())?,
        &[],
        vec![config],
        bounds,
    )?;
    let result = runtime
        .acquire_for_effort(Effort::Low, &[Effort::Low], std::slice::from_ref(&model))
        .await;
    if result.is_ok() {
        return Err("mismatched advertised model profile was accepted".into());
    }
    let status = std::fs::read_to_string(status_path).map_err(|error| {
        std::io::Error::other(format!("helper fixture status was unavailable: {error}"))
    })?;
    let pid_text = status
        .lines()
        .next()
        .ok_or("synthetic server status was empty")?
        .strip_prefix("bound:")
        .ok_or("synthetic server did not report a bound listener")?;
    let pid = pid_text.parse::<u32>().map_err(|error| {
        std::io::Error::other(format!(
            "invalid synthetic process status {status:?}: {error}"
        ))
    })?;
    let target = sysinfo::Pid::from_u32(pid);
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(4))
        .ok_or("fixture deadline overflow")?;
    loop {
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
        if system.process(target).is_none() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("failed model startup left its owned process running".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

#[tokio::test]
async fn helper_process_listener_fails_closed_and_tree_cleanup_reaps_both()
-> Result<(), Box<dyn Error>> {
    let alias = "synthetic-helper-model";
    let port = reserve_loopback_port()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let directory = tempfile::tempdir()?;
    let status_path = directory.path().join("helper_model_status.txt");
    let model = ModelId::new(alias)?;
    let mut spec = spawn_spec(
        std::env::current_exe()?,
        port,
        alias,
        "synthetic-build",
        4096,
        Some(status_path.as_path()),
    );
    spec.env
        .push((OsString::from(HELPER_ENV), OsString::from("1")));
    let process = LocalModelProcessConfig::new(
        model.clone(),
        spec,
        address,
        format!("http://{address}/v1"),
        profile(alias)?,
    )?;
    let mut bounds = lifecycle_config();
    bounds.startup_timeout = Duration::from_secs(8);
    bounds.slot.load_timeout = Duration::from_secs(9);
    let runtime = LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry(model.clone())?,
        &[],
        vec![process],
        bounds,
    )?;
    let result = runtime
        .acquire_for_effort(Effort::Low, &[Effort::Low], std::slice::from_ref(&model))
        .await;
    if result.is_ok() {
        return Err("listener owned by a helper process was accepted".into());
    }
    let status = std::fs::read_to_string(status_path).map_err(|error| {
        std::io::Error::other(format!("helper fixture status was unavailable: {error}"))
    })?;
    let identities = status
        .strip_prefix("helper-listener:")
        .ok_or("synthetic helper did not report both process ids")?;
    let mut pids = identities.split(':').map(str::parse::<u32>);
    let root_pid = pids.next().ok_or("missing synthetic root pid")??;
    let helper_pid = pids.next().ok_or("missing synthetic helper pid")??;
    if pids.next().is_some() {
        return Err("synthetic process status had extra fields".into());
    }
    let targets = [
        sysinfo::Pid::from_u32(root_pid),
        sysinfo::Pid::from_u32(helper_pid),
    ];
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(4))
        .ok_or("fixture deadline overflow")?;
    loop {
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&targets), true);
        if targets.iter().all(|pid| system.process(*pid).is_none()) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("helper listener failure left a process-tree member running".into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(())
}

#[tokio::test]
async fn configured_subagent_override_uses_the_existing_model_registry()
-> Result<(), Box<dyn Error>> {
    let first = ModelId::new("synthetic-first")?;
    let second = ModelId::new("synthetic-second")?;
    let mut models = vec![model_spec(first.clone()), model_spec(second.clone())];
    models.sort_by(|left, right| left.id.cmp(&right.id));
    let registry = ModelRegistry::with_config(RegistryConfig {
        models,
        aliases: Vec::new(),
        efforts: polaris_config::EffortModels {
            low: first.clone(),
            medium: first.clone(),
            high: first.clone(),
            xhigh: first.clone(),
            max: first.clone(),
        },
    })?;
    let port_first = reserve_loopback_port()?;
    let port_second = reserve_loopback_port()?;
    let processes = vec![
        process_config(first.clone(), port_first)?,
        process_config(second.clone(), port_second)?,
    ];
    let runtime = LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry,
        &[ModelEffort {
            model: second.to_string(),
            min: Effort::High,
            max: Effort::Max,
            reasoning: ReasoningBudget::Default,
        }],
        processes,
        lifecycle_config(),
    )?;
    let choice =
        runtime.choice_for_effort(Effort::High, &[Effort::High], std::slice::from_ref(&second))?;
    if choice.model != second {
        return Err("configured subagent override was not applied".into());
    }
    Ok(())
}

/// Runs only against the separately provenance-pinned local llama.cpp/Qwen files.
/// The existing smoke server on port 18931 is deliberately outside this test's scope.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires provenance-pinned local llama.cpp/Qwen runtime files"]
async fn pinned_llama_loader_runs_and_drains_real_qwen_smoke_profile() -> Result<(), Box<dyn Error>>
{
    use std::{collections::BTreeSet, ffi::OsStr, net::TcpStream, time::Instant};

    use polaris_sdk::registry::{ChatChunk, Effects, ToolSpec, Usage};

    const PORT: u16 = 18_932;
    const ALIAS: &str = "qwen2.5-1.5b-instruct";
    const BUILD: &str = "b11804-c7fc50941";
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let executable = root.join("@prime/tools/runtime/llama-b11804/llama-server.exe");
    let weights = root.join("@prime/tools/runtime/qwen2.5-1.5b-instruct-q4_k_m.gguf");
    if !executable.is_file() || !weights.is_file() {
        return Err("pinned llama.cpp executable or Qwen weights are missing".into());
    }
    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    if TcpListener::bind(address).is_err() {
        return Err("dedicated acceptance port 18932 is already occupied".into());
    }
    let mut before_system = sysinfo::System::new_all();
    before_system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let preexisting: BTreeSet<u32> = before_system
        .processes()
        .iter()
        .filter(|(_, process)| process.name() == OsStr::new("llama-server.exe"))
        .map(|(pid, _)| pid.as_u32())
        .collect();

    let model = ModelId::new(ALIAS)?;
    let spec = polaris_sdk::registry::SpawnSpec {
        program: executable.clone(),
        args: [
            "--host",
            "127.0.0.1",
            "--port",
            "18932",
            "--threads",
            "2",
            "--threads-batch",
            "2",
            "--ctx-size",
            "4096",
            "--parallel",
            "1",
            "--jinja",
            "--no-webui",
            "--alias",
            ALIAS,
            "--model",
        ]
        .into_iter()
        .map(OsString::from)
        .chain(std::iter::once(weights.as_os_str().to_owned()))
        .collect(),
        cwd: executable.parent().map(PathBuf::from),
        env: std::env::var_os("SystemRoot")
            .map(|system_root| vec![(OsString::from("SystemRoot"), system_root)])
            .unwrap_or_default(),
    };
    let process = LocalModelProcessConfig::new(
        model.clone(),
        spec,
        address,
        format!("http://{address}/v1"),
        LlamaCppExpectedProfile::new(BUILD, ALIAS, TokenCount::new(4096))?,
    )?;
    let mut bounds = lifecycle_config();
    bounds.startup_timeout = Duration::from_secs(180);
    bounds.probe_timeout = Duration::from_secs(10);
    bounds.request_timeout = Duration::from_secs(180);
    bounds.stop_grace = Duration::from_secs(5);
    bounds.slot.load_timeout = Duration::from_secs(190);
    bounds.slot.unload_timeout = Duration::from_secs(15);
    let runtime = LocalModelRuntime::new(
        Arc::new(polaris_platform::process::ProcessPlatform::new()),
        registry(model.clone())?,
        &[],
        vec![process],
        bounds,
    )?;
    let selected = runtime
        .acquire_for_effort(Effort::Low, &[Effort::Low], std::slice::from_ref(&model))
        .await?;
    let mut probe = request(model.clone());
    probe.messages = vec![
        ChatMessage::text(
            ChatRole::System,
            "Call the function with probe_id exactly as requested.".to_owned(),
        ),
        ChatMessage::text(
            ChatRole::User,
            "Invoke p3_local_probe with probe_id=adapter-smoke.".to_owned(),
        ),
    ];
    probe.tools = vec![ToolSpec {
        name: "p3_local_probe".into(),
        description: "Return the requested probe identifier.".into(),
        input_schema_json: json!({
            "type": "object",
            "properties": {"probe_id": {"type": "string"}},
            "required": ["probe_id"]
        })
        .to_string()
        .into(),
        effects: Effects::READ_ONLY,
    }];
    probe.max_output_tokens = Some(TokenCount::new(64));
    let count = selected.count_prompt(&probe).await?;
    if count.input_tokens == TokenCount::new(0)
        || count.input_tokens > count.max_context_tokens
        || count.max_context_tokens != TokenCount::new(4096)
    {
        return Err("real adapter returned an invalid prompt count/context".into());
    }
    let mut stream = selected.stream(probe).await?;
    drop(selected);
    let mut tool_name = None;
    let mut tool_arguments = String::new();
    let mut usage: Option<Usage> = None;
    while let Some(chunk) = stream.next_chunk().await {
        match chunk? {
            ChatChunk::ToolCallDelta {
                index,
                name,
                arguments_fragment,
                ..
            } => {
                if index != 0 {
                    return Err("real smoke response produced an unexpected tool index".into());
                }
                if let Some(name) = name
                    && tool_name.replace(name).is_some()
                {
                    return Err("real smoke response repeated the tool name".into());
                }
                tool_arguments.push_str(&arguments_fragment);
            }
            ChatChunk::Usage(report) => usage = Some(report),
            _ => {}
        }
    }
    if tool_name.as_deref() != Some("p3_local_probe") {
        return Err("real adapter did not preserve the structured tool call".into());
    }
    let arguments: serde_json::Value = serde_json::from_str(&tool_arguments)?;
    if arguments
        .get("probe_id")
        .and_then(serde_json::Value::as_str)
        != Some("adapter-smoke")
    {
        return Err("real structured tool arguments did not preserve probe_id".into());
    }
    let usage = usage.ok_or("real response omitted prompt usage")?;
    if usage.prompt_tokens != count.input_tokens {
        return Err(format!(
            "exact adapter token count {} differed from chat usage {}",
            count.input_tokens.get(),
            usage.prompt_tokens.get()
        )
        .into());
    }
    drop(stream);

    let mut after_load = sysinfo::System::new_all();
    after_load.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let owned: BTreeSet<u32> = after_load
        .processes()
        .iter()
        .filter(|(_, process)| process.name() == OsStr::new("llama-server.exe"))
        .map(|(pid, _)| pid.as_u32())
        .filter(|pid| !preexisting.contains(pid))
        .collect();
    if owned.len() != 1 {
        let _ = runtime.close(Duration::from_secs(15)).await;
        return Err(
            "acceptance could not identify exactly one newly owned llama-server process".into(),
        );
    }
    let owned_pid = *owned.iter().next().ok_or("owned pid set became empty")?;
    let outcomes = runtime.close(Duration::from_secs(15)).await?;
    if outcomes
        .iter()
        .any(|outcome| !matches!(outcome, polaris_models::EvictOutcome::Evicted))
    {
        return Err("pinned local model close did not observe its process exit".into());
    }
    let target = sysinfo::Pid::from_u32(owned_pid);
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .ok_or("acceptance deadline overflow")?;
    loop {
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
        let process_gone = system.process(target).is_none();
        let listener_gone =
            TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err();
        if process_gone && listener_gone {
            break;
        }
        if Instant::now() >= deadline {
            return Err("owned llama process or port 18932 listener remained after close".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut after_close = sysinfo::System::new_all();
    after_close.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    if preexisting.iter().any(|pid| {
        let existing = sysinfo::Pid::from_u32(*pid);
        after_close.process(existing).is_none()
    }) {
        return Err("a preexisting llama-server process disappeared during acceptance".into());
    }
    Ok(())
}

fn reserve_loopback_port() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    Ok(listener.local_addr()?.port())
}

fn spawn_spec(
    executable: PathBuf,
    port: u16,
    alias: &str,
    build: &str,
    context: u32,
    status: Option<&std::path::Path>,
) -> polaris_sdk::registry::SpawnSpec {
    let mut env = vec![
        (OsString::from(PORT_ENV), OsString::from(port.to_string())),
        (OsString::from(ALIAS_ENV), OsString::from(alias)),
        (OsString::from(BUILD_ENV), OsString::from(build)),
        (
            OsString::from(CONTEXT_ENV),
            OsString::from(context.to_string()),
        ),
    ];
    if let Some(status) = status {
        env.push((OsString::from(STATUS_ENV), status.as_os_str().to_owned()));
    }
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        env.push((OsString::from("SystemRoot"), system_root));
    }
    polaris_sdk::registry::SpawnSpec {
        program: executable,
        args: vec![
            OsString::from("--exact"),
            OsString::from("synthetic_llama_server_fixture"),
            OsString::from("--nocapture"),
        ],
        cwd: std::env::current_dir().ok(),
        env,
    }
}

fn profile(alias: &str) -> Result<LlamaCppExpectedProfile, Box<dyn Error>> {
    Ok(LlamaCppExpectedProfile::new(
        "synthetic-build",
        alias,
        TokenCount::new(4096),
    )?)
}

fn model_spec(id: ModelId) -> ModelSpec {
    ModelSpec {
        id,
        kind: ModelKind::Chat,
        provider: ProviderKind::OpenAiCompatible,
        context_tokens: Some(4096),
        tools: CapabilitySupport::Unknown,
        reasoning: CapabilitySupport::Unknown,
        price: None::<PriceTable>,
        provenance: CatalogProvenance::Configured,
        availability: Availability::Unknown,
        availability_observed_at: None,
    }
}

fn registry(model: ModelId) -> Result<ModelRegistry, polaris_models::ModelsError> {
    ModelRegistry::with_config(RegistryConfig {
        models: vec![model_spec(model.clone())],
        aliases: Vec::new(),
        efforts: polaris_config::EffortModels {
            low: model.clone(),
            medium: model.clone(),
            high: model.clone(),
            xhigh: model.clone(),
            max: model,
        },
    })
}

fn process_config(model: ModelId, port: u16) -> Result<LocalModelProcessConfig, Box<dyn Error>> {
    let profile = profile(model.as_str())?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let spec = spawn_spec(
        std::env::current_exe()?,
        port,
        model.as_str(),
        "synthetic-build",
        4096,
        None,
    );
    Ok(LocalModelProcessConfig::new(
        model,
        spec,
        address,
        format!("http://{address}/v1"),
        profile,
    )?)
}

fn lifecycle_config() -> LocalModelLifecycleConfig {
    LocalModelLifecycleConfig {
        startup_timeout: Duration::from_secs(8),
        probe_timeout: Duration::from_secs(2),
        retry_interval: Duration::from_millis(50),
        request_timeout: Duration::from_secs(10),
        stop_grace: Duration::from_millis(250),
        slot: SlotConfig {
            idle_ttl: Duration::from_secs(60),
            max_leases: 4,
            load_timeout: Duration::from_secs(8),
            unload_timeout: Duration::from_secs(2),
        },
    }
}

fn request(model: ModelId) -> ChatRequest {
    ChatRequest {
        model,
        messages: vec![ChatMessage::text(
            ChatRole::User,
            "synthetic test".to_owned(),
        )],
        tools: Vec::new(),
        max_output_tokens: Some(TokenCount::new(8)),
        effort: Effort::Low,
    }
}
