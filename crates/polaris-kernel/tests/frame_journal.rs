//! Real kernel, temporary files, signed frame identity, CAS and event-log journal integration.
use polaris_kernel::{
    Kernel, KernelFrameJournal, KernelHandle, KernelToolRunner, LocalKernelConfig,
};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, ExecutedToolResult, FrameJournal, FrameJournalLimits,
    FrameScope, FrameStepRecord, ModelSelection, ResourceQuota, ScratchpadRef, ToolCall,
    ToolRunner,
};
use polaris_sdk::{
    contracts::{Event, FrameStepOutcome},
    effort::Effort,
    ids::{Cid, FrameId, ModelId, Seq, SessionId},
    registry::{EventSource, KernelApi},
};
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
struct Fixture {
    root: tempfile::TempDir,
    workspace: tempfile::TempDir,
    session: SessionId,
    kernel: KernelHandle,
    authority: Authority,
    token: Token,
    frame: AgentContextFrame<Active>,
}
impl Fixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(
            workspace.path().join("sample.txt"),
            "synthetic journal source\n",
        )?;
        let session = SessionId::new_v7();
        let id = FrameId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            id,
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
        let token = authority.issue_root(id, &scope, &SystemClock)?;
        let expires_at_micros = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            id,
            AgentRole::Orchestrator,
            token.clone(),
            FrameScope {
                capability: scope,
                expires_at_micros,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 8,
                max_depth: 3,
                token_budget: 1000,
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
        Ok(Self {
            root,
            workspace,
            session,
            kernel,
            authority,
            token,
            frame,
        })
    }
    fn journal(&self) -> Result<KernelFrameJournal, polaris_kernel::KernelFrameJournalError> {
        KernelFrameJournal::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            FrameJournalLimits::default(),
        )
    }
    fn record(&self) -> FrameStepRecord {
        FrameStepRecord {
            schema: 1,
            frame: self.frame.tool_binding().frame_id(),
            step: 1,
            goal: "synthetic journal goal".to_owned(),
            request: None,
            assistant_response: None,
            result: None,
            executed_tool_results: Vec::new(),
            watchdog_memory: None,
            outcome: FrameStepOutcome::Failed,
        }
    }
    fn frame_events(&self) -> TestResult<usize> {
        Ok(self
            .kernel
            .context()
            .records(Seq::new(1), Seq::new(u64::MAX))?
            .iter()
            .filter(|r| matches!(r.event(), Event::FrameStepRecorded { .. }))
            .count())
    }
}

