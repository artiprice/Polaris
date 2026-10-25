//! Exercise real durable session adapters; no model/provider fixtures.
use polaris_cli::session::OfflineSession;
use polaris_sdk::{contracts::Event, ids::SessionId};

#[tokio::test]
async fn offline_directive_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::new_v7();
    let mut session = OfflineSession::open(state.path(), workspace.path(), id).await?;
    session.apply_directive("$websearch(off)").await?;
    session.close().await?;
    drop(session);
    let mut reopened = OfflineSession::open(state.path(), workspace.path(), id).await?;
    let mut seen = false;
    for _ in 0..16 {
        let next = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            reopened.next_record(),
        )
        .await;
        match next {
            Ok(Ok(Some(item))) => {
                if matches!(item.record.event(), Event::DirectiveApplied { .. }) {
                    assert_eq!(item.text.as_deref(), Some("$websearch(off)"));
                    seen = true;
                    break;
                }
            }
            Ok(Ok(None)) | Err(_) => break,
            Ok(Err(error)) => return Err(error.into()),
        }
    }
    reopened.close().await?;
    assert!(seen, "durable directive absent after reopening");
    Ok(())
}

#[tokio::test]
async fn rejected_directive_preserves_draft() -> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let mut draft = polaris_ui_model::composer::Composer::new();
    draft.insert("$ratelimit(rpm=not-a-number)")?;
    draft.move_left();
    let text = draft.text().to_owned();
    let cursor = draft.cursor();
    let prepared = draft.prepare_submit()?;
    assert!(
        session
            .apply_directive(&prepared.submission().text)
            .await
            .is_err(),
        "malformed directive accepted"
    );
    draft.cancel_submit(prepared);
    assert_eq!(draft.text(), text);
    assert_eq!(draft.cursor(), cursor);
    session.close().await?;
    Ok(())
}

#[tokio::test]
async fn registry_is_real_and_oversized_directives_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::new_v7();
    let mut session = OfflineSession::open(state.path(), workspace.path(), id).await?;
    assert!(
        session.session_exists(id).await?,
        "exact registry lookup missed active session"
    );
    assert!(
        !session.session_exists(SessionId::new_v7()).await?,
        "unknown session was reported as present"
    );
    let rows = session.list_sessions().await?;
    assert!(
        rows.iter().any(|row| row.id == id),
        "active session missing from actual registry"
    );
    assert!(
        session.apply_directive(&"x".repeat(65_537)).await.is_err(),
        "oversized control accepted"
    );
    session.close().await?;
    Ok(())
}
