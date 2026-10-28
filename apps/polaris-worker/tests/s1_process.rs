#![cfg(windows)]

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use polaris_proto::worker_auth::WorkerCredential;
use polaris_sdk::registry::DecisionEngine;
use polaris_sdk::system_one::{Answer, DecisionInput, Label, Question, QuestionId, QuestionKind};
use polaris_system_one_client::{ClientConfig, S1Client};
use tokio::io::AsyncWriteExt;
use zeroize::Zeroizing;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type TestError = Box<dyn std::error::Error + Send + Sync>;

struct LiveWorker {
    child: tokio::process::Child,
    bootstrap: tokio::process::ChildStdin,
    adapter: Arc<S1Client>,
}

fn unique_pipe() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(format!(
        r"\\.\pipe\polaris-s1-{}-{elapsed}",
        std::process::id()
    ))
}

impl LiveWorker {
    async fn start(idle_ms: Option<u64>) -> Result<Self, TestError> {
        let model = std::env::var("POLARIS_S1_MODEL_DIR")?;
        let ort = std::env::var("POLARIS_ORT_DYLIB")?;
        let pipe = unique_pipe()?;
        let credential = WorkerCredential::generate()?;
        let executable = env!("CARGO_BIN_EXE_polaris-worker");
        let mut command = tokio::process::Command::new(executable);
        command.args([
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
        ]);
        if let Some(idle_ms) = idle_ms {
            command.args(["--idle-ttl-ms", &idle_ms.to_string()]);
        }
        if let Some(directory) = std::env::var_os("POLARIS_PII_MODEL_DIR") {
            command.arg("--pii-model-dir").arg(directory);
        }
        let mut child = command
            .env("GLINER2_DEVICE", "cpu")
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
        let adapter = S1Client::connect_named_pipe(
            PathBuf::from(&pipe),
            ClientConfig::new(Duration::from_secs(30), 262_144)?,
            credential,
        )
        .await?;
        Ok(Self {
            child,
            bootstrap: child_stdin,
            adapter: Arc::new(adapter),
        })
    }

    async fn stop(mut self) -> TestResult {
        // EOF on the inherited bootstrap pipe requests graceful worker shutdown.
        drop(self.bootstrap);
        let status = tokio::time::timeout(Duration::from_secs(40), self.child.wait()).await??;
        assert!(
            status.success(),
            "worker process exited successfully after graceful shutdown"
        );
        Ok(())
    }
}

async fn check_real_decision(adapter: &S1Client) -> TestResult {
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
    Ok(())
}

/// Set POLARIS_S1_MODEL_DIR and POLARIS_ORT_DYLIB to the installed local fp32 artifacts.
#[tokio::test]
#[ignore = "requires explicit local GLiNER model and ORT DLL paths"]
async fn configured_windows_worker_process_returns_real_decision() -> TestResult {
    let worker = LiveWorker::start(None).await?;
    check_real_decision(&worker.adapter).await?;
    assert_eq!(
        worker.adapter.evict_models().await?,
        1,
        "loaded model is dropped explicitly"
    );
    check_real_decision(&worker.adapter).await?;
    assert_eq!(
        worker.adapter.evict_models().await?,
        1,
        "real inference reloads after eviction"
    );
    worker.stop().await
}

#[tokio::test]
#[ignore = "requires explicit installed GLiNER/ORT artifacts"]
async fn real_worker_reloads_after_idle_eviction() -> TestResult {
    let worker = LiveWorker::start(Some(250)).await?;
    check_real_decision(&worker.adapter).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        worker.adapter.evict_models().await?,
        0,
        "idle owner already dropped the real session"
    );
    check_real_decision(&worker.adapter).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        worker.adapter.evict_models().await?,
        0,
        "reloaded model follows the same idle policy"
    );
    worker.stop().await
}

/// Real IPC workload; each report contains only observed outcomes and timings.
#[tokio::test]
#[ignore = "requires explicit installed GLiNER/ORT artifacts"]
async fn real_worker_runs_warm_and_post_eviction_fifty_request_bursts() -> TestResult {
    use polaris_spike_s1::{BurstConfig, BurstRequest, run_burst};
    let worker = LiveWorker::start(None).await?;
    let operation = async {
        check_real_decision(&worker.adapter).await?;
        for round in ["warm", "post_eviction", "warm_distinct"] {
            if round == "post_eviction" {
                assert_eq!(worker.adapter.evict_models().await?, 1);
            }
            let question = Question {
                id: QuestionId::new("triage_kind")?,
                kind: QuestionKind::Choice {
                    options: vec![Label::new("bug")?, Label::new("feature")?],
                },
            };
            let requests = (0..50)
                .map(|index| BurstRequest {
                    questions: vec![question.clone()],
                    input: DecisionInput {
                        text: "Fix the parser when an empty file is provided".to_owned(),
                        context: if round == "warm_distinct" {
                            Some(format!("Independent workload case {index}."))
                        } else {
                            None
                        },
                    },
                })
                .collect();
            let report = run_burst(
                Arc::clone(&worker.adapter), requests,
                BurstConfig { max_owned_tasks: 8, request_timeout: Duration::from_secs(30) },
                &tokio_util::sync::CancellationToken::new(),
            ).await?;
            assert_eq!(report.requested, 50);
            assert_eq!(report.succeeded(), 50, "all actual IPC requests must receive valid decisions");
            assert!(report.peak_owned_tasks <= 8);
            println!("OBSERVED S1 burst {round}: requested={} answered={} peak_owned={} elapsed_us={} p95_ns={:?}", report.requested, report.succeeded(), report.peak_owned_tasks, report.elapsed.as_micros(), report.p95_latency.map(|value| value.as_nanos()));
        }
        Ok::<(), TestError>(())
    }.await;
    let stopped = worker.stop().await;
    operation?;
    stopped
}

#[tokio::test]
#[ignore = "requires explicit installed Decide, PII companion and ORT artifacts"]
async fn real_worker_pii_spans_then_decide_reloads_independent_model() -> TestResult {
    let _configured =
        std::env::var_os("POLARIS_PII_MODEL_DIR").ok_or("PII companion path required")?;
    let worker = LiveWorker::start(None).await?;
    let operation = async {
        let text = "Contact Alice Example at alice.example@example.test. Київ.";
        let spans = worker.adapter.detect_pii_spans(text).await?;
        assert!(
            !spans.is_empty(),
            "actual companion must detect the planted fixture"
        );
        for span in &spans {
            assert!(
                text.get(usize::try_from(span.start())?..usize::try_from(span.end())?)
                    .is_some()
            );
        }
        check_real_decision(&worker.adapter).await?;
        assert_eq!(
            worker.adapter.evict_models().await?,
            1,
            "the independently loaded Decide model must be acknowledged dropped"
        );
        let again = worker.adapter.detect_pii_spans(text).await?;
        assert_eq!(
            again, spans,
            "companion reload preserves this exact fixture's spans"
        );
        assert_eq!(
            worker.adapter.evict_models().await?,
            1,
            "the independently reloaded PII model must be acknowledged dropped"
        );
        println!(
            "OBSERVED PII IPC spans={} raw_utf8_bytes={}",
            spans.len(),
            text.len()
        );
        Ok::<(), TestError>(())
    }
    .await;
    let stopped = worker.stop().await;
    operation?;
    stopped
}
