use polaris_platform_process::ProcessPlatform;
use polaris_sdk::registry::{LocalEndpointOwnership, PlatformOps, SpawnSpec, StopOutcome};
use std::{
    error::Error,
    ffi::OsString,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    time::Duration,
};

const FIXTURE_PORT_ENV: &str = "POLARIS_SYNTHETIC_LISTENER_PORT";
const FIXTURE_STATUS_ENV: &str = "POLARIS_SYNTHETIC_LISTENER_STATUS";

/// Synthetic child used only by the integration test below. It binds a local TCP listener and
/// stays alive; it does not serve a model or make any production identity claim.
#[test]
fn synthetic_listener_fixture() -> Result<(), Box<dyn Error>> {
    let Some(port) = std::env::var_os(FIXTURE_PORT_ENV) else {
        return Ok(());
    };
    let status = std::env::var_os(FIXTURE_STATUS_ENV).ok_or("fixture status path missing")?;
    let port = port.to_string_lossy().parse::<u16>()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(error) => {
            std::fs::write(status, error.to_string())?;
            return Err(Box::new(error));
        }
    };
    std::fs::write(status, "bound")?;
    let _listener = listener;
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

#[tokio::test]
async fn retained_root_owns_only_its_exact_loopback_listener() -> Result<(), Box<dyn Error>> {
    let port = reserve_loopback_port()?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let directory = tempfile::tempdir()?;
    let status_path = directory.path().join("listener_status.txt");
    let platform = ProcessPlatform::new();
    let executable = std::env::current_exe()?;
    let tree = platform.spawn_tree(&fixture_spec(executable, port, status_path.as_path()))?;

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| {
        let status = match std::fs::read_to_string(status_path.as_path()) {
            Ok(status) => status,
            Err(_) => "fixture never reported startup".to_owned(),
        };
        std::io::Error::other(format!("synthetic listener did not bind: {status}"))
    })?;

    let owner = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if platform.owns_loopback_tcp_listener(&tree, address).await? {
                return Ok::<bool, polaris_sdk::registry::PlatformError>(true);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    assert!(owner);

    let wrong_port = SocketAddr::from(([127, 0, 0, 1], reserve_loopback_port()?));
    assert!(
        !platform
            .owns_loopback_tcp_listener(&tree, wrong_port)
            .await?
    );

    let stopped = platform
        .graceful_stop(&tree, Duration::from_millis(250))
        .await?;
    assert!(matches!(
        stopped,
        StopOutcome::Exited { .. } | StopOutcome::Killed
    ));
    Ok(())
}

fn reserve_loopback_port() -> Result<u16, std::io::Error> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    Ok(listener.local_addr()?.port())
}

fn fixture_spec(executable: PathBuf, port: u16, status_path: &std::path::Path) -> SpawnSpec {
    let env = vec![
        (
            OsString::from(FIXTURE_PORT_ENV),
            OsString::from(port.to_string()),
        ),
        (
            OsString::from(FIXTURE_STATUS_ENV),
            status_path.as_os_str().to_owned(),
        ),
    ];
    #[cfg(windows)]
    let env = {
        let mut platform_env = env;
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            platform_env.push((OsString::from("SystemRoot"), system_root));
        }
        platform_env
    };
    SpawnSpec {
        program: executable,
        args: vec![
            OsString::from("--exact"),
            OsString::from("synthetic_listener_fixture"),
            OsString::from("--nocapture"),
        ],
        cwd: std::env::current_dir().ok(),
        env,
    }
}
