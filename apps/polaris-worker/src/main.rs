use std::path::PathBuf;

use polaris_proto::worker_auth::{WorkerAuthenticator, WorkerCredential};
use polaris_system_one_engine::{EngineConfig, TextNormalization};
use secrecy::SecretString;
use tokio::sync::watch;
use zeroize::Zeroizing;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = parse_options(&arguments)?;
    let mode = required(&options, "mode")?;
    let credential = read_worker_credential()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_stdin_shutdown_monitor(shutdown_tx.clone())?;
    let signal_tx = shutdown_tx.clone();
    let signal_task = tokio::spawn(async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to register or receive Ctrl-C: {error}");
        }
        let _ = signal_tx.send(true);
    });
    #[cfg(windows)]
    {
        let pipe = required(&options, "pipe")?.to_owned();
        match mode {
            "s1" => {
                let service = polaris_worker::S1Worker::new(engine_config(&options)?, shutdown_tx)?;
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
                let service = polaris_worker::S1Worker::new(engine_config(&options)?, shutdown_tx)?;
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
    signal_task.abort();
    let _ = signal_task.await;
    Ok(())
}

fn read_worker_credential() -> Result<WorkerCredential, Box<dyn std::error::Error + Send + Sync>> {
    use std::io::Read;

    let mut bytes = Zeroizing::new(vec![0_u8; 64]);
    std::io::stdin().read_exact(&mut bytes)?;
    let mut secret = Zeroizing::new(String::from_utf8(std::mem::take(&mut *bytes))?);
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
    if arguments.len() > 14 || arguments.len() % 2 != 0 {
        return Err("worker accepts at most seven option/value pairs".into());
    }
    for pair in arguments.chunks_exact(2) {
        let key = pair.first().ok_or("missing argument")?;
        if !key.starts_with("--") {
            return Err(format!("unexpected argument: {key}").into());
        }
        let value = pair
            .get(1)
            .ok_or_else(|| format!("missing value for {key}"))?;
        if value.is_empty() || value.len() > 4096 {
            return Err(format!("invalid value length for {key}").into());
        }
        let key = key.strip_prefix("--").ok_or("invalid option")?;
        if !matches!(
            key,
            "mode" | "pipe" | "socket" | "cas-root" | "model-dir" | "ort-dylib" | "normalization"
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

fn required<'a>(
    options: &'a std::collections::HashMap<String, String>,
    key: &str,
) -> Result<&'a str, Box<dyn std::error::Error + Send + Sync>> {
    options
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required --{key}").into())
}
