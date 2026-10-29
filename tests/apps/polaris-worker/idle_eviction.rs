//! Installed-artifact regression for real native session idle retirement.

use std::path::PathBuf;
use std::time::Duration;

use polaris_proto::pb;
use polaris_proto::pb::worker_service_server::WorkerService;
use polaris_sdk::system_one::{Label, Question, QuestionId, QuestionKind};
use polaris_system_one_engine::{EngineConfig, TextNormalization};
use polaris_worker::S1Worker;
use tonic::Request;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn configured_idle_policy_rejects_invalid_bounds() -> TestResult {
    use polaris_worker::S1IdlePolicy;

    for invalid in [
        Duration::ZERO,
        Duration::from_nanos(999_999),
        Duration::from_secs(86_401),
        Duration::MAX,
    ] {
        assert!(S1IdlePolicy::new(invalid).is_err(), "invalid idle interval");
    }
    assert_eq!(S1IdlePolicy::default().idle_ttl(), Duration::from_secs(45));
    assert_eq!(
        S1IdlePolicy::new(Duration::from_millis(1))?.idle_ttl(),
        Duration::from_millis(1)
    );
    assert!(S1IdlePolicy::new(Duration::from_secs(86_400)).is_ok());
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit installed model/runtime paths and 46 seconds of real idle time"]
async fn default_worker_drops_real_model_after_idle_ttl() -> TestResult {
    let config = EngineConfig::new(
        PathBuf::from(std::env::var("POLARIS_S1_MODEL_DIR")?),
        PathBuf::from(std::env::var("POLARIS_ORT_DYLIB")?),
        TextNormalization::Gliner2Python21,
    );
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = S1Worker::new(config, shutdown)?;
    let question = Question {
        id: QuestionId::new("triage_kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?, Label::new("feature")?],
        },
    };
    let response = worker
        .decide(Request::new(pb::DecideRequest {
            items: vec![pb::DecisionRequest {
                questions: vec![pb::Question::from(&question)],
                input: Some(pb::DecisionInput {
                    text: "Fix the parser when an empty file is provided".to_owned(),
                    context: None,
                }),
            }],
        }))
        .await?;
    assert_eq!(response.into_inner().decisions.len(), 1);
    tokio::time::sleep(Duration::from_secs(46)).await;
    let remaining = worker
        .evict_models(Request::new(pb::EvictModelsRequest::default()))
        .await?
        .into_inner()
        .evicted;
    worker.drain_local(Duration::from_secs(5)).await?;
    assert_eq!(
        remaining, 0,
        "the idle owner must have dropped the model before explicit eviction"
    );
    Ok(())
}
