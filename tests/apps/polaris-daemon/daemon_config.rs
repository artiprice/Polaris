//! Daemon Configuration (descriptor: specs/descriptors/daemon_config.json).
use polaris_daemon::{DaemonSettings, EndpointSettings};
use std::{net::SocketAddr, time::Duration};

#[test]
fn daemon_limits_accept_only_supported_bounds() -> Result<(), Box<dyn std::error::Error>> {
    let default = DaemonSettings::default();
    assert!(default.validate().is_ok());
    assert!(
        DaemonSettings {
            event_buffer: 0,
            ..default
        }
        .validate()
        .is_err()
    );
    assert!(
        DaemonSettings {
            event_buffer: 1025,
            ..default
        }
        .validate()
        .is_err()
    );
    assert!(
        DaemonSettings {
            shutdown_grace: Duration::ZERO,
            ..default
        }
        .validate()
        .is_err()
    );
    assert!(
        DaemonSettings {
            shutdown_grace: Duration::from_secs(121),
            ..default
        }
        .validate()
        .is_err()
    );
    Ok(())
}

#[test]
fn public_tcp_requires_mutual_tls_material() -> Result<(), Box<dyn std::error::Error>> {
    let public: SocketAddr = "0.0.0.0:43127".parse()?;
    let loopback: SocketAddr = "127.0.0.1:43127".parse()?;
    assert!(EndpointSettings::tcp(public, None, None, None).is_err());
    assert!(EndpointSettings::tcp(public, Some("cert"), Some("key"), None).is_err());
    assert!(EndpointSettings::tcp(public, Some("cert"), Some("key"), Some("client-ca")).is_ok());
    assert!(EndpointSettings::tcp(loopback, None, None, None).is_ok());
    Ok(())
}

#[test]
fn local_endpoint_configuration_is_explicit() -> Result<(), Box<dyn std::error::Error>> {
    assert!(EndpointSettings::select(None, None, None).is_err());
    assert!(EndpointSettings::select(Some("polaris-test"), Some("/tmp/rpc.sock"), None).is_err());
    assert!(EndpointSettings::select(Some("remote\\\\pipe\\\\x"), None, None).is_err());
    assert!(EndpointSettings::select(None, Some("relative.sock"), None).is_err());
    Ok(())
}
