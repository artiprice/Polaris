// Unit tests of crates/polaris-openshell/crates/client/src/implementation.rs (`tests`); compiled into that module via #[path].
use super::*;
use polaris_openshell_policy::{AgentRole, ExecTier, policy_for};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn requested_output_limit_counts_both_streams() {
    assert!(check_combined_output(5, 4, 1, Some(10)).is_ok());
    assert!(check_combined_output(5, 4, 2, Some(10)).is_err());
    assert!(check_combined_output(0, 0, 1, Some(0)).is_err());
    assert!(check_combined_output(usize::MAX, 0, 1, None).is_err());
    assert!(check_combined_output(0, usize::MAX, 1, None).is_err());
}

#[test]
fn scratch_quota_is_bounded_and_has_no_host_source() -> Result<(), Box<dyn std::error::Error>> {
    use prost_types::value::Kind;
    for bytes in [0, 1, 1024 * 1024 - 1, 64 * 1024 * 1024 + 1, u32::MAX] {
        assert!(ScratchQuota::new(bytes).is_err());
    }
    for bytes in [1024 * 1024, 64 * 1024 * 1024] {
        let quota = ScratchQuota::new(bytes)?;
        assert_eq!(quota.bytes(), bytes);
        let config = quota.driver_config();
        assert_eq!(config.fields.len(), 1);
        let Some(Kind::StructValue(docker)) = config
            .fields
            .get("docker")
            .and_then(|value| value.kind.as_ref())
        else {
            return Err("missing Docker configuration".into());
        };
        assert_eq!(docker.fields.len(), 1);
        let Some(Kind::ListValue(mounts)) = docker
            .fields
            .get("mounts")
            .and_then(|value| value.kind.as_ref())
        else {
            return Err("missing mount list".into());
        };
        assert_eq!(mounts.values.len(), 1);
        let Some(Kind::StructValue(mount)) =
            mounts.values.first().and_then(|value| value.kind.as_ref())
        else {
            return Err("missing mount object".into());
        };
        assert_eq!(mount.fields.len(), 4);
        assert!(!mount.fields.contains_key("source"));
        assert_eq!(
            mount
                .fields
                .get("target")
                .and_then(|value| value.kind.as_ref()),
            Some(&Kind::StringValue(ScratchQuota::PATH.to_owned()))
        );
        assert_eq!(
            mount
                .fields
                .get("type")
                .and_then(|value| value.kind.as_ref()),
            Some(&Kind::StringValue("tmpfs".to_owned()))
        );
        assert_eq!(
            mount
                .fields
                .get("size_bytes")
                .and_then(|value| value.kind.as_ref()),
            Some(&Kind::NumberValue(f64::from(bytes)))
        );
    }
    Ok(())
}

