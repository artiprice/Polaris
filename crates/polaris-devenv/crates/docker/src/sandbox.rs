//! A CI sandbox on a local Docker Engine (`/ci-run`, P8): one container per
//! [`SandboxBackend::create`], the workspace bind-mounted read-only, and named volumes for caches
//! that outlive the container (the cargo registry and the target directory), so a second run
//! reuses what the first one built. Hardening follows the one-shot containers — every capability
//! dropped, no new privileges, bounded PIDs, memory and CPU, labelled for cleanup — except that the
//! network is on by default so a cold cache can fetch dependencies (`network: false` turns it off).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{ContainerCreateBody, HostConfig};
use bytes::{Bytes, BytesMut};
use futures::TryStreamExt;
use polaris_sdk::registry::{
    ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec,
};
use uuid::Uuid;

use crate::{Docker, DockerError, ImageRef, MIN_MEMORY_BYTES};

/// Label on every CI sandbox container, so a crashed run's containers can be found and removed.
pub const CI_LABEL: &str = "polaris.ci";
/// Output captured per command when the request names no bound.
pub const DEFAULT_CI_OUTPUT: usize = 4 * 1024 * 1024;
/// Processes allowed in one CI container (cargo and rustc spawn many).
const PIDS_LIMIT: i64 = 4_096;

/// A named Docker volume mounted at `target` in every CI container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheVolume {
    /// Volume name (`[A-Za-z0-9][A-Za-z0-9_.-]*`).
    pub name: String,
    /// Absolute path inside the container.
    pub target: String,
}

/// How CI containers are made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerSandboxConfig {
    /// Host directory mounted read-only at `mount_point`.
    pub workspace: PathBuf,
    /// Absolute container path of the workspace.
    pub mount_point: String,
    /// Cache volumes.
    pub caches: Vec<CacheVolume>,
    /// Image when the sandbox spec names none.
    pub default_image: ImageRef,
    /// Memory ceiling when the spec names none, in bytes.
    pub memory_bytes: u64,
    /// CPU ceiling when the spec names none, in nanocpus.
    pub nano_cpus: u64,
    /// Bridge networking (dependency fetches); `false` runs without a network.
    pub network: bool,
}

/// [`SandboxBackend`] over a local Docker Engine for CI pipelines.
pub struct DockerSandbox {
    docker: Docker,
    config: DockerSandboxConfig,
}

impl std::fmt::Debug for DockerSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerSandbox")
            .field("engine_version", &self.docker.engine_version)
            .field("config", &self.config)
            .finish()
    }
}

fn valid_volume_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && name.len() <= 128
}

fn valid_container_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4_096
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|part| part == "..")
}

impl DockerSandbox {
    /// A sandbox factory on `docker`.
    /// # Errors
    /// An invalid mount point, cache volume, workspace or limit.
    pub fn new(docker: Docker, config: DockerSandboxConfig) -> Result<Self, DockerError> {
        if !valid_container_path(&config.mount_point) {
            return Err(DockerError::InvalidSpec(
                "invalid workspace mount point".to_owned(),
            ));
        }
        if !config.workspace.is_dir() {
            return Err(DockerError::InvalidSpec(
                "workspace is not a directory".to_owned(),
            ));
        }
        let mut targets = vec![config.mount_point.as_str()];
        for cache in &config.caches {
            if !valid_volume_name(&cache.name) || !valid_container_path(&cache.target) {
                return Err(DockerError::InvalidSpec("invalid cache volume".to_owned()));
            }
            if targets.contains(&cache.target.as_str()) {
                return Err(DockerError::InvalidSpec(
                    "duplicate mount target".to_owned(),
                ));
            }
            targets.push(&cache.target);
        }
        if config.memory_bytes < MIN_MEMORY_BYTES
            || i64::try_from(config.memory_bytes).is_err()
            || config.nano_cpus == 0
            || i64::try_from(config.nano_cpus).is_err()
        {
            return Err(DockerError::InvalidSpec(
                "invalid memory or CPU limit".to_owned(),
            ));
        }
        Ok(Self { docker, config })
    }

    /// The configuration.
    #[must_use]
    pub const fn config(&self) -> &DockerSandboxConfig {
        &self.config
    }

    /// The engine version the sandbox talks to.
    #[must_use]
    pub fn engine_version(&self) -> &str {
        &self.docker.engine_version
    }

    /// Whether `image` is present locally (nothing is pulled).
    /// # Errors
    /// The engine is unreachable.
    pub async fn image_present(&self, image: &ImageRef) -> Result<bool, DockerError> {
        match self.docker.client.inspect_image(image.as_str()).await {
            Ok(_) => Ok(true),
            Err(error) if not_found(&error) => Ok(false),
            Err(error) => Err(DockerError::Engine(error)),
        }
    }

