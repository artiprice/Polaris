//! Real temporary workspace, CAS and capability integration; fixture bytes are synthetic.
use polaris_kernel::{Kernel, KernelHandle, KernelToolRunner, LocalKernelConfig};
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    ToolCall, ToolRunner,
};
use polaris_sdk::{
    effort::Effort,
    ids::{Cid, FrameId, ModelId, SessionId},
};
use polaris_security::capability::{
    Authority, Clock, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: tempfile::TempDir,
    kernel: KernelHandle,
    session: SessionId,
    authority: Authority,
    frame: AgentContextFrame<Active>,
    token: Token,
}

impl Fixture {
    async fn new(resource: ResourceScope) -> TestResult<Self> {
        Self::with_rights(resource, Rights::new(&[Right::Read, Right::ExecuteTool])).await
    }

    async fn with_rights(resource: ResourceScope, rights: Rights) -> TestResult<Self> {
        Self::with_clock(resource, rights, &SystemClock).await
    }

    async fn with_clock(
        resource: ResourceScope,
        rights: Rights,
        clock: &impl Clock,
    ) -> TestResult<Self> {
        Self::with_role_and_clock(resource, rights, AgentRole::Orchestrator, clock).await
    }

    async fn with_role_and_clock(
        resource: ResourceScope,
        rights: Rights,
        role: AgentRole,
        clock: &impl Clock,
    ) -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        std::fs::write(
            workspace.path().join("sample.txt"),
            "synthetic exact bytes\n",
        )?;
        std::fs::create_dir(workspace.path().join("src"))?;
        std::fs::write(
            workspace.path().join("src/secret.txt"),
            "synthetic child bytes",
        )?;
        let id = FrameId::new_v7();
        let session = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights,
        };
        let token = authority.issue_root(id, &scope, clock)?;
        let expiry = clock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            id,
            role,
            token.clone(),
            FrameScope {
                capability: scope,
                expires_at_micros: expiry,
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
                model: ModelId::new("test/model")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        Ok(Self {
            _root: root,
            workspace,
            kernel,
            session,
            authority,
            frame,
            token,
        })
    }
    fn runner(&self) -> Result<KernelToolRunner, polaris_sdk::registry::ToolError> {
        KernelToolRunner::new(
            self.kernel.clone(),
            self.frame.tool_binding(),
            self.authority.public_key(),
            CancellationToken::new(),
        )
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "synthetic-call".to_owned(),
        name: name.to_owned(),
        arguments: arguments.to_string(),
    }
}

