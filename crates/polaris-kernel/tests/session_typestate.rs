//! Durable core sessions on real redb, CAS, SQLite and bus instances (synthetic workspaces).
use polaris_kernel::directives::{ParseLimits, directive::SessionDirective, parse};
use polaris_kernel::{Archived, Created, KernelCtx, KernelError, Session, SessionConfig, storage};
use polaris_sdk::contracts::Event;
use polaris_sdk::ids::{Seq, SessionId};
use proptest::prelude::*;
use std::error::Error;
use std::sync::Arc;
use storage::registry::SessionStatus;

type TestResult = Result<(), Box<dyn Error>>;
fn directive(text: &str) -> Result<SessionDirective, Box<dyn Error>> {
    parse(text, &ParseLimits::default())?
        .directives
        .into_iter()
        .next()
        .map(|item| item.value)
        .ok_or_else(|| "expected directive".into())
}
fn config(workspace: &std::path::Path) -> Result<SessionConfig, Box<dyn Error>> {
    Ok(SessionConfig {
        name: "core test session".to_owned(),
        workspace: workspace.to_owned(),
        branch: "synthetic-main".to_owned(),
        base_commit: "synthetic-base".to_owned(),
        defaults: vec![directive("$websearch(off)")?],
    })
}

#[tokio::test]
async fn created_configures_to_configured() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let id = SessionId::from_u128(1);
    let configured =
        Session::<Created>::create(id, Arc::clone(&ctx))?.configure(config(workspace.path())?)?;
    assert_eq!(
        configured.directives().get("websearch"),
        Some(&directive("$websearch(off)")?),
        "defaults applied only to this session"
    );
    let records = ctx.records(Seq::ZERO, Seq::new(u64::MAX))?;
    assert_eq!(records.len(), 1, "configuration is one durable event");
    assert!(
        matches!(
            records.first().map(|record| record.event()),
            Some(Event::SessionConfigured { .. })
        ),
        "configuration precedes start"
    );
    let running = configured.start().await?;
    let meta = ctx.session_meta(id)?.ok_or("registry projection")?;
    assert_eq!(meta.status, SessionStatus::Active);
    assert_eq!(
        Some(meta.created_at),
        records.first().map(|record| record.at()),
        "registry creation time comes from the durable configuration event"
    );
    let _archived = running.archive().await?;
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn paused_resumes_to_running() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let id = SessionId::from_u128(2);
    let running = Session::<Created>::create(id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    let paused = running.pause().await?;
    assert_eq!(
        ctx.session_meta(id)?.ok_or("paused metadata")?.status,
        SessionStatus::Paused
    );
    let resumed = paused.resume().await?;
    assert_eq!(
        ctx.session_meta(id)?.ok_or("resumed metadata")?.status,
        SessionStatus::Active
    );
    let events: Vec<_> = ctx
        .records(Seq::ZERO, Seq::new(u64::MAX))?
        .iter()
        .map(|record| *record.event())
        .collect();
    assert!(
        events.contains(&Event::SessionPaused) && events.contains(&Event::SessionResumed),
        "both transitions are durable"
    );
    let _archived = resumed.archive().await?;
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn reopen_replays_directives_in_order() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(3);
    {
        let ctx = KernelCtx::open(root.path()).await?;
        let mut running = Session::<Created>::create(id, Arc::clone(&ctx))?
            .configure(config(workspace.path())?)?
            .start()
            .await?;
        running.apply(directive("$websearch(on)")?)?;
        running.apply(directive("$budget(usd=12.50)")?)?;
        running.apply(directive("$websearch(off)")?)?;
        let archived = running.archive().await?;
        drop(archived);
        ctx.shutdown().await?;
    }
    let ctx = KernelCtx::open(root.path()).await?;
    let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
    assert_eq!(
        paused.directives().get("websearch"),
        Some(&directive("$websearch(off)")?),
        "replay keeps last occurrence"
    );
    assert_eq!(
        paused.directives().get("budget"),
        Some(&directive("$budget(usd=12.50)")?),
        "other settings survive"
    );
    assert_eq!(
        ctx.session_meta(id)?.ok_or("reopened metadata")?.status,
        SessionStatus::Paused
    );
    let _archived = paused.resume().await?.archive().await?;
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn session_defaults_do_not_leak_to_other_sessions() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let mut a = Session::<Created>::create(SessionId::from_u128(4), Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    a.apply(directive("$websearch(on)")?)?;
    let b = Session::<Created>::create(SessionId::from_u128(5), Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    assert_eq!(
        b.directives().get("websearch"),
        Some(&directive("$websearch(off)")?),
        "fresh session uses defaults"
    );
    let _a = a.archive().await?;
    let _b = b.archive().await?;
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn one_live_owner_per_session() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(6);
    let ctx = KernelCtx::open(root.path()).await?;
    let running = Session::<Created>::create(id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    assert!(
        matches!(
            Session::<Created>::create(id, Arc::clone(&ctx)),
            Err(KernelError::SessionAlreadyOpen(_))
        ),
        "no duplicate mutable owner"
    );
    assert!(
        matches!(
            Session::<Archived>::reopen(id, Arc::clone(&ctx)).await,
            Err(KernelError::SessionAlreadyOpen(_))
        ),
        "reopen cannot race a live owner"
    );
    let archived = running.archive().await?;
    let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
    drop(archived);
    drop(paused);
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn running_session_is_recovered_as_paused() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(7);
    let ctx = KernelCtx::open(root.path()).await?;
    let running = Session::<Created>::create(id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    drop(running);
    let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
    assert_eq!(
        ctx.session_meta(id)?.ok_or("recovery projection")?.status,
        SessionStatus::Paused
    );
    assert_eq!(
        ctx.records(Seq::ZERO, Seq::new(u64::MAX))?
            .last()
            .map(|record| *record.event()),
        Some(Event::SessionPaused),
        "recovery pause itself is durable"
    );
    drop(paused);
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn configured_but_never_started_is_not_resumed() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(8);
    let ctx = KernelCtx::open(root.path()).await?;
    let configured =
        Session::<Created>::create(id, Arc::clone(&ctx))?.configure(config(workspace.path())?)?;
    drop(configured);
    assert!(
        matches!(
            Session::<Archived>::reopen(id, Arc::clone(&ctx)).await,
            Err(KernelError::NotStarted(_))
        ),
        "incomplete startup is explicit"
    );
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn invalid_configuration_commits_nothing() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let mut bad = config(workspace.path())?;
    bad.name = "  ".to_owned();
    assert!(
        Session::<Created>::create(SessionId::from_u128(9), Arc::clone(&ctx))?
            .configure(bad)
            .is_err(),
        "blank name rejected"
    );
    let mut missing = config(workspace.path())?;
    missing.workspace = workspace.path().join("missing");
    assert!(
        Session::<Created>::create(SessionId::from_u128(10), Arc::clone(&ctx))?
            .configure(missing)
            .is_err(),
        "missing workspace rejected"
    );
    assert!(
        ctx.records(Seq::ZERO, Seq::new(u64::MAX))?.is_empty(),
        "invalid config never reaches log"
    );
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn shutdown_closes_session_mutations() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let mut running = Session::<Created>::create(SessionId::from_u128(11), Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    ctx.shutdown().await?;
    assert!(
        matches!(
            running.apply(directive("$websearch(on)")?),
            Err(KernelError::Closed)
        ),
        "no mutation after root shutdown"
    );
    Ok(())
}

#[tokio::test]
async fn unresolved_orphans_block_startup() -> TestResult {
    let root = tempfile::tempdir()?;
    std::fs::create_dir_all(root.path().join("run"))?;
    std::fs::write(
        root.path().join("run/reaper.journal"),
        "polaris-reaper-journal v1\nR\t1\tprocess\tpid\t4242\n",
    )?;
    assert!(
        matches!(
            KernelCtx::open(root.path()).await,
            Err(KernelError::RecoveryRequired { .. })
        ),
        "unsafe orphan cannot be silently ignored"
    );
    Ok(())
}

#[tokio::test]
async fn live_publication_is_already_in_the_durable_log() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let mut running = Session::<Created>::create(SessionId::from_u128(12), Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    let mut events = ctx.subscribe();
    let seq = running.apply(directive("$websearch(on)")?)?;
    let record = tokio::time::timeout(std::time::Duration::from_secs(2), events.next()).await??;
    assert_eq!(
        record.seq(),
        seq,
        "subscriber sees assigned durable sequence"
    );
    assert_eq!(
        ctx.records(seq, seq)?,
        [record],
        "broadcast record already exists on disk"
    );
    let _archived = running.archive().await?;
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn registry_can_be_rebuilt_after_its_file_is_lost() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(13);
    {
        let ctx = KernelCtx::open(root.path()).await?;
        let mut running = Session::<Created>::create(id, Arc::clone(&ctx))?
            .configure(config(workspace.path())?)?
            .start()
            .await?;
        running.apply(directive("$websearch(on)")?)?;
        let archived = running.archive().await?;
        drop(archived);
        ctx.shutdown().await?;
    }
    // Only the database belonging to this generated TempDir fixture is removed.
    let registry = root.path().join("registry.db");
    assert!(
        registry.starts_with(root.path()),
        "fixture path stays inside its owned root"
    );
    std::fs::remove_file(registry)?;
    let ctx = KernelCtx::open(root.path()).await?;
    assert!(
        ctx.session_meta(id)?.is_none(),
        "new catalogue starts empty"
    );
    let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
    assert_eq!(
        paused.directives().get("websearch"),
        Some(&directive("$websearch(on)")?),
        "log and CAS restore overrides"
    );
    assert_eq!(
        ctx.session_meta(id)?.ok_or("rebuilt metadata")?.name,
        "core test session",
        "catalogue rebuilt from immutable configuration"
    );
    drop(paused);
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn corrupt_configuration_cas_fails_recovery() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(14);
    let ctx = KernelCtx::open(root.path()).await?;
    let archived = Session::<Created>::create(id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?
        .archive()
        .await?;
    drop(archived);
    let records = ctx.records(Seq::ZERO, Seq::new(u64::MAX))?;
    let configuration = records
        .iter()
        .find_map(|record| match *record.event() {
            Event::SessionConfigured { configuration } => Some(configuration),
            _ => None,
        })
        .ok_or("configuration event")?;
    let cas = storage::cas::Cas::open(storage::cas::CasConfig::new(root.path().join("cas")))?;
    std::fs::write(
        cas.blob_path(&configuration),
        b"deliberately corrupt synthetic CAS fixture",
    )?;
    let result = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await;
    assert!(
        matches!(result, Err(KernelError::Cas(_))),
        "corrupt CAS is rejected, not replaced with defaults"
    );
    assert_eq!(
        ctx.records(Seq::ZERO, Seq::new(u64::MAX))?.len(),
        records.len(),
        "failed replay publishes no recovery event"
    );
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn invalid_lifecycle_order_is_not_silently_replayed() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(15);
    {
        let ctx = KernelCtx::open(root.path()).await?;
        let archived = Session::<Created>::create(id, Arc::clone(&ctx))?
            .configure(config(workspace.path())?)?
            .start()
            .await?
            .archive()
            .await?;
        drop(archived);
        ctx.shutdown().await?;
    }
    {
        let log = storage::eventlog::EventLog::open(storage::eventlog::EventLogConfig::new(
            root.path().join("events.redb"),
        ))?;
        let _bad = log.append(&polaris_sdk::contracts::EventRecord::new(
            Seq::ZERO,
            polaris_sdk::contracts::Timestamp::UNIX_EPOCH,
            id,
            Event::SessionResumed,
        ))?;
    }
    let ctx = KernelCtx::open(root.path()).await?;
    assert!(
        matches!(
            Session::<Archived>::reopen(id, Arc::clone(&ctx)).await,
            Err(KernelError::Integrity(_))
        ),
        "resume after archive without reopen/pause is invalid"
    );
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn startup_failure_releases_previously_opened_databases() -> TestResult {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("registry.db"), b"not a SQLite database")?;
    assert!(
        KernelCtx::open(root.path()).await.is_err(),
        "registry startup must fail"
    );
    let bad_registry = root.path().join("registry.db");
    assert!(
        bad_registry.starts_with(root.path()),
        "only generated fixture database is removed"
    );
    std::fs::remove_file(bad_registry)?;
    let ctx = KernelCtx::open(root.path()).await?;
    assert!(
        ctx.records(Seq::ZERO, Seq::new(u64::MAX))?.is_empty(),
        "event log lock was released during rollback"
    );
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn committed_projection_failure_is_explicit_and_recoverable() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(16);
    let ctx = KernelCtx::open(root.path()).await?;
    let mut running = Session::<Created>::create(id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    let connection = storage::registry::open_connection(
        &ctx.root().join("registry.db"),
        &storage::registry::RegistryConfig::default(),
    )?;
    connection.execute_batch("CREATE TRIGGER refuse_test_update BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT, 'synthetic projection failure'); END;")?;
    let result = running.apply(directive("$websearch(on)")?);
    assert!(
        matches!(result, Err(KernelError::ProjectionBehind { .. })),
        "commit is distinguished from projection failure"
    );
    assert_eq!(
        running.directives().get("websearch"),
        Some(&directive("$websearch(on)")?),
        "memory follows successful durable append"
    );
    connection.execute_batch("DROP TRIGGER refuse_test_update;")?;
    drop(connection);
    drop(running);
    let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
    assert_eq!(
        paused.directives().get("websearch"),
        Some(&directive("$websearch(on)")?),
        "committed override survives recovery"
    );
    drop(paused);
    ctx.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn replay_crosses_page_boundary_and_filters_other_sessions() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let ctx = KernelCtx::open(root.path()).await?;
    let a_id = SessionId::from_u128(17);
    let b_id = SessionId::from_u128(18);
    let mut a = Session::<Created>::create(a_id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    let mut b = Session::<Created>::create(b_id, Arc::clone(&ctx))?
        .configure(config(workspace.path())?)?
        .start()
        .await?;
    for step in 0u16..130 {
        let (a_text, b_text) = if step & 1 == 0 {
            ("$websearch(on)", "$websearch(off)")
        } else {
            ("$websearch(off)", "$websearch(on)")
        };
        a.apply(directive(a_text)?)?;
        b.apply(directive(b_text)?)?;
    }
    let a_archive = a.archive().await?;
    let b_archive = b.archive().await?;
    assert!(
        a_archive.last_seq().get() > 256,
        "test must exercise more than one replay page"
    );
    let a = Session::<Archived>::reopen(a_id, Arc::clone(&ctx)).await?;
    let b = Session::<Archived>::reopen(b_id, Arc::clone(&ctx)).await?;
    assert_eq!(
        a.directives().get("websearch"),
        Some(&directive("$websearch(off)")?),
        "A retains its own last directive"
    );
    assert_eq!(
        b.directives().get("websearch"),
        Some(&directive("$websearch(on)")?),
        "B retains its own last directive"
    );
    drop(a);
    drop(b);
    drop(a_archive);
    drop(b_archive);
    ctx.shutdown().await?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn prop_replayed_directive_fold_matches_last_override(
        overrides in proptest::collection::vec(any::<bool>(), 0..24),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let result: Result<_, Box<dyn Error>> = runtime.block_on(async {
            let root = tempfile::tempdir()?; let workspace = tempfile::tempdir()?;
            let ctx = KernelCtx::open(root.path()).await?; let id = SessionId::from_u128(19);
            let mut running = Session::<Created>::create(id, Arc::clone(&ctx))?.configure(config(workspace.path())?)?.start().await?;
            for enabled in &overrides {
                running.apply(directive(if *enabled { "$websearch(on)" } else { "$websearch(off)" })?)?;
            }
            let archived = running.archive().await?; drop(archived);
            let paused = Session::<Archived>::reopen(id, Arc::clone(&ctx)).await?;
            let actual = paused.directives().get("websearch").cloned();
            let last = match overrides.last() { Some(enabled) => *enabled, None => false };
            let expected = directive(if last { "$websearch(on)" } else { "$websearch(off)" })?;
            drop(paused); ctx.shutdown().await?;
            Ok((actual, expected))
        });
        let (actual, expected) = result.map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(actual, Some(expected));
    }
}
