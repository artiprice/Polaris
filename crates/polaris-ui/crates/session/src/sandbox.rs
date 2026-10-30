//! The OpenShell sandbox of interactive sessions (P2). When `~/.polaris/sandbox.json` names the
//! local gateway, every session kernel starts with the sandbox execution profile: the model's
//! `run_tests` prepares an immutable review, the person approves it in the review panel, and the
//! tests run in an isolated OpenShell sandbox with Proof of Done. A gateway in WSL is kept awake
//! while the session is open.
//!
//! `sandbox.json`: `endpoint` (`https://127.0.0.1:17670`), `server_name` (`localhost`),
//! `mtls_dir` (the gateway's client `ca.crt`, `tls.crt`, `tls.key`), `policy` (OpenShell policy
//! YAML), `image` (`sha256:…` of the sandbox image), optional `wsl_distro`, `cpu_millis`,
//! `memory_mib`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use polaris_kernel::{Kernel, KernelHandle, LocalKernelConfig};

/// The profile file, relative to the user's Polaris directory.
pub const PROFILE_FILE: &str = "sandbox.json";
const MAX_FILE_BYTES: u64 = 1_048_576;

/// Where the gateway is and what a sandbox may use.
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    endpoint: String,
    server_name: String,
    mtls_dir: PathBuf,
    policy: PathBuf,
    image: String,
    wsl_distro: Option<String>,
    cpu_millis: u32,
    memory_mib: u32,
}

static PROFILE: OnceLock<(Option<SandboxProfile>, String)> = OnceLock::new();

/// Read the sandbox profile once per process (frontends call it at startup); returns a status
/// line.
pub fn configure(global: &Path) -> String {
    PROFILE
        .get_or_init(|| match read_profile(&global.join(PROFILE_FILE)) {
            Ok(Some(profile)) => {
                let status = format!(
                    "OpenShell sandbox: {} (image {})",
                    profile.endpoint,
                    profile.image.get(..19).unwrap_or(&profile.image)
                );
                (Some(profile), status)
            }
            Ok(None) => (
                None,
                String::from(
                    "OpenShell sandbox: not configured (run @prime/tools/sandbox/bringup-wsl.sh, then write ~/.polaris/sandbox.json)",
                ),
            ),
            Err(reason) => (None, format!("OpenShell sandbox: profile rejected ({reason})")),
        })
        .1
        .clone()
}

/// The status line of [`configure`].
#[must_use]
pub fn status() -> String {
    PROFILE.get().map_or_else(
        || String::from("OpenShell sandbox: not configured"),
        |(_, status)| status.clone(),
    )
}

fn read_profile(path: &Path) -> Result<Option<SandboxProfile>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(String::from("unreadable")),
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(String::from("not a file of at most 1 MiB"));
    }
    let bytes = std::fs::read(path).map_err(|_| String::from("unreadable"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| String::from("not JSON"))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{key} is missing"))
    };
    let number = |key: &str, default: u32| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .unwrap_or(default)
    };
    let image = text("image")?;
    if !image.starts_with("sha256:") || image.len() != 71 {
        return Err(String::from(
            "image must be an immutable sha256:<64 hex> id",
        ));
    }
    Ok(Some(SandboxProfile {
        endpoint: text("endpoint")?,
        server_name: text("server_name").unwrap_or_else(|_| String::from("localhost")),
        mtls_dir: PathBuf::from(text("mtls_dir")?),
        policy: PathBuf::from(text("policy")?),
        image,
        wsl_distro: text("wsl_distro").ok(),
        cpu_millis: number("cpu_millis", 2000),
        memory_mib: number("memory_mib", 2048),
    }))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path)
        .map_err(|_| format!("{} cannot be read", path.display()))?
        .len();
    if size == 0 || size > MAX_FILE_BYTES {
        return Err(format!("{} is empty or too large", path.display()));
    }
    std::fs::read(path).map_err(|_| format!("{} cannot be read", path.display()))
}

/// What keeps a sandbox session's gateway reachable while it is open.
#[derive(Debug, Default)]
pub struct SandboxGuard {
    _keepalive: Option<polaris_platform::wsl::KeepAlive>,
}

/// Start a session kernel with the sandbox execution profile when one is configured; `None`
/// when no profile is configured. The kernel falls back to no sandbox on any failure, with the
/// reason.
pub(crate) async fn start_kernel(
    config: LocalKernelConfig,
) -> Option<Result<(KernelHandle, SandboxGuard), String>> {
    let profile = PROFILE.get()?.0.clone()?;
    Some(start_with(config, &profile).await)
}

async fn start_with(
    config: LocalKernelConfig,
    profile: &SandboxProfile,
) -> Result<(KernelHandle, SandboxGuard), String> {
    let keepalive = match &profile.wsl_distro {
        Some(distro) => {
            let wsl = polaris_platform::wsl::Wsl::locate()
                .map_err(|_| String::from("wsl.exe was not found"))?;
            let name = polaris_platform::wsl::DistroName::new(distro)
                .map_err(|_| String::from("invalid WSL distribution name"))?;
            Some(
                wsl.keepalive(&name)
                    .await
                    .map_err(|_| format!("WSL {distro} could not be kept awake"))?,
            )
        }
        None => None,
    };
    let source = profile.clone();
    let (tls, policy) = tokio::task::spawn_blocking(move || {
        let tls = polaris_openshell::client::GatewayTls {
            ca_certificate: read_bounded(&source.mtls_dir.join("ca.crt"))?,
            client_certificate: read_bounded(&source.mtls_dir.join("tls.crt"))?,
            client_key: read_bounded(&source.mtls_dir.join("tls.key"))?,
            server_name: source.server_name.clone(),
        };
        let text = String::from_utf8(read_bounded(&source.policy)?)
            .map_err(|_| String::from("the sandbox policy is not UTF-8"))?;
        let policy = polaris_openshell::policy::Policy::parse_yaml(&text)
            .map_err(|_| String::from("the sandbox policy is invalid"))?;
        Ok::<_, String>((tls, policy))
    })
    .await
    .map_err(|_| String::from("sandbox profile reader failed"))??;
    let mut gateway = polaris_openshell::client::OpenShellConfig::wsl2(tls);
    gateway.endpoint.clone_from(&profile.endpoint);
    let backend = Arc::new(
        polaris_openshell::client::OpenShellBackend::connect(gateway)
            .await
            .map_err(|error| format!("the OpenShell gateway is not reachable ({error})"))?,
    );
    let policy = backend
        .register_policy(&policy)
        .map_err(|error| format!("the sandbox policy was refused ({error})"))?;
    let spec = polaris_sdk::registry::SandboxSpec {
        image: Some(profile.image.clone()),
        policy: Some(policy),
        cpu_millis: Some(profile.cpu_millis),
        memory_mib: Some(profile.memory_mib),
        ..polaris_sdk::registry::SandboxSpec::default()
    };
    let kernel = Kernel::start_sandbox(config, backend, spec)
        .await
        .map_err(|error| format!("the sandbox session did not start ({error})"))?;
    Ok((
        kernel,
        SandboxGuard {
            _keepalive: keepalive,
        },
    ))
}