#[tokio::test]
async fn actual_file_result_round_trips_through_cas() -> TestResult {
    let f = Fixture::new(ResourceScope::Exact(ResourcePath::new("sample.txt")?)).await?;
    let runner = f.runner()?;
    let permit = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    let result = runner
        .execute(permit, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    let envelope: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(
        envelope
            .get("value")
            .and_then(|v| v.get("content"))
            .and_then(serde_json::Value::as_str),
        Some("synthetic exact bytes\n")
    );
    assert_eq!(
        envelope
            .get("truncated")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let read = f
        .kernel
        .invoke_tool(
            "polaris_expand",
            serde_json::json!({"reference":result.cid.to_hex()}),
        )
        .await?;
    assert_eq!(
        read.value.get("text").and_then(serde_json::Value::as_str),
        Some(result.content.as_str())
    );
    assert_eq!(result.cid, result.content_cid);
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn wrong_token_issuer_frame_and_runner_are_denied() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let runner = f.runner()?;
    let other = Authority::new_session()?;
    let wrong_issuer = KernelToolRunner::new(
        f.kernel.clone(),
        f.frame.tool_binding(),
        other.public_key(),
        CancellationToken::new(),
    )?;
    assert!(
        wrong_issuer
            .authorize(
                &f.token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    let scope = Scope {
        resource: ResourceScope::Workspace,
        max_churn_lines: 0,
        ttl: Duration::from_secs(60),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    let wrong_token = other.issue_root(f.frame.tool_binding().frame_id(), &scope, &SystemClock)?;
    assert!(
        runner
            .authorize(
                &wrong_token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    assert!(runner.execute(p, FrameId::new_v7(), 4096).await.is_err());
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    assert!(
        f.runner()?
            .execute(p, f.frame.tool_binding().frame_id(), 4096)
            .await
            .is_err()
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn recursive_reads_cannot_escape_exact_directory_scope() -> TestResult {
    let f = Fixture::new(ResourceScope::Exact(ResourcePath::new("src")?)).await?;
    let runner = f.runner()?;
    let p = runner
        .authorize(
            &f.token,
            call("list_dir", serde_json::json!({"path":"src"})),
        )
        .await?;
    assert!(
        runner
            .execute(p, f.frame.tool_binding().frame_id(), 4096)
            .await
            .is_err(),
        "descendant has no exact-directory authorization"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn revocation_between_authorize_and_execute_denies() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let runner = f.runner()?;
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    runner.revoke_token(&f.token)?;
    assert!(
        runner
            .execute(p, f.frame.tool_binding().frame_id(), 4096)
            .await
            .is_err()
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cas_visibility_and_output_caps_are_enforced() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let runner = f.runner()?;
    let cid = f.kernel.store_text("synthetic private CAS bytes").await?;
    assert!(
        runner
            .authorize(
                &f.token,
                call(
                    "polaris_expand",
                    serde_json::json!({"reference":cid.to_hex()})
                )
            )
            .await
            .is_err()
    );
    runner.allow_reference(cid)?;
    let p = runner
        .authorize(
            &f.token,
            call(
                "polaris_expand",
                serde_json::json!({"reference":cid.to_hex()}),
            ),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    assert!(result.content.contains("synthetic private CAS bytes"));
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    assert!(
        runner
            .execute(p, f.frame.tool_binding().frame_id(), 8)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(f.workspace.path().join("sample.txt"))?,
        "synthetic exact bytes\n"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn malformed_mutating_and_closed_calls_are_denied() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let runner = f.runner()?;
    for c in [
        call(
            "read_file",
            serde_json::json!({"path":"sample.txt","unknown":true}),
        ),
        call("shell", serde_json::json!({})),
        call("read_file", serde_json::json!({"path":"../outside"})),
    ] {
        assert!(runner.authorize(&f.token, c).await.is_err());
    }
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    let _report = f.kernel.shutdown().await?;
    assert!(
        runner
            .execute(p, f.frame.tool_binding().frame_id(), 4096)
            .await
            .is_err()
    );
    assert!(
        runner
            .authorize(
                &f.token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn pure_tools_require_execute_right_on_their_namespace() -> TestResult {
    let f = Fixture::with_rights(
        ResourceScope::Exact(ResourcePath::new("tools/json_query")?),
        Rights::new(&[Right::ExecuteTool]),
    )
    .await?;
    let runner = f.runner()?;
    let p = runner
        .authorize(
            &f.token,
            call(
                "json_query",
                serde_json::json!({"json":"{\"answer\":42}","pointer":"/answer"}),
            ),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    assert!(
        result.content.contains("42"),
        "real pure query result is retained"
    );
    assert!(
        runner
            .authorize(
                &f.token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    let _report = f.kernel.shutdown().await?;
    let read_only =
        Fixture::with_rights(ResourceScope::Workspace, Rights::new(&[Right::Read])).await?;
    assert!(
        read_only
            .runner()?
            .authorize(
                &read_only.token,
                call("json_query", serde_json::json!({"json":"42","pointer":""}))
            )
            .await
            .is_err()
    );
    let _report = read_only.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn short_references_preserve_capability_scope() -> TestResult {
    let bytes = "synthetic scoped CAS bytes";
    let cid = polaris_kernel::storage::cas::content_id(bytes.as_bytes());
    let f = Fixture::new(ResourceScope::Exact(ResourcePath::new(&format!(
        "cas/{}",
        cid.to_hex()
    ))?))
    .await?;
    assert_eq!(f.kernel.store_text(bytes).await?, cid);
    let runner = f.runner()?;
    runner.allow_reference(cid)?;
    let marker = polaris_sdk::ids::ShortRef::of(&cid).to_marker();
    let p = runner
        .authorize(
            &f.token,
            call("polaris_expand", serde_json::json!({"reference":marker})),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    assert!(result.content.contains(bytes));
    assert!(
        runner
            .authorize(
                &f.token,
                call(
                    "polaris_expand",
                    serde_json::json!({"reference":result.cid.to_hex()})
                )
            )
            .await
            .is_err(),
        "a result becomes visible but does not widen its token's exact CAS resource"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cancellation_and_input_bounds_reject_without_execution() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let cancellation = CancellationToken::new();
    let runner = KernelToolRunner::new(
        f.kernel.clone(),
        f.frame.tool_binding(),
        f.authority.public_key(),
        cancellation.clone(),
    )?;
    let p = runner
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    cancellation.cancel();
    assert!(
        runner
            .execute(p, f.frame.tool_binding().frame_id(), 4096)
            .await
            .is_err()
    );
    assert!(
        runner
            .authorize(
                &f.token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    let active = f.runner()?;
    let oversized = ToolCall {
        id: "synthetic-call".to_owned(),
        name: "read_file".to_owned(),
        arguments: " ".repeat(65_537),
    };
    assert!(active.authorize(&f.token, oversized).await.is_err());
    let p = active
        .authorize(
            &f.token,
            call("read_file", serde_json::json!({"path":"sample.txt"})),
        )
        .await?;
    assert!(
        active
            .execute(p, f.frame.tool_binding().frame_id(), 0)
            .await
            .is_err()
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn bounded_read_keeps_truncation_metadata() -> TestResult {
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let runner = f.runner()?;
    let p = runner
        .authorize(
            &f.token,
            call(
                "read_file",
                serde_json::json!({"path":"sample.txt", "limit":1}),
            ),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    let envelope: serde_json::Value = serde_json::from_str(&result.content)?;
    assert_eq!(
        envelope
            .get("truncated")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        envelope
            .get("value")
            .and_then(|v| v.get("content"))
            .and_then(serde_json::Value::as_str),
        Some("s")
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn expired_tokens_and_foreign_kernel_bindings_are_denied() -> TestResult {
    struct PastClock;
    impl Clock for PastClock {
        fn now(&self) -> polaris_sdk::contracts::Timestamp {
            polaris_sdk::contracts::Timestamp::UNIX_EPOCH
        }
    }
    let expired = Fixture::with_clock(
        ResourceScope::Workspace,
        Rights::new(&[Right::Read]),
        &PastClock,
    )
    .await?;
    assert!(
        expired
            .runner()?
            .authorize(
                &expired.token,
                call("read_file", serde_json::json!({"path":"sample.txt"}))
            )
            .await
            .is_err()
    );
    let actual = Fixture::new(ResourceScope::Workspace).await?;
    assert!(
        KernelToolRunner::new(
            actual.kernel.clone(),
            expired.frame.tool_binding(),
            expired.authority.public_key(),
            CancellationToken::new()
        )
        .is_err()
    );
    let _report = expired.kernel.shutdown().await?;
    let _report = actual.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn history_uses_actual_kernel_session() -> TestResult {
    use polaris_kernel::storage::registry::Registry;
    use polaris_sdk::{contracts::Timestamp, registry::ChatRole};
    let f = Fixture::new(ResourceScope::Workspace).await?;
    let registry = Registry::open(f._root.path())?;
    registry.index_message(
        f.session,
        ChatRole::User,
        "synthetic recall active",
        Timestamp::UNIX_EPOCH,
    )?;
    registry.index_message(
        SessionId::new_v7(),
        ChatRole::User,
        "synthetic recall other",
        Timestamp::UNIX_EPOCH,
    )?;
    let runner = f.runner()?;
    let p = runner
        .authorize(
            &f.token,
            call(
                "polaris_recall",
                serde_json::json!({"query":"synthetic recall"}),
            ),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 8192)
        .await?;
    assert!(result.content.contains("synthetic recall active"));
    assert!(
        !result.content.contains("synthetic recall other"),
        "recall cannot read a foreign session"
    );
    let _report = f.kernel.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn pure_only_role_cannot_read_persistent_cas_or_history() -> TestResult {
    let f = Fixture::with_role_and_clock(
        ResourceScope::Workspace,
        Rights::new(&[Right::Read, Right::ExecuteTool]),
        AgentRole::Hacker,
        &SystemClock,
    )
    .await?;
    let runner = f.runner()?;
    let cid = f.kernel.store_text("synthetic persistent bytes").await?;
    runner.allow_reference(cid)?;
    assert!(
        runner
            .authorize(
                &f.token,
                call(
                    "polaris_expand",
                    serde_json::json!({"reference":cid.to_hex()})
                )
            )
            .await
            .is_err(),
        "persistent CAS reads are not pure operations"
    );
    assert!(
        runner
            .authorize(
                &f.token,
                call("polaris_recall", serde_json::json!({"query":"synthetic"}))
            )
            .await
            .is_err(),
        "history reads are not pure operations"
    );
    let p = runner
        .authorize(
            &f.token,
            call("json_query", serde_json::json!({"json":"42","pointer":""})),
        )
        .await?;
    let result = runner
        .execute(p, f.frame.tool_binding().frame_id(), 4096)
        .await?;
    assert!(result.content.contains("42"));
    let _report = f.kernel.shutdown().await?;
    Ok(())
}