#[test]
fn quota_limits_use_the_gateway_struct_shape_and_units() -> Result<(), Box<dyn std::error::Error>> {
    let resources = sandbox_resource_limits(Some(1_000), Some(512))
        .ok_or("requested limits should be present")?;
    let Some(prost_types::value::Kind::StructValue(limits)) = resources
        .fields
        .get("limits")
        .and_then(|value| value.kind.as_ref())
    else {
        return Err("limits object should be present".into());
    };
    assert_eq!(
        limits
            .fields
            .get("cpu")
            .and_then(|value| value.kind.as_ref()),
        Some(&prost_types::value::Kind::StringValue("1000m".to_owned()))
    );
    assert_eq!(
        limits
            .fields
            .get("memory")
            .and_then(|value| value.kind.as_ref()),
        Some(&prost_types::value::Kind::StringValue("512Mi".to_owned()))
    );
    assert!(sandbox_resource_limits(None, None).is_none());
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectTestError {
    Transient,
    Permanent,
}

#[test]
fn catalog_resolves_exact_content_and_missing_cids_fail() -> Result<(), Box<dyn std::error::Error>>
{
    let catalog = PolicyCatalog::default();
    let policy = policy_for(AgentRole::Builder, ExecTier::Docker)
        .map_err(|error| SandboxError::PolicyDenied(error.to_string()))?;
    let cid = catalog.register(&policy)?;
    let bytes = catalog.resolve(&cid)?;
    assert_eq!(Cid::from_bytes(*blake3::hash(&bytes).as_bytes()), cid);
    assert!(catalog.resolve(&Cid::from_bytes([7; 32])).is_err());
    Ok(())
}

#[tokio::test]
async fn connection_retry_is_bounded_and_skips_permanent_errors() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempt_counter = Arc::clone(&attempts);
    let result = retry_transient_connect(
        Duration::from_millis(350),
        move || {
            attempt_counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(ConnectTestError::Transient) }
        },
        |error| *error == ConnectTestError::Transient,
    )
    .await;
    assert!(matches!(result, Err(ConnectRetryError::Deadline)));
    assert!(attempts.load(Ordering::SeqCst) >= 2);

    let permanent_attempts = Arc::new(AtomicUsize::new(0));
    let attempt_counter = Arc::clone(&permanent_attempts);
    let result = retry_transient_connect(
        Duration::from_secs(1),
        move || {
            attempt_counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(ConnectTestError::Permanent) }
        },
        |error| *error == ConnectTestError::Transient,
    )
    .await;
    assert!(matches!(
        result,
        Err(ConnectRetryError::Failed(ConnectTestError::Permanent))
    ));
    assert_eq!(permanent_attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_connection_retry_cancels_the_inflight_attempt()
-> Result<(), Box<dyn std::error::Error>> {
    struct DropMarker(Arc<AtomicBool>);
    impl Drop for DropMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let marker = Arc::clone(&dropped);
    let mut retry = Box::pin(retry_transient_connect(
        Duration::from_secs(5),
        move || {
            let marker = Arc::clone(&marker);
            async move {
                let _drop_marker = DropMarker(marker);
                tokio::time::sleep(Duration::from_secs(30)).await;
                Err::<(), _>(ConnectTestError::Transient)
            }
        },
        |_| true,
    ));
    tokio::select! {
        _ = &mut retry => return Err("in-flight connection attempt unexpectedly completed".into()),
        _ = tokio::time::sleep(Duration::from_millis(10)) => {}
    }
    drop(retry);
    assert!(dropped.load(Ordering::SeqCst));
    Ok(())
}

#[test]
fn only_refused_or_disconnected_sockets_are_retryable() {
    let refused = std::io::Error::new(ErrorKind::ConnectionRefused, "gateway starting");
    let disconnected = std::io::Error::new(ErrorKind::NotConnected, "gateway disconnected");
    let invalid_certificate = std::io::Error::new(ErrorKind::InvalidData, "invalid certificate");
    assert!(is_transient_connect_error(&refused));
    assert!(is_transient_connect_error(&disconnected));
    assert!(!is_transient_connect_error(&invalid_certificate));
}

#[test]
fn duplicate_sandbox_error_is_typed_for_cleanup_ownership() {
    assert_eq!(
        map_create_error(tonic::Status::already_exists("sandbox name already exists")),
        SandboxError::AlreadyExists("sandbox name already exists".to_owned())
    );
    assert!(matches!(
        map_create_error(tonic::Status::permission_denied("denied")),
        SandboxError::Unavailable(_)
    ));
}

#[test]
fn gateway_driver_identity_must_match_the_selected_backend() {
    let docker = GatewayDriverInfo {
        configured_name: "default".to_owned(),
        driver_name: "docker".to_owned(),
        driver_version: "test".to_owned(),
    };
    assert!(validate_gateway_driver_kind(&docker, DriverKind::Docker).is_ok());
    assert!(validate_gateway_driver_kind(&docker, DriverKind::Vm).is_err());
}

#[test]
fn recovery_domain_normalizes_and_binds_the_trusted_backend_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let config = OpenShellConfig::wsl2(GatewayTls {
        ca_certificate: b"ca-one".to_vec(),
        client_certificate: Vec::new(),
        client_key: Vec::new(),
        server_name: "gateway.local".to_owned(),
    });
    let driver = GatewayDriverInfo {
        configured_name: "default".to_owned(),
        driver_name: "docker".to_owned(),
        driver_version: "1".to_owned(),
    };
    let domain = openshell_recovery_domain(&config, &driver)
        .ok_or("valid TLS endpoint must have a recovery domain")?;
    let mut normalized = config.clone();
    normalized.endpoint = "https://user:secret@LOCALHOST:17670/?token=ignored#fragment".to_owned();
    assert_eq!(
        openshell_recovery_domain(&normalized, &driver),
        Some(domain)
    );

    let mut implicit_https_port = config.clone();
    implicit_https_port.endpoint = "https://localhost/".to_owned();
    let mut explicit_https_port = implicit_https_port.clone();
    explicit_https_port.endpoint = "HTTPS://user:secret@LOCALHOST:443".to_owned();
    let implicit_port_domain = openshell_recovery_domain(&implicit_https_port, &driver)
        .ok_or("valid HTTPS endpoint must have a recovery domain")?;
    assert_eq!(
        openshell_recovery_domain(&explicit_https_port, &driver),
        Some(implicit_port_domain)
    );
    assert_eq!(implicit_port_domain.to_hex().len(), 64);
    assert!(!implicit_port_domain.to_hex().contains("secret"));

    let mut other_endpoint = config.clone();
    other_endpoint.endpoint = "https://localhost:17671".to_owned();
    assert_ne!(
        openshell_recovery_domain(&other_endpoint, &driver),
        Some(domain)
    );
    let mut other_ca = config.clone();
    other_ca.tls.ca_certificate = b"ca-two".to_vec();
    assert_ne!(openshell_recovery_domain(&other_ca, &driver), Some(domain));
    let mut other_server_name = config.clone();
    other_server_name.tls.server_name = "other.local".to_owned();
    assert_ne!(
        openshell_recovery_domain(&other_server_name, &driver),
        Some(domain)
    );
    let mut other_driver = driver.clone();
    other_driver.driver_version = "2".to_owned();
    assert_ne!(
        openshell_recovery_domain(&config, &other_driver),
        Some(domain)
    );
    let mut insecure = config;
    insecure.endpoint = "http://localhost:17670".to_owned();
    assert!(openshell_recovery_domain(&insecure, &driver).is_none());
    Ok(())
}

