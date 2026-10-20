//! Spike S0-1: drive the WSL2 OpenShell gateway from Windows through `openshell-sdk`.
//!
//! The SDK's `ClientConfig` has no client-certificate option, but the local gateway
//! authenticates with mTLS, so the spike builds the tonic channel itself (CA + client
//! identity, server name `localhost`) and hands it to `OpenShellClient::from_parts`.
//!
//! Usage: `s0-1-openshell-sdk [GATEWAY_URL] [MTLS_DIR]`
//! Defaults: `https://localhost:17670` and
//! `\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls`.

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use openshell_sdk::{DeleteOptions, EdgeAuthInterceptor, ExecOptions, OpenShellClient, SandboxSpec};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

type AnyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const DEFAULT_GATEWAY: &str = "https://localhost:17670";
const DEFAULT_MTLS_DIR: &str = r"\\wsl.localhost\Ubuntu-24.04\home\dev\.config\openshell\gateways\openshell\mtls";
const SANDBOX: &str = "polaris-s0-1";

fn ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let mut args = std::env::args().skip(1);
    let gateway = args.next().unwrap_or_else(|| DEFAULT_GATEWAY.to_owned());
    let mtls_dir = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_MTLS_DIR.to_owned()));

    // ADR-005: install the aws-lc-rs provider explicitly before any TLS use.
    if rustls::crypto::aws_lc_rs::default_provider().install_default().is_err() {
        return Err("a rustls crypto provider was already installed".into());
    }

    let ca = std::fs::read(mtls_dir.join("ca.crt"))?;
    let cert = std::fs::read(mtls_dir.join("tls.crt"))?;
    let key = std::fs::read(mtls_dir.join("tls.key"))?;
    println!("mtls material: ca={}B cert={}B key={}B from {}", ca.len(), cert.len(), key.len(), mtls_dir.display());

    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca))
        .identity(Identity::from_pem(cert, key))
        .domain_name("localhost");

    let t = Instant::now();
    let channel = Endpoint::from_shared(gateway.clone())?
        .connect_timeout(Duration::from_secs(10))
        .tls_config(tls)?
        .connect()
        .await?;
    println!("connect {gateway}: ok ({} ms)", ms(t));

    let client = OpenShellClient::from_parts(channel, EdgeAuthInterceptor::noop());

    let t = Instant::now();
    let health = client.health().await?;
    println!("health: {:?} version={} ({} ms)", health.status, health.version, ms(t));

    let spec = SandboxSpec {
        name: Some(SANDBOX.to_owned()),
        command: vec!["sleep".to_owned(), "infinity".to_owned()],
        ..SandboxSpec::default()
    };
    let t = Instant::now();
    let created = client.create_sandbox(spec).await?;
    println!("create: id={} phase={:?} ({} ms)", created.id, created.phase, ms(t));

    let t = Instant::now();
    let ready = client.wait_ready(SANDBOX, Duration::from_secs(120)).await?;
    println!("wait_ready: phase={:?} ({} ms)", ready.phase, ms(t));

    let mut opts = ExecOptions::default();
    opts.no_login_shell = true;
    let cmd: Vec<String> = ["sh", "-c", "uname -sr; id -un; pwd"].iter().map(|s| (*s).to_owned()).collect();
    let t = Instant::now();
    let out = client.exec(SANDBOX, &cmd, opts.clone()).await?;
    println!(
        "exec: exit={} ({} ms)\n{}",
        out.exit_code,
        ms(t),
        String::from_utf8_lossy(&out.stdout).trim_end()
    );

    let fail: Vec<String> = ["sh", "-c", "exit 7"].iter().map(|s| (*s).to_owned()).collect();
    let out = client.exec(SANDBOX, &fail, opts).await?;
    let propagation_ok = out.exit_code == 7;
    println!("exit-code propagation: {} (got {})", if propagation_ok { "ok" } else { "FAILED" }, out.exit_code);

    let t = Instant::now();
    let deletion = client.delete_sandbox(SANDBOX, DeleteOptions::default()).await?;
    client.wait_deleted(SANDBOX, Duration::from_secs(60), Some(&created.id)).await?;
    println!("delete: {:?} -> gone ({} ms)", deletion.outcome, ms(t));

    if propagation_ok {
        println!("RESULT: PASS");
        Ok(())
    } else {
        Err("exit-code propagation failed".into())
    }
}