    /// Whether the volume `name` exists (a warm cache).
    /// # Errors
    /// The engine is unreachable.
    pub async fn volume_exists(&self, name: &str) -> Result<bool, DockerError> {
        match self.docker.client.inspect_volume(name).await {
            Ok(_) => Ok(true),
            Err(error) if not_found(&error) => Ok(false),
            Err(error) => Err(DockerError::Engine(error)),
        }
    }

    /// Remove the cache volumes (the next run is cold); a missing volume is fine.
    /// # Errors
    /// The engine refuses (for example, a running container still uses the volume).
    pub async fn remove_caches(&self) -> Result<(), DockerError> {
        for cache in &self.config.caches {
            match self
                .docker
                .client
                .remove_volume(
                    &cache.name,
                    None::<bollard::query_parameters::RemoveVolumeOptions>,
                )
                .await
            {
                Ok(()) => {}
                Err(error) if not_found(&error) => {}
                Err(error) => return Err(DockerError::Engine(error)),
            }
        }
        Ok(())
    }

    fn container_body(&self, spec: &SandboxSpec) -> Result<ContainerCreateBody, SandboxError> {
        let image = match spec.image.as_deref() {
            Some(image) => ImageRef::new(image)
                .map_err(|_| SandboxError::PolicyDenied("invalid image reference".into()))?,
            None => self.config.default_image.clone(),
        };
        let memory = spec.memory_mib.map_or(self.config.memory_bytes, |mib| {
            u64::from(mib).saturating_mul(1024 * 1024)
        });
        let cpus = spec.cpu_millis.map_or(self.config.nano_cpus, |millis| {
            u64::from(millis).saturating_mul(1_000_000)
        });
        if memory < MIN_MEMORY_BYTES || cpus == 0 {
            return Err(SandboxError::PolicyDenied(
                "memory or CPU limit too small".into(),
            ));
        }
        let mut binds = vec![format!(
            "{}:{}:ro",
            self.config.workspace.display(),
            self.config.mount_point
        )];
        binds.extend(
            self.config
                .caches
                .iter()
                .map(|cache| format!("{}:{}", cache.name, cache.target)),
        );
        Ok(ContainerCreateBody {
            image: Some(image.as_str().to_owned()),
            cmd: Some(vec!["sleep".to_owned(), "infinity".to_owned()]),
            labels: Some(HashMap::from([(CI_LABEL.to_owned(), "1".to_owned())])),
            network_disabled: Some(!self.config.network),
            host_config: Some(HostConfig {
                binds: Some(binds),
                memory: Some(
                    i64::try_from(memory)
                        .map_err(|_| SandboxError::PolicyDenied("memory limit overflow".into()))?,
                ),
                nano_cpus: Some(
                    i64::try_from(cpus)
                        .map_err(|_| SandboxError::PolicyDenied("CPU limit overflow".into()))?,
                ),
                network_mode: Some(
                    if self.config.network {
                        "bridge"
                    } else {
                        "none"
                    }
                    .to_owned(),
                ),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
                pids_limit: Some(PIDS_LIMIT),
                tmpfs: Some(HashMap::from([(
                    "/tmp".to_owned(),
                    "rw,nosuid,size=1g".to_owned(),
                )])),
                ..HostConfig::default()
            }),
            ..ContainerCreateBody::default()
        })
    }

    async fn remove(&self, name: &str) -> Result<(), DockerError> {
        // Volumes are kept: they are the cache.
        match self
            .docker
            .client
            .remove_container(
                name,
                Some(
                    bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .build(),
                ),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if not_found(&error) => Ok(()),
            Err(error) => Err(DockerError::Engine(error)),
        }
    }

    async fn run(
        &self,
        container: &str,
        request: &ExecRequest,
        limit: usize,
    ) -> Result<ExecOutput, SandboxError> {
        let started = Instant::now();
        let exec = self
            .docker
            .client
            .create_exec(
                container,
                CreateExecOptions::<String> {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(request.argv.clone()),
                    env: Some(
                        request
                            .env
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect(),
                    ),
                    working_dir: request.workdir.clone(),
                    privileged: Some(false),
                    tty: Some(false),
                    ..CreateExecOptions::default()
                },
            )
            .await
            .map_err(engine)?;
        let StartExecResults::Attached { output, .. } = self
            .docker
            .client
            .start_exec(&exec.id, None)
            .await
            .map_err(engine)?
        else {
            return Err(SandboxError::Protocol(
                "Docker detached an attached command".into(),
            ));
        };
        let mut stdout = BytesMut::new();
        let mut stderr = BytesMut::new();
        let mut captured = 0_usize;
        let mut stream = output;
        while let Some(log) = stream.try_next().await.map_err(engine)? {
            let (destination, bytes) = match log {
                LogOutput::StdErr { message } => (&mut stderr, message),
                LogOutput::StdOut { message }
                | LogOutput::Console { message }
                | LogOutput::StdIn { message } => (&mut stdout, message),
            };
            captured = captured.saturating_add(bytes.len());
            if captured > limit {
                return Err(SandboxError::LimitExceeded(format!(
                    "command output exceeds {limit} bytes"
                )));
            }
            destination.extend_from_slice(&bytes);
        }
        let inspected = self
            .docker
            .client
            .inspect_exec(&exec.id)
            .await
            .map_err(engine)?;
        let exit_code = inspected
            .exit_code
            .ok_or_else(|| SandboxError::Protocol("Docker omitted the exit code".into()))?;
        Ok(ExecOutput {
            exit_code: i32::try_from(exit_code).unwrap_or(-1),
            stdout: stdout.freeze(),
            stderr: stderr.freeze(),
            duration: started.elapsed(),
        })
    }
}