#[test]
fn delete_acknowledgements_require_a_terminal_or_pollable_target() {
    let original_id = "sandbox-id".to_owned();
    assert_eq!(
        validate_delete_ack(openshell_sdk::DeletionResult {
            outcome: DeletionOutcome::Accepted,
            sandbox_id: Some(original_id.clone()),
        })
        .ok(),
        Some(Some(original_id.clone()))
    );
    assert_eq!(
        validate_delete_ack(openshell_sdk::DeletionResult {
            outcome: DeletionOutcome::AlreadyAbsent,
            sandbox_id: None,
        })
        .ok(),
        Some(None)
    );
    assert!(
        validate_delete_ack(openshell_sdk::DeletionResult {
            outcome: DeletionOutcome::Accepted,
            sandbox_id: None,
        })
        .is_err()
    );
    assert!(
        validate_delete_ack(openshell_sdk::DeletionResult {
            outcome: DeletionOutcome::Unspecified,
            sandbox_id: Some(original_id),
        })
        .is_err()
    );
}

#[test]
fn request_and_output_bounds_fail_closed() {
    assert!(extend_bounded(&mut vec![0; MAX_EXEC_OUTPUT_BYTES], &[1]).is_err());
    assert!(validate_path("/sandbox/../etc/passwd").is_err());
    assert!(validate_path("relative").is_err());
    assert!(validate_path("/sandbox/file").is_ok());
    const { assert!(MAX_REQUEST_CHUNK_BYTES < 1_048_576) };
}

