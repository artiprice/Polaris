//! Offline typed user message recording (descriptor: specs/descriptors/kernel_submit_user_input.json).
//! Real redb event log, CAS and signed local-control credentials; no model or provider.
use polaris_kernel::storage::cas::{Cas, CasConfig};
use polaris_kernel::{Kernel, KernelHandle, LocalKernelConfig};
use polaris_sdk::{
    contracts::{Event, EventRecord},
    ids::{FrameId, Seq, SessionId},
    registry::KernelError as ApiError,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

const MAX_CONTROL_BYTES: usize = 65_536;

async fn start(
    root: &std::path::Path,
    workspace: &std::path::Path,
    session: SessionId,
) -> Result<KernelHandle, Box<dyn Error>> {
    Ok(Kernel::start(LocalKernelConfig::new(
        root,
        workspace,
        session,
        FrameId::from_u128(2),
    ))
    .await?)
}

fn all_records(handle: &KernelHandle) -> Result<Vec<EventRecord>, Box<dyn Error>> {
    Ok(handle
        .context()
        .records(Seq::ZERO, Seq::new(u64::MAX))?
        .to_vec())
}

fn user_inputs(records: &[EventRecord]) -> Vec<&EventRecord> {
    records
        .iter()
        .filter(|record| matches!(record.event(), Event::UserInputSubmitted { .. }))
        .collect()
}

fn read_text(root: &std::path::Path, record: &EventRecord) -> Result<String, Box<dyn Error>> {
    let Event::UserInputSubmitted { text } = record.event() else {
        return Err("not a user input record".into());
    };
    let cas = Cas::open(CasConfig::new(root.join("cas")))?;
    let bytes = cas.get_bounded(text, 65_536)?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

#[tokio::test]
async fn plain_prose_commits_user_input_event() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = start(root.path(), workspace.path(), SessionId::from_u128(1)).await?;
    let text = "  plan tomorrow: $not_a_directive @file and /slash stay verbatim\n";
    let receipt = handle.submit_user_input(text).await?;
    let records = all_records(&handle)?;
    let inputs = user_inputs(&records);
    let [record] = inputs.as_slice() else {
        return Err(format!("expected one user input record, got {}", inputs.len()).into());
    };
    assert_eq!(
        record.seq(),
        receipt.accepted_at,
        "receipt names the record"
    );
    assert_eq!(
        read_text(root.path(), record)?,
        text,
        "CAS bytes equal text"
    );
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn blank_user_input_is_rejected_without_event() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = start(root.path(), workspace.path(), SessionId::from_u128(1)).await?;
    let before = all_records(&handle)?.len();
    for blank in ["", "   ", "\n\t \r\n"] {
        let error = handle
            .submit_user_input(blank)
            .await
            .err()
            .ok_or("blank input was accepted")?;
        assert!(
            matches!(&error, ApiError::Rejected(message) if message == "user input is empty"),
            "unexpected error {error:?}"
        );
    }
    assert_eq!(all_records(&handle)?.len(), before, "nothing was appended");
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn oversize_user_input_is_rejected() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = start(root.path(), workspace.path(), SessionId::from_u128(1)).await?;
    let before = all_records(&handle)?.len();
    let text = "a".repeat(MAX_CONTROL_BYTES + 1);
    assert!(handle.submit_user_input(&text).await.is_err());
    assert_eq!(all_records(&handle)?.len(), before, "nothing was appended");
    let exact = "a".repeat(MAX_CONTROL_BYTES);
    handle.submit_user_input(&exact).await?;
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn shutdown_closes_user_input_admission() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = start(root.path(), workspace.path(), SessionId::from_u128(1)).await?;
    let _report = handle.shutdown().await?;
    assert!(handle.submit_user_input("hello").await.is_err());
    Ok(())
}

#[tokio::test]
async fn user_input_replays_after_restart() -> TestResult {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let session = SessionId::from_u128(1);
    let handle = start(root.path(), workspace.path(), session).await?;
    handle.submit_user_input("first message").await?;
    handle.apply_directive("$websearch(off)").await?;
    handle.submit_user_input("second message").await?;
    let _report = handle.shutdown().await?;
    drop(handle);

    let reopened = start(root.path(), workspace.path(), session).await?;
    let records = all_records(&reopened)?;
    let texts = user_inputs(&records)
        .into_iter()
        .map(|record| read_text(root.path(), record))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(texts, ["first message", "second message"]);
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event(), Event::DirectiveApplied { .. })),
        "directive path is unchanged"
    );
    let _report = reopened.shutdown().await?;
    Ok(())
}
