// Unit tests of crates/polaris-ui/crates/web/src/backend.rs (`tests`); compiled into that module via #[path].
use super::{Publisher, Replay, WebEvent};

fn provider() -> WebEvent {
    WebEvent::Provider {
        provider: "local".to_owned(),
        model: "a".to_owned(),
        effort: "medium".to_owned(),
        models: vec![("medium".to_owned(), "a".to_owned())],
    }
}

#[test]
fn replay_keeps_provider_and_only_the_open_session() {
    let mut replay = Replay::default();
    replay.record(&provider());
    replay.record(&WebEvent::Message {
        kind: "user".to_owned(),
        text: "old session".to_owned(),
    });
    replay.record(&WebEvent::Opening);
    replay.record(&WebEvent::Opened {
        id: "s2".to_owned(),
        title: String::new(),
        workspace: "w".to_owned(),
    });
    replay.record(&WebEvent::Notice {
        text: "transient".to_owned(),
    });
    replay.record(&WebEvent::ModelSwitched {
        model: "b".to_owned(),
        effort: "high".to_owned(),
    });
    let snapshot = replay.snapshot();
    assert_eq!(snapshot.len(), 2, "{snapshot:?}");
    assert!(matches!(
        &snapshot[0],
        WebEvent::Provider { model, effort, .. } if model == "b" && effort == "high"
    ));
    assert!(matches!(&snapshot[1], WebEvent::Opened { id, .. } if id == "s2"));
}

#[tokio::test]
async fn subscribers_get_the_snapshot_then_live_events() {
    let publisher = Publisher::new(8);
    publisher.publish(provider());
    let (snapshot, mut live) = publisher.subscribe();
    assert_eq!(snapshot, vec![provider()]);
    publisher.publish(WebEvent::ReplyStarted);
    assert_eq!(live.recv().await.ok(), Some(WebEvent::ReplyStarted));
}