fn not_found(error: &bollard::errors::Error) -> bool {
    matches!(
        error,
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

fn engine(error: bollard::errors::Error) -> SandboxError {
    if not_found(&error) {
        SandboxError::NotFound("container".into())
    } else {
        SandboxError::Unavailable(format!("Docker Engine: {error}"))
    }
}

const NO_FILES: &str = "the CI sandbox reads the workspace through its read-only mount";

impl SandboxBackend for DockerSandbox {
    async fn confirm_absence(&self, sandbox: &SandboxHandle) -> Result<bool, SandboxError> {
        match self
            .docker
            .client
            .inspect_container(sandbox.as_str(), None)
            .await
        {
            Ok(_) => Ok(false),
            Err(error) if not_found(&error) => Ok(true),
            Err(error) => Err(engine(error)),
        }
    }

    async fn create(&self, spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let body = self.container_body(spec)?;
        let name = format!("polaris-ci-{}", Uuid::now_v7().simple());
        let handle = SandboxHandle::new(&name)
            .map_err(|_| SandboxError::Protocol("invalid container name".into()))?;
        self.docker
            .client
            .create_container(
                Some(
                    bollard::query_parameters::CreateContainerOptionsBuilder::default()
                        .name(&name)
                        .build(),
                ),
                body,
            )
            .await
            .map_err(engine)?;
        if let Err(primary) = self.docker.client.start_container(&name, None).await {
            let cleanup = self.remove(&name).await;
            return Err(match cleanup {
                Ok(()) => engine(primary),
                Err(cleanup) => SandboxError::Unavailable(format!(
                    "Docker could not start the container ({primary}) nor remove it ({cleanup})"
                )),
            });
        }
        Ok(handle)
    }

    async fn exec(
        &self,
        sandbox: &SandboxHandle,
        request: &ExecRequest,
    ) -> Result<ExecOutput, SandboxError> {
        if request.argv.is_empty() || request.argv.iter().any(|arg| arg.contains('\0')) {
            return Err(SandboxError::PolicyDenied("invalid argv".into()));
        }
        if request.stdin.is_some() {
            return Err(SandboxError::PolicyDenied(
                "the CI sandbox does not take stdin".into(),
            ));
        }
        let limit = match request.output_limit_bytes {
            Some(0) => return Err(SandboxError::PolicyDenied("zero output limit".into())),
            Some(limit) => limit,
            None => DEFAULT_CI_OUTPUT,
        };
        let run = self.run(sandbox.as_str(), request, limit);
        match request.timeout {
            Some(timeout) => tokio::time::timeout(timeout, run)
                .await
                .map_err(|_| SandboxError::Timeout)?,
            None => run.await,
        }
    }

    async fn upload(
        &self,
        _sandbox: &SandboxHandle,
        _path: &str,
        _bytes: Bytes,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::PolicyDenied(NO_FILES.into()))
    }

    async fn download(&self, _sandbox: &SandboxHandle, _path: &str) -> Result<Bytes, SandboxError> {
        Err(SandboxError::PolicyDenied(NO_FILES.into()))
    }

    async fn list_files(
        &self,
        _sandbox: &SandboxHandle,
        _dir: &str,
    ) -> Result<Vec<String>, SandboxError> {
        Err(SandboxError::PolicyDenied(NO_FILES.into()))
    }

    async fn destroy(&self, sandbox: &SandboxHandle) -> Result<(), SandboxError> {
        self.remove(sandbox.as_str())
            .await
            .map_err(|error| SandboxError::Unavailable(error.to_string()))
    }
}

/// Remove CI containers a crashed run left behind (by [`CI_LABEL`]); returns how many.
/// # Errors
/// The engine is unreachable or refuses a removal.
pub async fn remove_orphan_ci_containers(docker: &Docker) -> Result<usize, DockerError> {
    let containers = docker
        .client
        .list_containers(Some(
            bollard::query_parameters::ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&HashMap::from([(
                    "label".to_owned(),
                    vec![format!("{CI_LABEL}=1")],
                )]))
                .build(),
        ))
        .await?;
    let mut removed = 0_usize;
    for container in containers {
        let Some(id) = container.id else { continue };
        docker
            .client
            .remove_container(
                &id,
                Some(
                    bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                        .force(true)
                        .build(),
                ),
            )
            .await?;
        removed = removed.saturating_add(1);
    }
    Ok(removed)
}