#[test]
fn file_listing_checks_each_path_before_allocation() {
    assert_eq!(
        parse_file_list("/sandbox/a\n/sandbox/b\n").ok(),
        Some(vec!["/sandbox/a".to_owned(), "/sandbox/b".to_owned()])
    );
    assert!(parse_file_list("relative\n").is_err());
    let long_path = format!("/{}", "x".repeat(MAX_LIST_PATH_BYTES));
    assert!(parse_file_list(&long_path).is_err());
}

#[test]
fn sync_snapshot_response_is_bounded_and_validated() -> Result<(), Box<dyn std::error::Error>> {
    let page = SandboxSnapshotPage {
        generation: "a".repeat(64),
        manifest_id: Cid::from_bytes([3; 32]),
        total_entries: 2,
        offset: 0,
        entries: vec![polaris_sdk::registry::SandboxSnapshotEntry {
            relative_path: "src/main.rs".to_owned(),
            size: 7,
            hash: Cid::from_bytes(*blake3::hash(b"payload").as_bytes()),
            executable: false,
            link_target: None,
        }],
        next_offset: Some(1),
    };
    let encoded = serde_json::to_vec(&page)?;
    assert!(parse_snapshot_page(&encoded, None, 0, 1).is_ok());
    assert!(parse_snapshot_page(&encoded, Some(&"b".repeat(64)), 0, 1).is_err());
    assert!(parse_snapshot_page(&encoded, None, 0, 0).is_err());
    assert!(parse_snapshot_page(&vec![b' '; MAX_EXEC_OUTPUT_BYTES + 1], None, 0, 1).is_err());
    Ok(())
}

#[test]
fn sync_snapshot_rejects_unsafe_paths_and_link_hashes() {
    let invalid_path = polaris_sdk::registry::SandboxSnapshotEntry {
        relative_path: "../outside".to_owned(),
        size: 0,
        hash: Cid::from_bytes([0; 32]),
        executable: false,
        link_target: None,
    };
    assert!(validate_snapshot_entry(&invalid_path).is_err());

    let escaping_link = polaris_sdk::registry::SandboxSnapshotEntry {
        relative_path: "src/link".to_owned(),
        size: 9,
        hash: Cid::from_bytes(*blake3::hash(b"../../out").as_bytes()),
        executable: false,
        link_target: Some("../../out".to_owned()),
    };
    assert!(validate_snapshot_entry(&escaping_link).is_err());

    let wrong_hash = polaris_sdk::registry::SandboxSnapshotEntry {
        relative_path: "src/link".to_owned(),
        size: 6,
        hash: Cid::from_bytes([7; 32]),
        executable: false,
        link_target: Some("target".to_owned()),
    };
    assert!(validate_snapshot_entry(&wrong_hash).is_err());
}

#[test]
fn sync_commit_request_is_bounded_before_exec() {
    let request = SandboxCommitRequest {
        root: "/workspace".to_owned(),
        relative_path: "src/main.rs".to_owned(),
        expected: None,
        action: polaris_sdk::registry::SandboxCommitAction::Delete,
    };
    assert!(encode_commit_request(&request).is_ok());
    let oversized = SandboxCommitRequest {
        relative_path: "a".repeat(MAX_LIST_PATH_BYTES + 1),
        ..request
    };
    assert!(matches!(
        encode_commit_request(&oversized),
        Err(SandboxError::LimitExceeded(_))
    ));
}

#[test]
fn gateway_names_fail_before_remote_create_when_invalid() {
    assert!(validate_gateway_name("p2-0123456789abcdef").is_ok());
    assert!(validate_gateway_name("p2-0123456789abcdef0").is_err());
    assert!(validate_gateway_name("bad/name").is_err());
}

#[test]
fn image_references_require_immutable_sha256_digests() {
    assert!(validate_image_reference(&format!("sha256:{}", "a".repeat(64))).is_ok());
    assert!(
        validate_image_reference(&format!(
            "nvcr.io/polaris/sandbox@sha256:{}",
            "b".repeat(64)
        ))
        .is_ok()
    );
    assert!(validate_image_reference("polaris-sandbox:latest").is_err());
    assert!(validate_image_reference("sha256:abc").is_err());
}
