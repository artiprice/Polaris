// Unit tests of crates/polaris-devenv/crates/docker/src/lib.rs (`tests`); compiled into that module via #[path].
use super::{
    BuildCacheMount, BuildContext, BuildOptions, ContainerSpec, DEFAULT_OUTPUT_LIMIT,
    DockerEndpoint, DockerError, ExecRequest, ImageRef, MIN_MEMORY_BYTES, WarmPoolConfig,
};
use bytes::Bytes;
use std::time::Duration;
#[cfg(target_os = "linux")]
use uuid::Uuid;

#[test]
fn hardened_spec_requires_memory_and_cpu_limits() {
    let image = ImageRef::new("alpine:3.21");
    assert!(image.is_ok());
    if let Ok(image) = image {
        assert!(matches!(
            ContainerSpec::hardened(
                image,
                vec!["sh".to_owned()],
                Vec::new(),
                MIN_MEMORY_BYTES - 1,
                1_000_000_000
            ),
            Err(DockerError::InvalidSpec(_))
        ));
    }
}

#[test]
fn hardened_spec_rejects_secret_environment_entries() {
    let image = ImageRef::new("alpine:3.21");
    assert!(image.is_ok());
    if let Ok(image) = image {
        assert!(matches!(
            ContainerSpec::hardened(
                image,
                vec!["sh".to_owned()],
                vec![("API_TOKEN".to_owned(), "credential".to_owned())],
                MIN_MEMORY_BYTES,
                1_000_000_000
            ),
            Err(DockerError::InvalidSpec(_))
        ));
    }
}

#[test]
fn exec_request_rejects_empty_argv_before_engine_use() {
    let request = ExecRequest {
        argv: Vec::new(),
        env: Vec::new(),
        timeout: Duration::from_secs(1),
        output_limit: DEFAULT_OUTPUT_LIMIT,
    };
    assert!(super::validate_argv(&request.argv).is_err());
}

#[test]
fn endpoint_selection_is_platform_specific() {
    let endpoint = DockerEndpoint::local();
    #[cfg(windows)]
    assert!(matches!(endpoint, DockerEndpoint::NamedPipe(_)));
    #[cfg(unix)]
    assert!(matches!(endpoint, DockerEndpoint::UnixSocket(_)));
}

#[test]
fn build_context_requires_root_dockerfile_and_rejects_traversal() {
    let valid = archive_with_file("Dockerfile");
    assert!(valid.is_ok());
    if let Ok(valid) = valid {
        assert!(BuildContext::from_tar(valid, 4096).is_ok());
    }
    let symlink = archive_with_symlink();
    assert!(symlink.is_ok());
    if let Ok(symlink) = symlink {
        assert!(matches!(
            BuildContext::from_tar(symlink, 4096),
            Err(DockerError::InvalidSpec(_))
        ));
    }
}

#[test]
fn buildkit_cache_mounts_require_supported_local_frontend_and_exact_run_mount() {
    let dockerfile = "# syntax=docker/dockerfile:1.7\nFROM alpine:3.21\nrun --mount=type=cache,target=/root/.cache/pip,id=pip-cache pip install -r requirements.txt\n";
    let context = archive_with_contents("Dockerfile", dockerfile.as_bytes());
    assert!(context.is_ok());
    if let Ok(bytes) = context {
        let context = BuildContext::from_tar(bytes, 4096);
        assert!(context.is_ok());
        if let Ok(context) = context {
            let options = BuildOptions {
                cache_mounts: vec![BuildCacheMount {
                    id: "pip-cache".to_owned(),
                    target: "/root/.cache/pip".to_owned(),
                }],
            };
            assert!(super::validate_build_options(&context, &options).is_ok());
            assert_eq!(
                super::dockerfile_frontend(&context.dockerfile)
                    .ok()
                    .flatten(),
                Some("docker/dockerfile:1.7".to_owned())
            );
        }
    }
}

#[test]
fn buildkit_preflight_checks_external_copy_images_are_preloaded() {
    let references = super::dockerfile_image_references(
        "FROM alpine:3.21 AS build\nCOPY --from=build /out /out\nCOPY --from=registry.example/tools:2 /tool /tool\n",
    );
    assert!(
        matches!(references, Ok(images) if images == vec!["alpine:3.21", "registry.example/tools:2"])
    );
    assert!(
        super::dockerfile_image_references("FROM alpine:3.21\nCOPY --from=$STAGE /out /out\n")
            .is_err()
    );
}

