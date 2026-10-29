use std::path::PathBuf;

use polaris_proto::worker_auth::{WorkerAuthenticator, WorkerCredential};
use polaris_system_one_engine::{EngineConfig, PiiEngineConfig, TextNormalization};
use secrecy::SecretString;
use tokio::sync::watch;
use zeroize::Zeroizing;

const RUNTIME_SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_worker());
    shutdown_runtime(runtime, RUNTIME_SHUTDOWN_WAIT);
    result
}

async fn run_worker() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = parse_options(&arguments)?;
    let mode = required(&options, "mode")?;
    let credential = read_worker_credential()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_stdin_shutdown_monitor(shutdown_tx.clone())?;
    let signal_tx = shutdown_tx.clone();
    let signal_task = tokio::spawn(async move {
        let result = tokio::signal::ctrl_c().await;
        let _ = signal_tx.send(true);
        result
    });
    #[cfg(windows)]
    {
        let pipe = required(&options, "pipe")?.to_owned();
        match mode {
            "s1" => {
                let service = s1_worker(&options, shutdown_tx)?;
                let lifecycle = service.clone();
                let authenticator = WorkerAuthenticator::new(&credential);
                let served =
                    polaris_worker::serve_named_pipe(pipe, service, authenticator, shutdown_rx)
                        .await;
                let drained = lifecycle
                    .drain_local(std::time::Duration::from_secs(30))
                    .await;
                served?;
                drained?;
            }
            "ingest" => {
                let service = polaris_worker::IngestWorker::open(
                    PathBuf::from(required(&options, "cas-root")?),
                    shutdown_tx,
                )?;
                let authenticator = WorkerAuthenticator::new(&credential);
                polaris_worker::serve_named_pipe(pipe, service, authenticator, shutdown_rx).await?;
            }
            _ => return Err("supported worker modes are --mode s1 and --mode ingest".into()),
        }
    }
    #[cfg(unix)]
    {
        let socket = PathBuf::from(required(&options, "socket")?);
        match mode {
            "s1" => {
                let service = s1_worker(&options, shutdown_tx)?;
                let lifecycle = service.clone();
                let authenticator = WorkerAuthenticator::new(&credential);
                let served =
                    polaris_worker::serve_unix(socket, service, authenticator, shutdown_rx).await;
                let drained = lifecycle
                    .drain_local(std::time::Duration::from_secs(30))
                    .await;
                served?;
                drained?;
            }
            "ingest" => {
                let service = polaris_worker::IngestWorker::open(
                    PathBuf::from(required(&options, "cas-root")?),
                    shutdown_tx,
                )?;
                let authenticator = WorkerAuthenticator::new(&credential);
                polaris_worker::serve_unix(socket, service, authenticator, shutdown_rx).await?;
            }
            _ => return Err("supported worker modes are --mode s1 and --mode ingest".into()),
        }
    }
    if signal_task.is_finished() {
        signal_task.await??;
    } else {
        signal_task.abort();
        let _ = signal_task.await;
    }
    Ok(())
}

fn read_worker_credential() -> Result<WorkerCredential, Box<dyn std::error::Error + Send + Sync>> {
    read_worker_credential_from(&mut std::io::stdin())
}

fn read_worker_credential_from(
    reader: &mut impl std::io::Read,
) -> Result<WorkerCredential, Box<dyn std::error::Error + Send + Sync>> {
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.try_reserve_exact(64)?;
    bytes.resize(64, 0);
    reader.read_exact(&mut bytes)?;
    let text = std::str::from_utf8(&bytes)?;
    let mut secret = Zeroizing::new(String::new());
    secret.try_reserve_exact(64)?;
    secret.push_str(text);
    Ok(WorkerCredential::from_secret(SecretString::from(
        std::mem::take(&mut *secret),
    ))?)
}

fn spawn_stdin_shutdown_monitor(
    shutdown: watch::Sender<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::thread::Builder::new()
        .name("polaris-worker-stdin-shutdown".to_owned())
        .spawn(move || {
            use std::io::Read;

            let mut trailing = [0_u8; 1];
            let mut stdin = std::io::stdin();
            // The parent keeps this inherited pipe open after writing the 64-byte token. An extra
            // byte or EOF means the bootstrap channel is no longer valid and requests shutdown.
            let _ = stdin.read(&mut trailing);
            let _ = shutdown.send(true);
        })?;
    Ok(())
}

