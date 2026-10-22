//! Actual composition-root tool backends; all content below is labelled synthetic test data.
use polaris_kernel::{Kernel, LocalKernelConfig, storage::registry::Registry};
use polaris_sdk::{
    contracts::Timestamp,
    ids::{FrameId, SessionId},
    registry::ChatRole,
};
#[tokio::test]
async fn local_tools_use_actual_workspace_cas_and_scoped_history()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(
        workspace.path().join("sample.txt"),
        "synthetic workspace bytes\r\n",
    )?;
    let session = SessionId::from_u128(101);
    let handle = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        session,
        FrameId::from_u128(102),
    ))
    .await?;
    let output = handle
        .invoke_tool("read_file", serde_json::json!({"path":"sample.txt"}))
        .await?;
    assert_eq!(output.value["content"], "synthetic workspace bytes\r\n");
    assert!(
        handle
            .invoke_tool("read_file", serde_json::json!({"path":"../secret.txt"}))
            .await
            .is_err()
    );
    let cid = handle.store_text("synthetic CAS content").await?;
    let expanded = handle
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":cid.to_hex()}),
        )
        .await?;
    assert_eq!(expanded.value["text"], "synthetic CAS content");
    let registry = Registry::open(root.path())?;
    registry.index_message(
        session,
        ChatRole::User,
        "synthetic recall active",
        Timestamp::UNIX_EPOCH,
    )?;
    registry.index_message(
        SessionId::from_u128(999),
        ChatRole::User,
        "synthetic recall other",
        Timestamp::now(),
    )?;
    let recalled = handle
        .invoke_tool(
            "polaris_recall",
            serde_json::json!({"query":"synthetic recall"}),
        )
        .await?;
    let records = recalled.value["records"]
        .as_array()
        .ok_or("recall records")?;
    assert_eq!(
        records.len(),
        1,
        "history backend enforces the active session before LIMIT"
    );
    assert_eq!(
        records.first().ok_or("recall hit")?["snippet"],
        "synthetic recall active"
    );
    let _report = handle.shutdown().await?;
    assert!(
        handle
            .invoke_tool("read_file", serde_json::json!({"path":"sample.txt"}))
            .await
            .is_err()
    );
    Ok(())
}
