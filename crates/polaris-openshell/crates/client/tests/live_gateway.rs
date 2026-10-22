use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use polaris_openshell_client::{GatewayTls, OpenShellBackend, OpenShellConfig};
use polaris_openshell_policy::{AgentRole, ExecTier, policy_for};
use polaris_sdk::registry::{
    CommitOutcome, ExecRequest, SandboxBackend, SandboxCommitAction, SandboxCommitRequest,
    SandboxFileState, SandboxSpec,
};

/// Same operations as the unit suite, executed against the local P2 OpenShell gateway when enabled.
#[ignore = "requires an explicitly configured local OpenShell gateway and mTLS certificates"]
#[tokio::test]
async fn live_gateway_create_exec_sync_destroy() -> Result<(), Box<dyn Error>> {
    let image = std::env::var("POLARIS_OPEN_SHELL_IMAGE").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "set POLARIS_OPEN_SHELL_IMAGE to the locally built immutable sha256:<image-id>",
        )
    })?;
    let mtls_dir = std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls",
            )
        });
    let tls = GatewayTls {
        ca_certificate: std::fs::read(mtls_dir.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls_dir.join("tls.crt"))?,
        client_key: std::fs::read(mtls_dir.join("tls.key"))?,
        server_name: "localhost".to_owned(),
    };
    let config = OpenShellConfig {
        endpoint: "https://localhost:17670".to_owned(),
        tls,
        expected_version: "0.1.4".to_owned(),
        driver: polaris_openshell_client::DriverKind::Docker,
        ready_timeout: Duration::from_secs(120),
        exec_timeout: Duration::from_secs(30),
    };
    let backend = OpenShellBackend::connect(config).await?;
    let policy = policy_for(AgentRole::Reader, ExecTier::Docker)?;
    let cid = backend.register_policy(&policy)?;
    let spec = SandboxSpec {
        image: Some(image),
        name: Some(format!(
            "p2-{}",
            uuid::Uuid::now_v7()
                .simple()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        )),
        policy: Some(cid),
        cpu_millis: Some(1_000),
        memory_mib: Some(512),
        ..Default::default()
    };
    let sandbox = backend.create(&spec).await?;
    match backend.create(&spec).await {
        Err(polaris_sdk::registry::SandboxError::AlreadyExists(_)) => {}
        Err(error) => {
            return Err(polaris_sdk::registry::SandboxError::Protocol(format!(
                "duplicate create returned the wrong typed error: {error}"
            ))
            .into());
        }
        Ok(duplicate) => {
            if duplicate != sandbox {
                backend.destroy(&duplicate).await?;
            }
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "duplicate create unexpectedly succeeded".to_owned(),
            )
            .into());
        }
    }
    let content = Bytes::from_static(b"polaris-openshell-p2-live-contract\n");
    let operations = async {
        let output = backend
            .exec(
                &sandbox,
                &ExecRequest {
                    argv: vec![
                        "sh".to_owned(),
                        "-c".to_owned(),
                        "printf p2-contract".to_owned(),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        if output.exit_code != 0 || output.stdout.as_ref() != b"p2-contract" {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "live exec output mismatch".to_owned(),
            ));
        }
        let echoed = backend
            .exec(
                &sandbox,
                &ExecRequest {
                    argv: vec!["cat".to_owned()],
                    stdin: Some(content.clone()),
                    ..Default::default()
                },
            )
            .await?;
        if echoed.exit_code != 0 || echoed.stdout != content {
            return Err(polaris_sdk::registry::SandboxError::Protocol(format!(
                "live exec stdin echo mismatch: exit_code={}, expected_len={}, received_len={}",
                echoed.exit_code,
                content.len(),
                echoed.stdout.len(),
            )));
        }
        backend
            .upload(
                &sandbox,
                "/sandbox/polaris-p2-contract.bin",
                content.clone(),
            )
            .await?;
        let downloaded = backend
            .download(&sandbox, "/sandbox/polaris-p2-contract.bin")
            .await?;
        if downloaded != content {
            let expected = content
                .iter()
                .take(32)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let received = downloaded
                .iter()
                .take(32)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            return Err(polaris_sdk::registry::SandboxError::Protocol(format!(
                "live upload/download bytes differ: expected_len={}, received_len={}, expected_prefix_hex={expected}, received_prefix_hex={received}",
                content.len(),
                downloaded.len(),
            )));
        }
        let files = backend.list_files(&sandbox, "/sandbox").await?;
        if !files
            .iter()
            .any(|path| path == "/sandbox/polaris-p2-contract.bin")
        {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "live file listing omitted uploaded file".to_owned(),
            ));
        }
        let sync_root = "/sandbox/p2-sync-contract";
        let made_sync_root = backend
            .exec(
                &sandbox,
                &ExecRequest {
                    argv: vec![
                        "mkdir".to_owned(),
                        "-p".to_owned(),
                        sync_root.to_owned(),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        if made_sync_root.exit_code != 0 {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "could not create live sync contract root".to_owned(),
            ));
        }
        backend.prepare_sync(&sandbox, sync_root).await?;
        backend
            .upload(
                &sandbox,
                &format!("{sync_root}/polaris-p2-contract.bin"),
                content.clone(),
            )
            .await?;
        let toolchain_dir = "/sandbox/.polaris/openshell-toolchain-contract";
        let made_toolchain_dir = backend
            .exec(
                &sandbox,
                &ExecRequest {
                    argv: vec![
                        "mkdir".to_owned(),
                        "-p".to_owned(),
                        format!("{toolchain_dir}/src"),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        if made_toolchain_dir.exit_code != 0 {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "could not create Rust toolchain contract workspace".to_owned(),
            ));
        }
        backend
            .upload(
                &sandbox,
                &format!("{toolchain_dir}/Cargo.toml"),
                Bytes::from_static(
                    b"[package]\nname = \"polaris-live-toolchain-contract\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde_json = \"1.0.154\"\n",
                ),
            )
            .await?;
        backend
            .upload(
                &sandbox,
                &format!("{toolchain_dir}/src/lib.rs"),
                Bytes::from_static(
                    b"#[test]\nfn offline_dependency_cache_executes_test_binaries() { assert_eq!(serde_json::json!({\"answer\": 42}).to_string(), \"{\\\"answer\\\":42}\"); }\n",
                ),
            )
            .await?;
        let cargo_test = backend
            .exec(
                &sandbox,
                &ExecRequest {
                    argv: vec![
                        "cargo".to_owned(),
                        "test".to_owned(),
                        "--offline".to_owned(),
                        "--manifest-path".to_owned(),
                        format!("{toolchain_dir}/Cargo.toml"),
                    ],
                    timeout: Some(Duration::from_secs(120)),
                    env: vec![
                        ("CARGO_HOME".to_owned(), "/sandbox/cargo".to_owned()),
                        ("CARGO_TARGET_DIR".to_owned(), "/sandbox/build".to_owned()),
                        ("CARGO_NET_OFFLINE".to_owned(), "true".to_owned()),
                        ("RUSTUP_HOME".to_owned(), "/usr/local/rustup".to_owned()),
                        ("RUSTUP_TOOLCHAIN".to_owned(), "1.99.0".to_owned()),
                        ("RUSTUP_AUTO_INSTALL".to_owned(), "0".to_owned()),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        if cargo_test.exit_code != 0
            || !String::from_utf8_lossy(&cargo_test.stdout).contains("1 passed")
        {
            return Err(polaris_sdk::registry::SandboxError::Protocol(format!(
                "sandbox cargo test failed: exit_code={}, stdout={}, stderr={}",
                cargo_test.exit_code,
                String::from_utf8_lossy(&cargo_test.stdout),
                String::from_utf8_lossy(&cargo_test.stderr),
            )));
        }
        let snapshot = backend
            .snapshot_page(&sandbox, sync_root, None, 0, 128)
            .await?;
        if !snapshot
            .entries
            .iter()
            .any(|entry| entry.relative_path == "polaris-p2-contract.bin")
        {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "live sync snapshot omitted uploaded file".to_owned(),
            ));
        }
        if let Some(next_offset) = snapshot.next_offset {
            let next_page = backend
                .snapshot_page(
                    &sandbox,
                    "/sandbox",
                    Some(&snapshot.generation),
                    next_offset,
                    128,
                )
                .await?;
            if next_page.generation != snapshot.generation
                || next_page.manifest_id != snapshot.manifest_id
                || next_page.offset != next_offset
            {
                return Err(polaris_sdk::registry::SandboxError::Protocol(
                    "live sync snapshot page changed generation".to_owned(),
                ));
            }
        }
        let stage_dir = format!("{sync_root}/.polaris/sync/staging");
        let staged_path = format!("{stage_dir}/contract-stage.bin");
        let destination = format!("{sync_root}/polaris-p2-committed.bin");
        backend
            .upload(&sandbox, &staged_path, content.clone())
            .await?;
        let content_hash = polaris_sdk::ids::Cid::from_bytes(*blake3::hash(&content).as_bytes());
        let commit = SandboxCommitRequest {
            root: sync_root.to_owned(),
            relative_path: "polaris-p2-committed.bin".to_owned(),
            expected: None,
            action: SandboxCommitAction::File {
                staged_path: staged_path.clone(),
                hash: content_hash,
                executable: false,
            },
        };
        if backend.conditional_commit(&sandbox, &commit).await? != CommitOutcome::Applied {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "initial live conditional commit did not apply".to_owned(),
            ));
        }
        if backend.download(&sandbox, &destination).await? != content {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "live conditional commit content mismatch".to_owned(),
            ));
        }
        let conflict = SandboxCommitRequest {
            expected: Some(SandboxFileState {
                hash: polaris_sdk::ids::Cid::from_bytes([0; 32]),
                executable: false,
                link_target: None,
            }),
            ..commit
        };
        if backend.conditional_commit(&sandbox, &conflict).await? != CommitOutcome::Conflict {
            return Err(polaris_sdk::registry::SandboxError::Protocol(
                "live stale conditional commit did not conflict".to_owned(),
            ));
        }
        Ok::<(), polaris_sdk::registry::SandboxError>(())
    }
    .await;
    let cleanup = backend.destroy(&sandbox).await;
    operations?;
    cleanup?;
    Ok(())
}