#[tokio::test]
async fn journal_persists_before_durable_live_event() -> TestResult {
    let f = Fixture::new().await?;
    let records = f
        .kernel
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    let cursor = records
        .last()
        .ok_or("session events missing")?
        .seq()
        .get()
        .checked_add(1)
        .ok_or("cursor overflow")?;
    let mut stream = f.kernel.subscribe(Seq::new(cursor)).await?;
    let journal = f.journal()?;
    let record = f.record();
    let cid = journal.record(&record).await?;
    let event = tokio::time::timeout(Duration::from_secs(5), stream.next_event())
        .await?
        .ok_or("frame event missing")?;
    assert_eq!(event.session(), f.session);
    assert!(
        matches!(event.event(),Event::FrameStepRecorded{frame,step:1,transcript,outcome:FrameStepOutcome::Failed} if *frame==record.frame && *transcript==cid)
    );
    let expanded = f
        .kernel
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":cid.to_hex()}),
        )
        .await?;
    let text = expanded
        .value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or("transcript missing")?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(text)?,
        serde_json::to_value(&record)?
    );
    assert_eq!(
        polaris_kernel::storage::cas::content_id(text.as_bytes()),
        cid
    );
    assert_eq!(f.frame_events()?, 1);
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn journal_preserves_actual_executed_tool_result() -> TestResult {
    let f = Fixture::new().await?;
    let tools = KernelToolRunner::new(
        f.kernel.clone(),
        f.frame.tool_binding(),
        f.authority.public_key(),
        CancellationToken::new(),
    )?;
    let p = tools
        .authorize(
            &f.token,
            ToolCall {
                id: "synthetic-call".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{\"path\":\"sample.txt\"}".to_owned(),
            },
        )
        .await?;
    let result = tools
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    let mut record = f.record();
    record.executed_tool_results.push(ExecutedToolResult {
        tool_call_id: "synthetic-call".to_owned(),
        result: result.clone(),
    });
    let cid = f.journal()?.record(&record).await?;
    let expanded = f
        .kernel
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":cid.to_hex()}),
        )
        .await?;
    let text = expanded
        .value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or("transcript missing")?;
    let value: serde_json::Value = serde_json::from_str(text)?;
    let persisted = value
        .get("executed_tool_results")
        .and_then(serde_json::Value::as_array)
        .and_then(|r| r.first())
        .and_then(|r| r.get("result"))
        .ok_or("executed result missing")?;
    assert_eq!(
        persisted.get("content").and_then(serde_json::Value::as_str),
        Some(result.content.as_str())
    );
    assert_eq!(f.frame_events()?, 1);
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn journal_rejects_foreign_frame_missing_content_and_oversize() -> TestResult {
    let f = Fixture::new().await?;
    let other = Fixture::new().await?;
    assert!(
        KernelFrameJournal::new(
            f.kernel.clone(),
            other.frame.tool_binding(),
            FrameJournalLimits::default()
        )
        .is_err()
    );
    assert!(
        KernelFrameJournal::new(
            f.kernel.clone(),
            f.frame.tool_binding(),
            FrameJournalLimits {
                max_record_bytes: 0,
                max_tool_result_bytes: 1,
                max_tool_results: 1
            }
        )
        .is_err()
    );
    let _report = other.kernel.shutdown().await?;
    let journal = f.journal()?;
    let mut foreign = f.record();
    foreign.frame = FrameId::new_v7();
    assert!(journal.record(&foreign).await.is_err());
    let mut missing = f.record();
    missing.executed_tool_results.push(ExecutedToolResult {
        tool_call_id: "synthetic-call".to_owned(),
        result: polaris_runtime::frames::ToolResultRef {
            cid: Cid::from_bytes([19; 32]),
            content_cid: Cid::from_bytes([19; 32]),
            content: "missing synthetic content".to_owned(),
        },
    });
    assert!(journal.record(&missing).await.is_err());
    let bounded = KernelFrameJournal::new(
        f.kernel.clone(),
        f.frame.tool_binding(),
        FrameJournalLimits {
            max_record_bytes: 256,
            max_tool_result_bytes: 128,
            max_tool_results: 2,
        },
    )?;
    let mut oversized = f.record();
    oversized.goal = "synthetic ".repeat(128);
    assert!(bounded.record(&oversized).await.is_err());
    assert_eq!(
        f.frame_events()?,
        0,
        "rejected journal attempts publish no event"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn journal_debug_hides_kernel_paths() -> TestResult {
    let f = Fixture::new().await?;
    let journal = f.journal()?;
    let debug = format!("{journal:?}");
    assert!(
        !debug.contains(&format!("{:?}", f.root.path().canonicalize()?)),
        "journal Debug must hide canonical durable root paths"
    );
    assert!(
        !debug.contains(&format!("{:?}", f.workspace.path().canonicalize()?)),
        "journal Debug must hide canonical workspace paths"
    );
    assert!(
        !debug.contains(&format!("{:?}", f.root.path())),
        "journal Debug must hide escaped durable root paths"
    );
    assert!(
        !debug.contains(&format!("{:?}", f.workspace.path())),
        "journal Debug must hide escaped workspace paths"
    );
    assert!(
        !debug.contains(f.root.path().to_string_lossy().as_ref()),
        "journal Debug must hide the durable state root"
    );
    assert!(
        !debug.contains(f.workspace.path().to_string_lossy().as_ref()),
        "journal Debug must hide the workspace root"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn closed_admission_precedes_transcript_preparation() -> TestResult {
    let f = Fixture::new().await?;
    let journal = KernelFrameJournal::new(
        f.kernel.clone(),
        f.frame.tool_binding(),
        FrameJournalLimits {
            max_record_bytes: 256,
            max_tool_result_bytes: 128,
            max_tool_results: 1,
        },
    )?;
    let mut record = f.record();
    record.goal = "synthetic oversized goal".repeat(64);
    let _report = f.kernel.shutdown().await?;
    assert!(
        matches!(
            journal.record(&record).await,
            Err(polaris_kernel::KernelFrameJournalError::Admission(_))
        ),
        "closed admission must reject before allocating a prepared transcript"
    );
    Ok(())
}

#[tokio::test]
async fn journal_survives_restart_and_refuses_closed_admission() -> TestResult {
    let f = Fixture::new().await?;
    let frame = f.frame.tool_binding().frame_id();
    let journal = f.journal()?;
    let record = f.record();
    let cid = journal.record(&record).await?;
    let _report = f.kernel.shutdown().await?;
    assert!(journal.record(&record).await.is_err());
    drop(journal);
    drop(f.kernel);
    let reopened = Kernel::start(LocalKernelConfig::new(
        f.root.path(),
        f.workspace.path(),
        f.session,
        frame,
    ))
    .await?;
    let records = reopened
        .context()
        .records(Seq::new(1), Seq::new(u64::MAX))?;
    assert!(
        records.iter().any(
            |r| matches!(r.event(),Event::FrameStepRecorded{transcript,..} if *transcript==cid)
        )
    );
    let expanded = reopened
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":cid.to_hex()}),
        )
        .await?;
    assert_eq!(
        expanded
            .value
            .get("text")
            .and_then(serde_json::Value::as_str),
        Some(serde_json::to_string(&record)?.as_str())
    );
    let _report = reopened.shutdown().await?;
    Ok(())
}
