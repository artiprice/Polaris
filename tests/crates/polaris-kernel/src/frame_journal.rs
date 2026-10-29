// Unit tests of crates/polaris-kernel/src/frame_journal.rs (`tests`); compiled into that module via #[path].
use super::KernelFrameJournal;
use crate::{Kernel, LocalKernelConfig, storage::cas::content_id};
use polaris_runtime::frames::{
    AgentContextFrame, AgentRole, FrameJournal, FrameJournalLimits, FrameScope, FrameStepRecord,
    ModelSelection, ResourceQuota, ScratchpadRef,
};
use polaris_sdk::{
    contracts::{Event, FrameStepOutcome},
    effort::Effort,
    ids::{Cid, FrameId, ModelId, Seq, SessionId},
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
};
use proptest::prelude::*;
use std::{
    future::{Future, poll_fn},
    io,
    sync::{Arc, mpsc},
    task::Poll,
    time::Duration,
};

proptest! {
    #[test]
    fn journal_preparation_obeys_exact_escaped_json_bound(goal in ".{0,1024}", cap in 1_usize..32768) {
        let record = FrameStepRecord { schema: 1, frame: FrameId::from_u128(7), step: 1, goal, request: None, assistant_response: None, result: None, executed_tool_results: Vec::new(), watchdog_memory: None, outcome: FrameStepOutcome::Failed };
        let encoded = serde_json::to_vec(&record);
        let prepared = polaris_runtime::frames::PreparedFrameStep::new(&record, FrameJournalLimits {max_record_bytes: cap, max_tool_result_bytes: 1, max_tool_results: 1});
        match encoded {
            Ok(bytes) if bytes.len() <= cap => prop_assert!(prepared.is_ok()),
            Ok(_) => prop_assert!(matches!(prepared, Err(polaris_runtime::frames::FrameJournalError::RecordTooLarge { .. })), "oversized transcript must fail with a size error"),
            Err(_) => prop_assert!(prepared.is_err()),
        }
    }
}

#[tokio::test]
async fn accepted_journal_finishes_after_future_drop_and_shutdown()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let frame_id = FrameId::new_v7();
    let session = SessionId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        session,
        frame_id,
    ))
    .await?;
    let authority = Authority::new_session()?;
    let scope = Scope {
        resource: ResourceScope::Workspace,
        max_churn_lines: 0,
        ttl: Duration::from_secs(300),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    let token = authority.issue_root(frame_id, &scope, &SystemClock)?;
    let expires_at_micros = SystemClock
        .now()
        .as_microsecond()
        .checked_add(300_000_000)
        .ok_or("expiry overflow")?;
    let frame = AgentContextFrame::new_root(
        frame_id,
        AgentRole::Orchestrator,
        token,
        FrameScope {
            capability: scope,
            expires_at_micros,
        },
        ResourceQuota {
            cpu_millis: 1000,
            memory_mb: 128,
            network: Default::default(),
            max_steps: 2,
            max_depth: 1,
            token_budget: 100,
        },
        ScratchpadRef::new(Cid::from_bytes([1; 32])),
    )
    .activate(
        ModelSelection {
            model: ModelId::new("test/synthetic-model")?,
            maximum_effort: Effort::Low,
        },
        Effort::Low,
    )?;
    let journal = KernelFrameJournal::new(
        kernel.clone(),
        frame.tool_binding(),
        FrameJournalLimits::default(),
    )?;
    let record = FrameStepRecord {
        schema: 1,
        frame: frame_id,
        step: 1,
        goal: "synthetic dropped journal".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: Vec::new(),
        watchdog_memory: None,
        outcome: FrameStepOutcome::Cancelled,
    };
    let expected = content_id(&serde_json::to_vec(&record)?);
    let context = Arc::clone(kernel.context());
    let blocker_context = Arc::clone(&context);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        let _writer = blocker_context
            .lock_writer()
            .map_err(|_| io::Error::other("writer unavailable"))?;
        started_tx
            .send(())
            .map_err(|_| io::Error::other("start receiver dropped"))?;
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("journal blocker timed out"))?;
        Ok::<(), io::Error>(())
    });
    tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(5))).await??;
    let mut pending = Box::pin(journal.record(&record));
    poll_fn(|cx| match pending.as_mut().poll(cx) {
        Poll::Pending => Poll::Ready(Ok::<(), io::Error>(())),
        Poll::Ready(_) => Poll::Ready(Err(io::Error::other("journal bypassed held writer"))),
    })
    .await?;
    // Actual CAS visibility proves the blocking owner progressed beyond persistence while
    // publication is fenced by the held writer; this is not inferred from a sleep alone.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let cas = context.cas.clone();
            if tokio::task::spawn_blocking(move || cas.contains(&expected)).await?? {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await??;
    drop(pending);
    let shutdown_kernel = kernel.clone();
    let mut shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
    let early = tokio::time::timeout(Duration::from_millis(25), &mut shutdown)
        .await
        .is_ok();
    release_tx.send(())?;
    blocker.await??;
    assert!(!early, "shutdown must wait for the accepted record");
    let _report = tokio::time::timeout(Duration::from_secs(5), shutdown).await???;
    let records = context.records(Seq::new(1), Seq::new(u64::MAX))?;
    assert!(records.iter().any(|entry| matches!(entry.event(), Event::FrameStepRecorded { transcript, .. } if *transcript == expected)), "dropped caller must not discard accepted audit publication");
    Ok(())
}
