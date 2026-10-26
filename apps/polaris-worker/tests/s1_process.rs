#![cfg(windows)]

use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyper_util::rt::TokioIo;
use polaris_proto::worker_auth::WorkerCredential;
use polaris_sdk::registry::DecisionEngine;
use polaris_sdk::system_one::{Answer, DecisionInput, Label, Question, QuestionId, QuestionKind};
use polaris_system_one_client::{ClientConfig, S1Client};
use tokio::io::AsyncWriteExt;
use tonic::transport::Endpoint;
use zeroize::Zeroizing;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn unique_pipe() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(format!(
        r"\\.\pipe\polaris-s1-{}-{elapsed}",
        std::process::id()
    ))
}

async fn connect_pipe(
    path: String,
) -> Result<tonic::transport::Channel, Box<dyn std::error::Error + Send + Sync>> {
    let connector = tower::service_fn(move |_uri| {
        let name = path.clone();
        async move {
            for _attempt in 0..100 {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&name) {
                    Ok(client) => return Ok(TokioIo::new(client)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "worker pipe did not become available",
            ))
        }
    });
    let channel = Endpoint::from_static("http://[::]:50051")
        .connect_timeout(Duration::from_secs(3))
        .connect_with_connector(connector)
        .await?;
    Ok(channel)
}

/// Set POLARIS_S1_MODEL_DIR and POLARIS_ORT_DYLIB to the installed local fp32 artifacts.
#[tokio::test]
#[ignore = "requires explicit local GLiNER model and ORT DLL paths"]
async fn configured_windows_worker_process_returns_real_decision() -> TestResult {
    let model = std::env::var("POLARIS_S1_MODEL_DIR")?;
    let ort = std::env::var("POLARIS_ORT_DYLIB")?;
    let pipe = unique_pipe()?;
    let credential = WorkerCredential::generate()?;
    let executable = env!("CARGO_BIN_EXE_polaris-worker");
    let mut child = tokio::process::Command::new(executable)
        .args([
            "--mode",
            "s1",
            "--pipe",
            &pipe,
            "--model-dir",
            &model,
            "--ort-dylib",
            &ort,
            "--normalization",
            "gliner2-python-2.1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut child_stdin = child.stdin.take().ok_or("worker stdin was not piped")?;
    let mut token_bytes = Zeroizing::new(Vec::new());
    credential.write_to(&mut *token_bytes)?;
    child_stdin.write_all(&token_bytes).await?;
    drop(token_bytes);
    let channel = connect_pipe(pipe).await?;
    let adapter = S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(30), 262_144)?,
        credential,
    )?;
    let question = Question {
        id: QuestionId::new("triage_kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?, Label::new("feature")?],
        },
    };
    let input = DecisionInput {
        text: "Fix the parser when an empty file is provided".to_owned(),
        context: None,
    };
    let decision = adapter.decide(&[question], &input).await?;
    let answer = decision
        .answers
        .first()
        .ok_or("worker returned no answer")?;
    let Answer::Choice { selected, probs } = &answer.answer else {
        return Err("worker returned the wrong answer kind".into());
    };
    assert_eq!(probs.len(), 2);
    assert!(
        probs
            .iter()
            .all(|score| (0.0..=1.0).contains(&score.probability.get()))
    );
    assert!(probs.iter().any(|score| score.label == *selected));
    // EOF on the inherited bootstrap pipe is the existing supervisor's graceful-stop signal.
    drop(child_stdin);
    let status = tokio::time::timeout(Duration::from_secs(40), child.wait()).await??;
    assert!(
        status.success(),
        "worker process exited successfully after graceful shutdown"
    );
    Ok(())
}