fn parse_options(
    arguments: &[String],
) -> Result<std::collections::HashMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut options = std::collections::HashMap::new();
    if arguments.len() > 16 || !arguments.len().is_multiple_of(2) {
        return Err("worker accepts at most eight option/value pairs".into());
    }
    let mut arguments = arguments.iter();
    while let Some(key) = arguments.next() {
        if !key.starts_with("--") {
            return Err(format!("unexpected argument: {key}").into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        if value.is_empty() || value.len() > 4096 {
            return Err(format!("invalid value length for {key}").into());
        }
        let key = key.strip_prefix("--").ok_or("invalid option")?;
        if !matches!(
            key,
            "mode"
                | "pipe"
                | "socket"
                | "cas-root"
                | "model-dir"
                | "ort-dylib"
                | "normalization"
                | "idle-ttl-ms"
                | "pii-model-dir"
        ) {
            return Err(format!("unknown option --{key}").into());
        }
        if options.insert(key.to_owned(), value.clone()).is_some() {
            return Err(format!("duplicate option --{key}").into());
        }
    }
    Ok(options)
}

fn engine_config(
    options: &std::collections::HashMap<String, String>,
) -> Result<EngineConfig, Box<dyn std::error::Error + Send + Sync>> {
    let model_dir = PathBuf::from(required(options, "model-dir")?);
    let ort_dylib = PathBuf::from(required(options, "ort-dylib")?);
    let normalization = match required(options, "normalization")? {
        "raw" => TextNormalization::Raw,
        "gliner2-python-2.1" => TextNormalization::Gliner2Python21,
        _ => return Err("--normalization must be raw or gliner2-python-2.1".into()),
    };
    Ok(EngineConfig::new(model_dir, ort_dylib, normalization))
}

fn s1_worker(
    options: &std::collections::HashMap<String, String>,
    shutdown: watch::Sender<bool>,
) -> Result<polaris_worker::S1Worker, Box<dyn std::error::Error + Send + Sync>> {
    let decide = engine_config(options)?;
    let idle = idle_policy(options)?;
    match options.get("pii-model-dir") {
        Some(directory) => {
            let runtime = PathBuf::from(required(options, "ort-dylib")?);
            let pii = PiiEngineConfig::new(PathBuf::from(directory), runtime);
            Ok(polaris_worker::S1Worker::with_pii_engine(
                decide, pii, idle, shutdown,
            )?)
        }
        None => Ok(polaris_worker::S1Worker::with_idle_policy(
            decide, idle, shutdown,
        )?),
    }
}

fn idle_policy(
    options: &std::collections::HashMap<String, String>,
) -> Result<polaris_worker::S1IdlePolicy, Box<dyn std::error::Error + Send + Sync>> {
    match options.get("idle-ttl-ms") {
        Some(value) => {
            let milliseconds = value
                .parse::<u64>()
                .map_err(|_| "--idle-ttl-ms requires an unsigned integer")?;
            Ok(polaris_worker::S1IdlePolicy::new(
                std::time::Duration::from_millis(milliseconds),
            )?)
        }
        None => Ok(polaris_worker::S1IdlePolicy::default()),
    }
}

fn required<'a>(
    options: &'a std::collections::HashMap<String, String>,
    key: &str,
) -> Result<&'a str, Box<dyn std::error::Error + Send + Sync>> {
    options
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required --{key}").into())
}

/// Bound how long this process waits for Tokio tasks during shutdown.
///
/// Tokio cannot preempt blocking tasks that have already started. Tasks still running after this
/// timeout may continue in the background until process exit or until they finish.
fn shutdown_runtime(runtime: tokio::runtime::Runtime, grace: std::time::Duration) {
    runtime.shutdown_timeout(grace);
}

#[cfg(test)]
#[path = "../../../tests/apps/polaris-worker/src/main.rs"]
mod tests;
