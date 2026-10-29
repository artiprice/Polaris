// Unit tests of crates/polaris-kernel/src/runtime.rs (`user_input_cas_tests`); compiled into that module via #[path].
use super::{Kernel, KernelHandle, LocalKernelConfig, MAX_CONTROL_BYTES};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event},
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
    registry::{KernelApi, KernelError as ApiError},
};
use std::time::Duration;

async fn submit_raw_cas(
    handle: &KernelHandle,
    bytes: &[u8],
) -> Result<Result<polaris_sdk::registry::CommandReceipt, ApiError>, Box<dyn std::error::Error>> {
    let cid: Cid = handle.shared.ctx.cas.put(bytes)?;
    let token = handle
        .shared
        .backends
        .control
        .credential(Duration::from_secs(30))?;
    Ok(handle
        .submit(Command::new(
            CommandId::new_v7(),
            handle.frame(),
            token,
            CommandBody::SubmitUserInput { text: cid },
        ))
        .await)
}

async fn started()
-> Result<(KernelHandle, tempfile::TempDir, tempfile::TempDir), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        SessionId::new_v7(),
        FrameId::new_v7(),
    ))
    .await?;
    Ok((handle, root, workspace))
}

fn user_input_count(handle: &KernelHandle) -> Result<usize, Box<dyn std::error::Error>> {
    Ok(handle
        .context()
        .records(Seq::ZERO, Seq::new(u64::MAX))?
        .iter()
        .filter(|record| matches!(record.event(), Event::UserInputSubmitted { .. }))
        .count())
}

#[tokio::test]
async fn non_utf8_cas_user_input_is_rejected_by_execute() -> Result<(), Box<dyn std::error::Error>>
{
    let (handle, _root, _workspace) = started().await?;
    let outcome = submit_raw_cas(&handle, &[0xff, 0xfe, 0xfd]).await?;
    assert!(
        matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input is not UTF-8"),
        "unexpected outcome {outcome:?}"
    );
    assert_eq!(user_input_count(&handle)?, 0, "nothing was appended");
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn oversize_cas_user_input_is_rejected_by_execute() -> Result<(), Box<dyn std::error::Error>>
{
    let (handle, _root, _workspace) = started().await?;
    let outcome = submit_raw_cas(&handle, &vec![b'a'; MAX_CONTROL_BYTES + 1]).await?;
    assert!(
        matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input exceeds limit"),
        "unexpected outcome {outcome:?}"
    );
    assert_eq!(user_input_count(&handle)?, 0, "nothing was appended");
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn blank_cas_user_input_is_rejected_by_execute() -> Result<(), Box<dyn std::error::Error>> {
    let (handle, _root, _workspace) = started().await?;
    let outcome = submit_raw_cas(&handle, b" \n\t").await?;
    assert!(
        matches!(&outcome, Err(ApiError::Rejected(message)) if message == "user input is empty"),
        "unexpected outcome {outcome:?}"
    );
    let _report = handle.shutdown().await?;
    Ok(())
}
