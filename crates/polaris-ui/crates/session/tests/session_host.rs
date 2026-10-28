//! Session host (descriptor: specs/descriptors/ui_session_host.json).
//! Real redb event log, CAS, SQLite registry and signed local control; no model or provider.
use polaris_kernel::storage::registry::{Registry, SessionStatus};
use polaris_sdk::{contracts::Event, ids::SessionId};
use polaris_ui_session::{
    OfflineSession, RecordTextKind, ResolvedRecord, SessionHostError, list_sessions,
    search_history, session_exists,
};
use std::{error::Error, path::Path, time::Duration};

type TestResult = Result<(), Box<dyn Error>>;

async fn open(
    state: &Path,
    workspace: &Path,
    id: SessionId,
) -> Result<OfflineSession, Box<dyn Error>> {
    Ok(OfflineSession::open(state, workspace, id).await?)
}

/// Replays until the first text-bearing record of the active session (or the stream is idle).
async fn text_records(session: &mut OfflineSession) -> Result<Vec<ResolvedRecord>, Box<dyn Error>> {
    let mut found = Vec::new();
    for _ in 0..64 {
        match tokio::time::timeout(Duration::from_millis(200), session.next_record()).await {
            Ok(Ok(Some(item))) => {
                if item.text.is_some() {
                    found.push(item);
                }
            }
            Ok(Ok(None)) | Err(_) => break,
            Ok(Err(error)) => return Err(error.into()),
        }
    }
    Ok(found)
}

fn kinds(records: &[ResolvedRecord]) -> Vec<(Option<RecordTextKind>, Option<&str>)> {
    records
        .iter()
        .map(|item| (item.text_kind, item.text.as_deref()))
        .collect()
}

#[tokio::test]
async fn plain_prose_commits() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session = open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let receipt = session.submit("plan tomorrow").await?;
    let records = text_records(&mut session).await?;
    session.close().await?;
    let [item] = records.as_slice() else {
        return Err(format!("expected one text record, got {}", records.len()).into());
    };
    assert!(matches!(
        item.record.event(),
        Event::UserInputSubmitted { .. }
    ));
    assert_eq!(item.text.as_deref(), Some("plan tomorrow"));
    assert_eq!(item.text_kind, Some(RecordTextKind::UserInput));
    assert_eq!(item.record.seq(), receipt.accepted_at);
    Ok(())
}

#[tokio::test]
async fn directive_routes_to_apply_directive() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session = open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    session.submit("$websearch(on)").await?;
    let records = text_records(&mut session).await?;
    session.close().await?;
    let [item] = records.as_slice() else {
        return Err(format!("expected one text record, got {}", records.len()).into());
    };
    assert!(matches!(
        item.record.event(),
        Event::DirectiveApplied { .. }
    ));
    // The directive is stored in its canonical rendering, where `on` is the default form.
    assert_eq!(item.text.as_deref(), Some("$websearch"));
    assert_eq!(item.text_kind, Some(RecordTextKind::Directive));
    Ok(())
}

#[tokio::test]
async fn directive_with_prose_is_recorded_as_user_input() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session = open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let lines = [
        "$websearch(on) please look this up",
        "check @src/main.rs",
        "/help",
    ];
    for line in lines {
        session.submit(line).await?;
    }
    let records = text_records(&mut session).await?;
    session.close().await?;
    assert_eq!(
        kinds(&records),
        lines.map(|line| (Some(RecordTextKind::UserInput), Some(line)))
    );
    Ok(())
}

#[tokio::test]
async fn rejected_directive_text_is_recorded_as_user_input() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session = open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let malformed = "$ratelimit(rpm=not-a-number)";
    assert!(
        session.apply_directive(malformed).await.is_err(),
        "the direct directive path still rejects malformed grammar"
    );
    session.submit(malformed).await?;
    let records = text_records(&mut session).await?;
    session.close().await?;
    assert_eq!(
        kinds(&records),
        [(Some(RecordTextKind::UserInput), Some(malformed))]
    );
    Ok(())
}

#[tokio::test]
async fn blank_submit_is_rejected() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session = open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let error = session
        .submit("  \n ")
        .await
        .err()
        .ok_or("blank accepted")?;
    assert_eq!(error.to_string(), "command rejected: user input is empty");
    assert!(text_records(&mut session).await?.is_empty());
    session.close().await?;
    Ok(())
}

#[tokio::test]
async fn list_sessions_without_kernel_includes_new_session() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let first = SessionId::new_v7();
    let second = SessionId::new_v7();
    for id in [first, second] {
        let mut session = open(state.path(), workspace.path(), id).await?;
        session.close().await?;
    }
    // No kernel is running here: the owning sessions are closed.
    let rows = list_sessions(state.path(), 10).await?;
    let ids: Vec<SessionId> = rows.iter().map(|row| row.id).collect();
    assert!(ids.contains(&first) && ids.contains(&second), "{ids:?}");
    assert!(rows.iter().all(|row| row.status != SessionStatus::Archived));
    assert_eq!(
        list_sessions(state.path(), 1).await?.len(),
        1,
        "limit honoured"
    );
    Ok(())
}

#[tokio::test]
async fn session_exists_without_kernel() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::new_v7();
    let mut session = open(state.path(), workspace.path(), id).await?;
    session.close().await?;
    assert!(session_exists(state.path(), id).await?);
    assert!(!session_exists(state.path(), SessionId::new_v7()).await?);
    Ok(())
}

#[tokio::test]
async fn resume_replays_user_input() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::new_v7();
    let mut session = open(state.path(), workspace.path(), id).await?;
    session.submit("first note").await?;
    session.submit("$websearch(off)").await?;
    session.submit("second note\nwith two lines").await?;
    session.close().await?;
    drop(session);

    let mut reopened = open(state.path(), workspace.path(), id).await?;
    let records = text_records(&mut reopened).await?;
    reopened.close().await?;
    assert_eq!(
        kinds(&records),
        [
            (Some(RecordTextKind::UserInput), Some("first note")),
            (Some(RecordTextKind::Directive), Some("$websearch(off)")),
            (
                Some(RecordTextKind::UserInput),
                Some("second note\nwith two lines")
            ),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn search_history_reports_only_indexed_messages() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::new_v7();
    let mut session = open(state.path(), workspace.path(), id).await?;
    session.submit("quokka migration plan").await?;
    session.close().await?;
    assert!(
        search_history(state.path(), "quokka", 10).await?.is_empty(),
        "typed user input is not indexed by the kernel yet"
    );

    let root = state.path().to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), Box<dyn Error + Send + Sync>> {
        let registry = Registry::open(&root)?;
        registry.index_message(
            id,
            polaris_sdk::registry::ChatRole::User,
            "wombat habitat notes",
            "2026-10-28T10:00:00Z".parse()?,
        )?;
        Ok(())
    })
    .await?
    .map_err(|error| error.to_string())?;
    let hits = search_history(state.path(), "WOMBAT", 10).await?;
    assert_eq!(hits.len(), 1, "case-insensitive literal match");
    assert_eq!(hits.first().map(|hit| hit.session), Some(id));

    let oversize = "q".repeat(polaris_kernel::storage::registry::MAX_SEARCH_QUERY_BYTES + 1);
    assert!(search_history(state.path(), &oversize, 10).await.is_err());
    Ok(())
}

#[test]
fn host_error_messages_are_preserved() {
    assert_eq!(
        SessionHostError::Ui("CAS reader failed: x".into()).to_string(),
        "CAS reader failed: x"
    );
    assert_eq!(
        SessionHostError::Unavailable("pending record disappeared").to_string(),
        "pending record disappeared"
    );
}