#[test]
fn bounded_build_rejects_missing_mount_and_remote_sources() {
    let dockerfile = "FROM alpine:3.21\nADD https://example.invalid/source /tmp/source\n";
    let archive = archive_with_contents("Dockerfile", dockerfile.as_bytes());
    assert!(archive.is_ok());
    if let Ok(archive) = archive {
        let context = BuildContext::from_tar(archive, 4096);
        assert!(context.is_ok());
        if let Ok(context) = context {
            assert!(matches!(
                super::validate_build_options(
                    &context,
                    &BuildOptions {
                        cache_mounts: vec![BuildCacheMount {
                            id: "missing".to_owned(),
                            target: "/root/.cache/pip".to_owned(),
                        }],
                    }
                ),
                Err(DockerError::InvalidSpec(_))
            ));
            assert!(matches!(
                super::validate_build_options(&context, &BuildOptions::default()),
                Err(DockerError::InvalidSpec(_))
            ));
        }
    }
}

#[test]
fn warm_pool_budget_is_enforced_before_engine_calls() {
    let config = WarmPoolConfig::new(2, MIN_MEMORY_BYTES * 2);
    assert!(config.is_ok());
    let too_small_budget = WarmPoolConfig::new(2, MIN_MEMORY_BYTES);
    assert!(too_small_budget.is_ok());
    if let (Ok(config), Ok(too_small_budget), Ok(image)) =
        (config, too_small_budget, ImageRef::new("alpine:3.21"))
    {
        let spec = ContainerSpec::hardened(
            image,
            vec!["sleep".to_owned(), "infinity".to_owned()],
            Vec::new(),
            MIN_MEMORY_BYTES,
            1_000_000_000,
        );
        assert!(matches!(
            spec.as_ref().map(|spec| config.validate(spec)),
            Ok(Ok(()))
        ));
        assert!(matches!(
            spec.as_ref().map(|spec| too_small_budget.validate(spec)),
            Ok(Err(DockerError::InvalidSpec(_)))
        ));
    }
    let too_large = WarmPoolConfig::new(17, MIN_MEMORY_BYTES * 17);
    assert!(matches!(too_large, Err(DockerError::InvalidSpec(_))));
}

