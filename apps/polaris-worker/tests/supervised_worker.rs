//! Actual supervisor, process ownership, credential bootstrap, and worker IPC composition.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use polaris_kernel::bus::{Bus, BusConfig};
use polaris_kernel::storage::eventlog::{EventLog, EventLogConfig};
use polaris_kernel::supervisor::{
    ChildName, HealthPolicy, RestartPolicy, Supervisor, SupervisorConfig, TaskSpec, WorkerSpec,
};
use polaris_kernel::worker_lifecycle::{ReadyS1Worker, S1WorkerLifecycle};
use polaris_platform::process::ProcessPlatform;
use polaris_sdk::ids::{Cid, SessionId};
use polaris_sdk::registry::{DecisionEngine, PlatformError, PlatformOps, SpawnSpec};
use polaris_sdk::system_one::{Answer, DecisionInput, Label, Question, QuestionId, QuestionKind};
use polaris_system_one_client::ClientConfig;
use tokio_util::sync::CancellationToken;

type TestError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), TestError>;

struct Fixture {
    supervisor: Supervisor<Arc<EventLog>>,
    lifecycle: Arc<S1WorkerLifecycle<ProcessPlatform>>,
    platform: Arc<ProcessPlatform>,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn start(model: PathBuf, runtime: PathBuf) -> Result<Self, TestError> {
        Self::start_with_policy(model, runtime, RestartPolicy::Temporary).await
    }

    async fn start_with_policy(
        model: PathBuf,
        runtime: PathBuf,
        policy: RestartPolicy,
    ) -> Result<Self, TestError> {
        let root = tempfile::tempdir()?;
        let database = root.path().join("events.redb");
        let store = Arc::new(
            tokio::task::spawn_blocking(move || EventLog::open(EventLogConfig::new(database)))
                .await??,
        );
        let (bus, _commands) = Bus::new(BusConfig::default(), store)?;
        let supervisor = Supervisor::new(
            Arc::new(bus),
            SessionId::from_u128(41),
            CancellationToken::new(),
            SupervisorConfig::default(),
        );
        let platform = Arc::new(ProcessPlatform::new());
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let pipe = format!(
            r"\\.\pipe\polaris-supervised-{}-{nonce}",
            std::process::id()
        );
        let lifecycle = Arc::new(S1WorkerLifecycle::new(
            Arc::clone(&platform),
            PathBuf::from(&pipe),
            ClientConfig::new(Duration::from_secs(30), 262_144)?,
            Duration::from_secs(5),
        )?);
        let mut task = TaskSpec::new(ChildName::new("supervised-s1", Cid::from_bytes([41; 32])));
        task.policy = policy;
        let mut environment = vec![("GLINER2_DEVICE".into(), "cpu".into())];
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            environment.push(("SystemRoot".into(), system_root));
        }
        let process = SpawnSpec {
            program: PathBuf::from(env!("CARGO_BIN_EXE_polaris-worker")),
            args: vec![
                "--mode".into(),
                "s1".into(),
                "--pipe".into(),
                pipe.into(),
                "--model-dir".into(),
                model.into_os_string(),
                "--ort-dylib".into(),
                runtime.into_os_string(),
                "--normalization".into(),
                "gliner2-python-2.1".into(),
            ],
            cwd: Some(root.path().to_owned()),
            env: environment,
        };
        supervisor.spawn_worker_with_lifecycle(
            Arc::clone(&platform),
            WorkerSpec {
                task,
                process,
                poll_interval: Duration::from_millis(10),
                stop_grace: Duration::from_secs(5),
            },
            Arc::clone(&lifecycle),
            HealthPolicy {
                interval: Duration::from_millis(50),
                timeout: Duration::from_secs(2),
            },
            Duration::from_secs(15),
        )?;
        Ok(Self {
            supervisor,
            lifecycle,
            platform,
            _root: root,
        })
    }

    async fn ready(&self) -> Result<ReadyS1Worker, TestError> {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(ready) = self.lifecycle.connection()? {
                    return Ok::<_, TestError>(ready);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?
    }

    async fn stop(&self, ready: ReadyS1Worker) -> TestResult {
        let tree = ready.tree();
        drop(ready);
        let _ = self.supervisor.shutdown(Duration::from_secs(8)).await;
        assert!(
            self.lifecycle.connection()?.is_none(),
            "stopped generation must not remain published"
        );
        assert!(
            matches!(
                self.platform.try_wait_tree(&tree).await,
                Err(PlatformError::NotFound(_))
            ),
            "joined shutdown must retire the exact native process record"
        );
        Ok(())
    }
}

#[tokio::test]
async fn supervised_worker_bootstraps_authenticated_health_and_retires() -> TestResult {
    let unavailable = tempfile::tempdir()?;
    let fixture = Fixture::start(
        unavailable.path().join("unavailable-model"),
        unavailable.path().join("unavailable-runtime.dll"),
    )
    .await?;
    let ready = fixture.ready().await?;
    assert!(
        ready.client().is_ready().await?,
        "readiness reports service admission without claiming a model was loaded"
    );
    fixture.stop(ready).await
}

/// Actual owned-process fault recovery; no model load or inference quality is implied.
#[tokio::test]
async fn supervised_worker_replaces_killed_generation_without_orphan() -> TestResult {
    let unavailable = tempfile::tempdir()?;
    let fixture = Fixture::start_with_policy(
        unavailable.path().join("unavailable-model"),
        unavailable.path().join("unavailable-runtime.dll"),
        RestartPolicy::Permanent,
    )
    .await?;
    let mut ready = fixture.ready().await?;
    let operation = async {
        for _ in 0..2 {
            let retired = ready.tree();
            fixture.platform.kill_tree(&retired)?;
            let replacement = tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    if let Some(candidate) = fixture.lifecycle.connection()?
                        && candidate.tree() != retired
                        && candidate.client().is_ready().await?
                    {
                        return Ok::<_, TestError>(candidate);
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await??;
            assert_ne!(
                replacement.tree(),
                retired,
                "a fresh process generation is required"
            );
            assert!(
                matches!(
                    fixture.platform.try_wait_tree(&retired).await,
                    Err(PlatformError::NotFound(_))
                ),
                "the exact killed process record must be retired"
            );
            ready = replacement;
        }
        Ok::<(), TestError>(())
    }
    .await;
    let stopped = fixture.stop(ready).await;
    operation?;
    stopped
}

#[tokio::test]
#[ignore = "requires explicit installed GLiNER/ORT artifact paths"]
async fn supervised_worker_returns_real_decision() -> TestResult {
    let model = PathBuf::from(std::env::var("POLARIS_S1_MODEL_DIR")?);
    let runtime = PathBuf::from(std::env::var("POLARIS_ORT_DYLIB")?);
    let fixture = Fixture::start(model, runtime).await?;
    let ready = fixture.ready().await?;
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
    let decision = ready.client().decide(&[question], &input).await?;
    let answer = decision.answers.first().ok_or("no model answer")?;
    let Answer::Choice { selected, probs } = &answer.answer else {
        return Err("wrong model answer kind".into());
    };
    assert_eq!(probs.len(), 2);
    assert!(
        probs
            .iter()
            .all(|score| (0.0..=1.0).contains(&score.probability.get()))
    );
    assert!(probs.iter().any(|score| score.label == *selected));
    fixture.stop(ready).await
}
