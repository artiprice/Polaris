//! P9 attach client: [`AttachTarget`] parsing, and a [`RemoteSession`] against a real `polarisd`
//! over its local endpoint, authenticated by the credential the daemon issued.

#[path = "../../../../support/attach_daemon.rs"]
mod attach_daemon;

use std::time::Duration;

use attach_daemon::{AnyError, Daemon};
use polaris_sdk::ids::SessionId;
use polaris_ui_session::{AttachTarget, RecordTextKind, RemoteSession};

#[test]
fn attach_targets_take_decimal_or_uuid_sessions_and_hex_credentials() -> Result<(), AnyError> {
    let decimal = AttachTarget::new("polaris-x", "42", "0a0B\n")?;
    assert_eq!(decimal.session, SessionId::from_u128(42));
    assert_eq!(decimal.credential, [0x0a, 0x0b]);
    let uuid = AttachTarget::new("polaris-x", &SessionId::from_u128(7).to_string(), "00")?;
    assert_eq!(uuid.session, SessionId::from_u128(7));
    for bad in ["", "abc", "zz", "0 1"] {
        assert!(AttachTarget::new("polaris-x", "1", bad).is_err(), "{bad:?}");
    }
    assert!(AttachTarget::new("polaris-x", "not-a-session", "00").is_err());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_session_records_and_replays_text_through_polarisd() -> Result<(), AnyError> {
    let daemon = Daemon::start()?;
    daemon.ready().await?;
    let mut remote = RemoteSession::attach(&daemon.target()?).await?;
    assert_eq!(remote.id, daemon.session);
    assert_eq!(remote.endpoint(), daemon.endpoint);

    remote.submit("Explain the parser").await?;
    remote.submit("$websearch(off)").await?;
    assert!(
        remote.submit("   ").await.is_err(),
        "blank text is not sent"
    );
    let mut texts = Vec::new();
    while texts.len() < 2 {
        let record = tokio::time::timeout(Duration::from_secs(10), remote.next_record())
            .await
            .map_err(|_| "replay timed out")??
            .ok_or("event stream ended")?;
        if let (Some(text), Some(kind)) = (record.text, record.text_kind) {
            texts.push((kind, text));
        }
    }
    assert_eq!(
        texts,
        [
            (RecordTextKind::UserInput, "Explain the parser".to_owned()),
            (RecordTextKind::Directive, "$websearch(off)".to_owned()),
        ]
    );

    // A credential from an issuer the daemon does not trust never attaches.
    assert!(
        RemoteSession::attach(&daemon.foreign_target()?)
            .await
            .is_err()
    );
    // A frontend pinned to another session cannot write into this one.
    let elsewhere = AttachTarget::new(&daemon.endpoint, "3054", &daemon.credential)?;
    if let Ok(other) = RemoteSession::attach(&elsewhere).await {
        assert!(other.submit("hello").await.is_err());
    }

    drop(remote);
    daemon.stop().await
}

/// Frontends poll `next_record` inside `select!`, which drops it whenever another branch wins:
/// a record taken off the stream while its text is being read must not be lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_next_record_never_skips_a_message() -> Result<(), AnyError> {
    let daemon = Daemon::start()?;
    daemon.ready().await?;
    let mut remote = RemoteSession::attach(&daemon.target()?).await?;
    let sent = ["first", "second", "third"];
    for text in sent {
        remote.submit(text).await?;
    }
    let mut texts = Vec::new();
    let mut dropped = 0_u32;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut budget = 1_u64;
    while texts.len() < sent.len() && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_micros(budget), remote.next_record()).await {
            Ok(record) => {
                if let Some(text) = record?.ok_or("event stream ended")?.text {
                    texts.push(text);
                }
                budget = 1;
            }
            Err(_) => {
                dropped += 1;
                budget = budget.saturating_mul(2).min(1_000_000);
            }
        }
    }
    assert!(dropped > 0, "the test must drop at least one pending read");
    assert_eq!(texts, sent);
    drop(remote);
    daemon.stop().await
}