#[test]
fn orphan_ownership_requires_both_exact_ids_and_managed_label() {
    let pool_id = uuid::Uuid::now_v7();
    let owner_id = uuid::Uuid::now_v7();
    let labels = super::pool_labels(pool_id, owner_id);
    assert!(super::owns_pool_labels(&labels, pool_id, owner_id));
    assert!(!super::owns_pool_labels(
        &labels,
        uuid::Uuid::now_v7(),
        owner_id
    ));
    assert!(!super::owns_pool_labels(
        &labels,
        pool_id,
        uuid::Uuid::now_v7()
    ));
    let mut unmarked = labels;
    unmarked.remove("io.polaris.devenv.managed");
    assert!(!super::owns_pool_labels(&unmarked, pool_id, owner_id));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn live_owned_exec_pool_failure_and_persisted_identity_recovery_are_confined() {
    if std::env::var("POLARIS_DEVD_DOCKER_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let docker = super::Docker::connect(&DockerEndpoint::UnixSocket(
        "unix:///var/run/docker.sock".to_owned(),
    ))
    .await;
    assert!(
        docker.is_ok(),
        "authorized WSL Docker daemon must be available"
    );
    let Ok(docker) = docker else { return };
    let image =
        ImageRef::new("sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697");
    assert!(image.is_ok());
    let Ok(image) = image else { return };
    let spec = ContainerSpec::hardened(
        image,
        vec!["infinity".to_owned()],
        Vec::new(),
        MIN_MEMORY_BYTES,
        1_000_000_000,
    );
    assert!(spec.is_ok());
    let Ok(spec) = spec else { return };
    let config = WarmPoolConfig::new(1, MIN_MEMORY_BYTES);
    assert!(config.is_ok());
    let Ok(config) = config else { return };
    let journal = std::env::temp_dir().join(format!("polaris-devenv-test-{}.txt", config.owner_id));
    let serialized = format!(
        "{}\n{}\n{}\n{}\n",
        config.pool_id, config.owner_id, config.target_size, config.memory_budget_bytes
    );
    let persisted = std::fs::write(&journal, serialized).is_ok();
    assert!(
        persisted,
        "persist exact owner IDs before Engine resource creation"
    );
    if !persisted {
        return;
    }
    let (pool_id, owner_id, target_size, memory_budget_bytes) =
        match std::fs::read_to_string(&journal) {
            Ok(contents) => {
                let mut parts = contents.lines();
                let pool = parts.next().and_then(|value| Uuid::parse_str(value).ok());
                let owner = parts.next().and_then(|value| Uuid::parse_str(value).ok());
                let size = parts.next().and_then(|value| value.parse::<usize>().ok());
                let memory = parts.next().and_then(|value| value.parse::<u64>().ok());
                (pool, owner, size, memory)
            }
            Err(_) => (None, None, None, None),
        };
    let restored = match (pool_id, owner_id, target_size, memory_budget_bytes) {
        (Some(pool_id), Some(owner_id), Some(size), Some(memory)) => {
            WarmPoolConfig::from_ids(pool_id, owner_id, size, memory).ok()
        }
        _ => None,
    };
    assert!(
        restored.is_some(),
        "owner identity journal must be restorable"
    );

    let one_shot = docker
        .exec_once_owned(
            &spec,
            &ExecRequest {
                argv: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf polaris-one-shot".to_owned(),
                ],
                env: Vec::new(),
                timeout: Duration::from_secs(10),
                output_limit: DEFAULT_OUTPUT_LIMIT,
            },
            &config,
        )
        .await;
    let one_shot_ok = matches!(one_shot, Ok(output) if output.exit_code == 0 && output.stdout.as_ref() == b"polaris-one-shot");

    let mut pool = docker
        .create_warm_pool(spec.clone(), config.clone())
        .await
        .ok();
    let first_container_id = pool.as_ref().and_then(|pool| pool.ready.front().cloned());
    let first_run = match pool.as_mut() {
        Some(pool) => {
            pool.execute(&ExecRequest {
                argv: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf polaris-pool".to_owned(),
                ],
                env: Vec::new(),
                timeout: Duration::from_secs(10),
                output_limit: DEFAULT_OUTPUT_LIMIT,
            })
            .await
        }
        None => Err(DockerError::InvalidSpec("pool creation failed".to_owned())),
    };
    let first_run_ok = matches!(first_run, Ok(output) if output.exit_code == 0 && output.stdout.as_ref() == b"polaris-pool");
    let first_removed = match first_container_id {
        Some(id) => docker.client.inspect_container(&id, None).await.is_err(),
        None => false,
    };
    let timeout_id = pool.as_ref().and_then(|pool| pool.ready.front().cloned());
    let timeout_result = match pool.as_mut() {
        Some(pool) => {
            pool.execute(&ExecRequest {
                argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), "sleep 30".to_owned()],
                env: Vec::new(),
                timeout: Duration::from_millis(250),
                output_limit: DEFAULT_OUTPUT_LIMIT,
            })
            .await
        }
        None => Err(DockerError::InvalidSpec("pool creation failed".to_owned())),
    };
    let timed_out = matches!(timeout_result, Err(DockerError::Timeout));
    let timeout_container_removed = match timeout_id {
        Some(id) => docker.client.inspect_container(&id, None).await.is_err(),
        None => false,
    };

    drop(pool);
    // Simulate a process exit with one ready labeled resource and restore only its persisted
    // identity in the cleanup path below.
    let orphan_pool = docker.create_warm_pool(spec, config.clone()).await.ok();
    let crash_orphan_id = orphan_pool
        .as_ref()
        .and_then(|pool| pool.ready.front().cloned());
    drop(orphan_pool);
    let recovered_orphans = match restored.as_ref() {
        Some(owner) => docker
            .recover_pool_orphans(owner.pool_id, owner.owner_id)
            .await
            .ok(),
        None => None,
    };
    let crash_orphan_removed = match crash_orphan_id {
        Some(id) => docker.client.inspect_container(&id, None).await.is_err(),
        None => false,
    };
    let residual = match restored.as_ref() {
        Some(owner) => owned_container_ids(&docker, owner).await.ok(),
        None => None,
    };
    let _ = std::fs::remove_file(journal);
    assert!(
        one_shot_ok,
        "owned one-shot command must capture output and remove its container"
    );
    assert!(first_run_ok, "warm pool command must capture output");
    assert!(
        first_removed,
        "used warm container must be absent after its one use"
    );
    assert!(
        timed_out,
        "timed command must report its configured deadline"
    );
    assert!(
        timeout_container_removed,
        "timed-out warm container must be absent after cleanup"
    );
    assert!(
        matches!(recovered_orphans, Some(1)),
        "durable owner IDs must recover exactly the one simulated crash orphan"
    );
    assert!(
        crash_orphan_removed,
        "orphan recovery must remove only the persisted owner's container"
    );
    assert!(
        matches!(residual, Some(ids) if ids.is_empty()),
        "no owned containers may remain"
    );
}

