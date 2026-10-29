// Unit tests of crates/polaris-ui/crates/session/src/offline.rs (`tests`); compiled into that module via #[path].
use super::{OfflineSession, PendingRecord, RecordTextKind};
use crate::SessionHostError;
use polaris_sdk::{contracts::Event, ids::SessionId, registry::EventSource};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn cancelled_payload_poll_retains_record_and_joins_one_cas_job()
-> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    session.apply_directive("$websearch(off)").await?;
    let record = loop {
        let Some(record) = session.events.next_event().await else {
            return Err("event stream ended before directive".into());
        };
        if matches!(record.event(), Event::DirectiveApplied { .. }) {
            break record;
        }
    };
    let cid = match record.event() {
        Event::DirectiveApplied { directive } => *directive,
        _ => return Err("expected directive event".into()),
    };
    let cas = Arc::clone(&session.cas);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    session.pending_record = Some(PendingRecord {
        record,
        kind: RecordTextKind::Directive,
        payload: tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            let _ = release_rx.blocking_recv();
            cas.get_bounded(&cid, 65_536).map(|bytes| bytes.to_vec())
        }),
    });
    started_rx.await?;

    let cancelled = tokio::time::timeout(Duration::from_millis(5), session.next_record()).await;
    if cancelled.is_ok() {
        return Err("CAS read unexpectedly completed before cancellation".into());
    }
    release_tx
        .send(())
        .map_err(|_| "CAS read receiver disappeared")?;

    let resolved = session
        .next_record()
        .await?
        .ok_or("retained directive record was lost")?;
    if resolved.text.as_deref() != Some("$websearch(off)") {
        return Err("retained directive payload changed".into());
    }
    assert_eq!(resolved.text_kind, Some(RecordTextKind::Directive));
    session.close().await?;
    Ok(())
}
#[tokio::test]
async fn failed_payload_owner_is_consumed_once_before_close()
-> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    session.apply_directive("$websearch(off)").await?;
    let record = loop {
        let record = session.events.next_event().await.ok_or("stream ended")?;
        if matches!(record.event(), Event::DirectiveApplied { .. }) {
            break record;
        }
    };
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let worker = tokio::spawn(async move {
        let _released = release_rx.await;
        Ok(Vec::<u8>::new())
    });
    worker.abort();
    session.pending_record = Some(PendingRecord {
        record,
        kind: RecordTextKind::Directive,
        payload: worker,
    });
    let failed = session.next_record().await.is_err();
    let cleared = session.pending_record.is_none();
    let cleanup = session.close().await;
    assert!(failed, "failed worker must be reported");
    assert!(
        cleared,
        "completed JoinHandle cannot be awaited twice during close"
    );
    cleanup?;
    Ok(())
}
#[tokio::test]
async fn unrelated_session_payload_is_not_materialized() -> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let other_id = SessionId::new_v7();
    let mut other = OfflineSession::open(state.path(), workspace.path(), other_id).await?;
    other.apply_directive("$websearch(off)").await?;
    let cid = loop {
        let record = other.events.next_event().await.ok_or("stream ended")?;
        if let Event::DirectiveApplied { directive } = record.event() {
            break *directive;
        }
    };
    let blob_path = other.cas.blob_path(&cid);
    other.close().await?;
    drop(other);
    std::fs::remove_file(blob_path)?;
    let mut active =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let observed = loop {
        match active.next_record().await {
            Ok(Some(record))
                if record.record.session() == other_id
                    && matches!(record.record.event(), Event::DirectiveApplied { .. }) =>
            {
                break Ok(record);
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                break Err(SessionHostError::Unavailable(
                    "stream ended before other session envelope",
                ));
            }
            Err(error) => break Err(error),
        }
    };
    let cleanup = active.close().await;
    let observed = observed?;
    assert!(
        observed.text.is_none(),
        "other-session payload must stay unresolved"
    );
    assert!(observed.text_kind.is_none());
    cleanup?;
    Ok(())
}
