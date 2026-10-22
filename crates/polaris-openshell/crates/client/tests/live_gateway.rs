use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use polaris_openshell_client::{GatewayTls, OpenShellBackend, OpenShellConfig};
use polaris_openshell_policy::{AgentRole, ExecTier, policy_for};
use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxSpec};

/// Same operations as the unit suite, executed against the local P2 OpenShell gateway when enabled.
#[tokio::test]
async fn live_gateway_create_exec_sync_destroy() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("POLARIS_OPEN_SHELL_LIVE").is_none() {
        return Ok(());
    }
    let mtls_dir = std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls"));
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
        name: Some(format!("polaris-p2-{}", uuid::Uuid::now_v7().simple())),
        policy: Some(cid),
        ..Default::default()
    };
    let sandbox = backend.create(&spec).await?;
    let content = Bytes::from_static(b"polaris-openshell-p2-live-contract\n");
    let operations = async {
        let output = backend.exec(&sandbox, &ExecRequest {
            argv: vec!["sh".to_owned(), "-c".to_owned(), "printf p2-contract".to_owned()],
            ..Default::default()
        }).await?;
        if output.exit_code != 0 || output.stdout.as_ref() != b"p2-contract" {
            return Err(polaris_sdk::registry::SandboxError::Protocol("live exec output mismatch".to_owned()));
        }
        backend.upload(&sandbox, "/sandbox/polaris-p2-contract.bin", content.clone()).await?;
        let downloaded = backend.download(&sandbox, "/sandbox/polaris-p2-contract.bin").await?;
        if downloaded != content { return Err(polaris_sdk::registry::SandboxError::Protocol("live upload/download bytes differ".to_owned())); }
        let files = backend.list_files(&sandbox, "/sandbox").await?;
        if !files.iter().any(|path| path == "/sandbox/polaris-p2-contract.bin") {
            return Err(polaris_sdk::registry::SandboxError::Protocol("live file listing omitted uploaded file".to_owned()));
        }
        Ok::<(), polaris_sdk::registry::SandboxError>(())
    }.await;
    let cleanup = backend.destroy(&sandbox).await;
    operations?;
    cleanup?;
    Ok(())
}