/// Opt-in live BuildKit contract. Registry provenance was verified before fetching the test
/// frontend: Docker Hub's official docker/dockerfile:1.7 index is
/// sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e, and the
/// linux/amd64 manifest fetched for this WSL daemon is
/// sha256:b5f3b260a9678e1d83d2fce86eeddf79420b79147eaba2a25986f47133d73720.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn live_buildkit_cache_mount_uses_local_frontend_and_base_only() {
    if std::env::var("POLARIS_DEVD_BUILDKIT_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    const BASE_IMAGE: &str = "polaris-sandbox:p2-final";
    const FRONTEND: &str = "docker/dockerfile:1.7@sha256:b5f3b260a9678e1d83d2fce86eeddf79420b79147eaba2a25986f47133d73720";
    let docker = super::Docker::connect(&DockerEndpoint::UnixSocket(
        "unix:///var/run/docker.sock".to_owned(),
    ))
    .await;
    assert!(
        docker.is_ok(),
        "authorized WSL Docker daemon must be available"
    );
    let Ok(docker) = docker else { return };
    let base = docker.client.inspect_image(BASE_IMAGE).await;
    assert!(
        matches!(
            base.as_ref().ok().and_then(|image| image.id.as_deref()),
            Some("sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697")
        ),
        "the local test tag must resolve to the pre-approved immutable base image"
    );
    assert!(docker.client.inspect_image(FRONTEND).await.is_ok());

    let request_config = WarmPoolConfig::new(1, MIN_MEMORY_BYTES);
    assert!(
        request_config.is_ok(),
        "fixed one-container budget is valid"
    );
    let Ok(request_config) = request_config else {
        return;
    };
    let identity_spec = ContainerSpec::hardened(
        ImageRef::new(BASE_IMAGE).expect("pinned base image reference is valid"),
        vec!["infinity".to_owned()],
        Vec::new(),
        MIN_MEMORY_BYTES,
        1_000_000_000,
    );
    assert!(identity_spec.is_ok(), "identity probe spec must validate");
    let Ok(identity_spec) = identity_spec else {
        return;
    };
    let identity_output = docker
        .exec_once_owned(
            &identity_spec,
            &ExecRequest {
                argv: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf '%s\\n' \"$(id -u)\" \"$(id -g)\"".to_owned(),
                ],
                env: Vec::new(),
                timeout: Duration::from_secs(10),
                output_limit: DEFAULT_OUTPUT_LIMIT,
            },
            &request_config,
        )
        .await;
    let cache_owner = identity_output.ok().and_then(|output| {
        if output.exit_code != 0 {
            return None;
        }
        let text = std::str::from_utf8(&output.stdout).ok()?;
        let mut ids = text.lines().map(str::parse::<u32>);
        Some((ids.next()?.ok()?, ids.next()?.ok()?))
    });
    assert!(
        cache_owner.is_some(),
        "sandbox image must report numeric uid and gid for cache ownership"
    );
    let Some((cache_uid, cache_gid)) = cache_owner else {
        return;
    };

    let cache_id = format!("polaris-buildkit-{}", Uuid::now_v7());
    let tag = ImageRef::new(format!("polaris-devenv-buildkit-test:{cache_id}"));
    assert!(tag.is_ok());
    let Ok(tag) = tag else { return };
    let dockerfile = format!(
        "# syntax={FRONTEND}\nFROM {BASE_IMAGE}\nRUN --mount=type=cache,target=/tmp/polaris-build-cache,id={cache_id},uid={cache_uid},gid={cache_gid},mode=0777,sharing=locked printf cache-hit > /tmp/polaris-build-cache/sentinel\nRUN --mount=type=cache,target=/tmp/polaris-build-cache,id={cache_id},uid={cache_uid},gid={cache_gid},mode=0777,sharing=locked test \"$(cat /tmp/polaris-build-cache/sentinel)\" = cache-hit\n"
    );
    let archive = archive_with_contents("Dockerfile", dockerfile.as_bytes());
    assert!(archive.is_ok());
    let context = archive
        .ok()
        .and_then(|archive| BuildContext::from_tar(archive, 64 * 1024).ok());
    assert!(
        context.is_some(),
        "bounded Docker build context must validate"
    );
    let Some(context) = context else { return };
    let options = BuildOptions {
        cache_mounts: vec![BuildCacheMount {
            id: cache_id,
            target: "/tmp/polaris-build-cache".to_owned(),
        }],
    };
    let build = docker.build_image(&context, &tag, &options).await;
    let build_succeeded = build.is_ok();
    let build_detail = build.as_ref().err().map(ToString::to_string);
    let built_id = build.as_ref().ok().cloned();
    let execution = match build {
        Ok(_) => {
            let spec = ContainerSpec::hardened(
                tag.clone(),
                vec!["infinity".to_owned()],
                Vec::new(),
                MIN_MEMORY_BYTES,
                1_000_000_000,
            );
            match spec {
                Ok(spec) => {
                    docker
                        .exec_once_owned(
                            &spec,
                            &ExecRequest {
                                argv: vec![
                                    "/bin/sh".to_owned(),
                                    "-c".to_owned(),
                                    "printf buildkit-ok".to_owned(),
                                ],
                                env: Vec::new(),
                                timeout: Duration::from_secs(10),
                                output_limit: DEFAULT_OUTPUT_LIMIT,
                            },
                            &request_config,
                        )
                        .await
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    let cleanup = if docker.client.inspect_image(tag.as_str()).await.is_ok() {
        docker
            .client
            .remove_image(
                tag.as_str(),
                Some(
                    bollard::query_parameters::RemoveImageOptionsBuilder::default()
                        .force(false)
                        .noprune(false)
                        .build(),
                ),
                None,
            )
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    let tag_absent = docker.client.inspect_image(tag.as_str()).await.is_err();

    assert!(
        build_succeeded,
        "BuildKit must complete the local cache-mount build for uid={cache_uid}, gid={cache_gid}: {build_detail:?}"
    );
    assert!(
        matches!(
            &execution,
            Ok(output) if output.exit_code == 0 && output.stdout.as_ref() == b"buildkit-ok"
        ),
        "the built image must run the bounded one-shot command: {execution:?}"
    );
    assert!(built_id.is_some_and(|id| id.starts_with("sha256:")));
    assert!(cleanup.is_ok(), "remove only the unique test image tag");
    assert!(
        tag_absent,
        "unique BuildKit test image tag must be absent after cleanup"
    );
}

#[cfg(target_os = "linux")]
async fn owned_container_ids(
    docker: &super::Docker,
    config: &WarmPoolConfig,
) -> Result<Vec<String>, bollard::errors::Error> {
    let labels = super::pool_labels(config.pool_id, config.owner_id);
    let filters = std::collections::HashMap::from([(
        "label".to_owned(),
        labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
    )]);
    let options = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    docker
        .client
        .list_containers(Some(options))
        .await
        .map(|containers| {
            containers
                .into_iter()
                .filter_map(|container| container.id)
                .collect()
        })
}

fn archive_with_file(path: &str) -> std::io::Result<Bytes> {
    archive_with_contents(path, b"x")
}

fn archive_with_contents(path: &str, contents: &[u8]) -> std::io::Result<Bytes> {
    let mut bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, std::io::Cursor::new(contents))?;
        archive.finish()?;
    }
    Ok(Bytes::from(bytes))
}

fn archive_with_symlink() -> std::io::Result<Bytes> {
    let mut bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut bytes);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../outside")?;
        header.set_cksum();
        archive.append_data(&mut header, "Dockerfile", std::io::Cursor::new(b""))?;
        archive.finish()?;
    }
    Ok(Bytes::from(bytes))
}
