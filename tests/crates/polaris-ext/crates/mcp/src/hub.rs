// Unit tests of crates/polaris-ext/crates/mcp/src/hub.rs (`validation_tests`); compiled into that module via #[path].
use super::*;

fn valid_spec() -> Result<McpServerSpec, McpError> {
    Ok(McpServerSpec {
        name: ServerName::new("fixture")?,
        transport: McpTransport::Stdio {
            command: vec!["fixture".to_owned()],
            cwd: None,
            env: Vec::new(),
        },
        trust: TrustLevel::UserGlobal,
        startup_timeout: Duration::from_secs(30),
        call_timeout: Duration::from_secs(30),
        tool_allowlist: None,
        max_result_bytes: 4096,
    })
}

#[test]
fn transport_limits_enforce_documented_inclusive_bounds() -> Result<(), McpError> {
    let mut spec = valid_spec()?;
    spec.startup_timeout = Duration::from_secs(120);
    spec.call_timeout = Duration::from_secs(120);
    spec.max_result_bytes = MAX_JSONRPC_FRAME_BYTES;
    assert_eq!(validate_spec(&spec), Ok(()));

    spec.startup_timeout = Duration::ZERO;
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));
    spec.startup_timeout = Duration::from_secs(121);
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));

    spec.startup_timeout = Duration::from_secs(30);
    spec.call_timeout = Duration::ZERO;
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));
    spec.call_timeout = Duration::from_secs(121);
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));

    spec.call_timeout = Duration::from_secs(30);
    spec.max_result_bytes = 0;
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));
    spec.max_result_bytes = MAX_JSONRPC_FRAME_BYTES.saturating_add(1);
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));
    Ok(())
}

#[test]
fn plaintext_http_is_limited_to_loopback_even_without_headers() -> Result<(), McpError> {
    let mut spec = valid_spec()?;
    spec.transport = McpTransport::StreamableHttp {
        url: url::Url::parse("http://mcp.example/mcp").map_err(|_| McpError::InvalidSpec)?,
        headers: Vec::new(),
    };
    assert!(matches!(validate_spec(&spec), Err(McpError::InvalidSpec)));

    spec.transport = McpTransport::StreamableHttp {
        url: url::Url::parse("http://127.0.0.1:8080/mcp").map_err(|_| McpError::InvalidSpec)?,
        headers: Vec::new(),
    };
    assert_eq!(validate_spec(&spec), Ok(()));
    Ok(())
}

#[test]
fn loopback_detection_accepts_local_hosts_only() {
    assert!(is_loopback_host(Some("localhost")));
    assert!(is_loopback_host(Some("127.0.0.1")));
    assert!(is_loopback_host(Some("::1")));
    assert!(!is_loopback_host(Some("mcp.example")));
    assert!(!is_loopback_host(None));
}
