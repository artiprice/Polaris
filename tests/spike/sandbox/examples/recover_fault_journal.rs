//! Explicit recovery of an interrupted private gateway fault test. No wildcard deletion.
use polaris_kernel_reaper::{Locator, Reaper, ResourceKind};
use polaris_openshell::client::{GatewayTls, OpenShellBackend, OpenShellConfig};
use polaris_sdk::registry::SandboxHandle;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| "TLS provider")?;
    let state = std::env::var("POLARIS_FAULT_GATEWAY_STATE")?;
    let token = state
        .strip_prefix("/home/dev/.local/state/polaris/p2-gateway.")
        .ok_or("private gateway state required")?;
    if token.len() != 10 || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("invalid private state identity".into());
    }
    let mtls = PathBuf::from(
        std::env::var_os("POLARIS_OPEN_SHELL_MTLS_DIR").ok_or("mTLS directory required")?,
    );
    let mut config = OpenShellConfig::wsl2(GatewayTls {
        ca_certificate: std::fs::read(mtls.join("ca.crt"))?,
        client_certificate: std::fs::read(mtls.join("tls.crt"))?,
        client_key: std::fs::read(mtls.join("tls.key"))?,
        server_name: "localhost".into(),
    });
    config.endpoint = "https://localhost:17771".into();
    let backend = OpenShellBackend::connect(config).await?;
    let journal = PathBuf::from(
        std::env::var_os("POLARIS_FAULT_REAPER_DIR").ok_or("retained reaper directory required")?,
    );
    if !journal.join("reaper.journal").is_file() {
        return Err("an existing recovery journal is required".into());
    }
    let reaper = Reaper::new(journal)?;
    let pending = reaper.pending_orphans();
    if pending.is_empty() {
        return Err("journal contains no unresolved ownership".into());
    }
    for entry in &pending {
        let Locator::Label(name) = &entry.locator else {
            return Err("non-sandbox locator retained for operator".into());
        };
        let suffix = name.strip_prefix("sb-").ok_or("not a burst-test sandbox")?;
        if entry.kind != ResourceKind::Sandbox
            || suffix.len() != 16
            || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(
                "journal contains a resource outside the burst-test identity format".into(),
            );
        }
    }
    for entry in pending {
        let Locator::Label(name) = &entry.locator else {
            return Err("validated locator changed".into());
        };
        let handle = SandboxHandle::new(name)?;
        println!(
            "BEFORE_RECOVERY {name} {:?}",
            backend.inspect(&handle).await?
        );
        if backend.inspect(&handle).await?.is_some() {
            return Err(
                "gateway still retains the sandbox name; ownership remains unresolved".into(),
            );
        }
        reaper.acknowledge_orphan(entry.id)?;
        println!("RECOVERED {name} authoritative_absence=true");
    }
    if !reaper.leak_report().entries.is_empty() {
        return Err("unresolved recovery records remain".into());
    }
    println!("RECOVERY_COMPLETE remaining_resources=0");
    Ok(())
}
