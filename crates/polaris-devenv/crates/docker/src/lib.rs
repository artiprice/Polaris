//! Docker Engine adapter with hardened one-shot container execution.
//!
//! Container creation requires explicit image, memory, CPU, and argv values. Each execution owns
//! and removes its container; pause/unpause is never treated as a filesystem reset.
mod journal;

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration;

use bollard::Docker as BollardClient;
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{ContainerCreateBody, HostConfig};
use bytes::{Bytes, BytesMut};
use futures::TryStreamExt;
use thiserror::Error;
use uuid::Uuid;

pub use journal::{
    PoolCleanup, PoolCleanupError, PoolIdentity, PoolJournal, PoolJournalError, PoolOrphan,
    PoolRegistration,
};

/// Minimum memory limit accepted for a one-shot container.
pub const MIN_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
/// Default maximum combined stdout and stderr captured from one command.
pub const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;
/// Maximum entries accepted in one Docker build archive.
pub const MAX_BUILD_ENTRIES: usize = 10_000;

/// A Docker daemon endpoint, selected explicitly by the composition root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerEndpoint {
    /// Windows Docker Desktop named pipe, for example `npipe:////./pipe/docker_engine`.
    NamedPipe(String),
    /// Unix socket, for example `unix:///var/run/docker.sock`.
    UnixSocket(String),
}

impl DockerEndpoint {
    /// Return the conventional local endpoint for the current operating system.
    #[must_use]
    pub fn local() -> Self {
        #[cfg(windows)]
        {
            Self::NamedPipe("npipe:////./pipe/docker_engine".to_owned())
        }
        #[cfg(unix)]
        {
            Self::UnixSocket("unix:///var/run/docker.sock".to_owned())
        }
    }
}

/// Image reference validated for use in a Docker API request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef(String);

impl ImageRef {
    /// Construct an image reference without shell syntax or control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, DockerError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.chars().any(char::is_control)
            || value.starts_with('-')
            || value.contains(' ')
        {
            return Err(DockerError::InvalidSpec(
                "invalid image reference".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Borrow the image reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated, hardened container configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// Container image.
    pub image: ImageRef,
    /// Optional container entry command.
    pub command: Vec<String>,
    /// Non-secret environment entries.
    pub env: Vec<(String, String)>,
    /// Required memory ceiling, in bytes.
    pub memory_bytes: u64,
    /// Required CPU ceiling, in nanocpus.
    pub nano_cpus: u64,
}

impl ContainerSpec {
    /// Create a hardened spec with network disabled and a non-root user.
    pub fn hardened(
        image: ImageRef,
        command: Vec<String>,
        env: Vec<(String, String)>,
        memory_bytes: u64,
        nano_cpus: u64,
    ) -> Result<Self, DockerError> {
        validate_argv(&command)?;
        validate_env(&env)?;
        if memory_bytes < MIN_MEMORY_BYTES || i64::try_from(memory_bytes).is_err() {
            return Err(DockerError::InvalidSpec(
                "memory limit must be at least 64 MiB and fit the engine API".to_owned(),
            ));
        }
        if nano_cpus == 0 || i64::try_from(nano_cpus).is_err() {
            return Err(DockerError::InvalidSpec(
                "CPU limit must be positive and fit the engine API".to_owned(),
            ));
        }
        let spec = Self {
            image,
            command,
            env,
            memory_bytes,
            nano_cpus,
        };
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), DockerError> {
        validate_argv(&self.command)?;
        validate_env(&self.env)?;
        if self.memory_bytes < MIN_MEMORY_BYTES || i64::try_from(self.memory_bytes).is_err() {
            return Err(DockerError::InvalidSpec("invalid memory limit".to_owned()));
        }
        if self.nano_cpus == 0 || i64::try_from(self.nano_cpus).is_err() {
            return Err(DockerError::InvalidSpec("invalid CPU limit".to_owned()));
        }
        Ok(())
    }
}

/// One argv-based command executed inside a running one-shot container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    /// Program and arguments. Shell interpretation is never implicit.
    pub argv: Vec<String>,
    /// Extra non-secret environment values for this command.
    pub env: Vec<(String, String)>,
    /// Deadline for start and execution; the Engine API bounds create and cleanup separately.
    pub timeout: Duration,
    /// Maximum combined stdout and stderr bytes retained.
    pub output_limit: usize,
}

/// Output and exit status from a one-shot Docker command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    /// Captured standard output, truncated at the configured combined limit.
    pub stdout: Bytes,
    /// Captured standard error, truncated at the configured combined limit.
    pub stderr: Bytes,
    /// Process exit code reported by the Engine API.
    pub exit_code: i64,
    /// Whether any output bytes were dropped due to the cap.
    pub truncated: bool,
}

/// Stable identity for a bounded, single-use warm pool. Persist this value in the caller's
/// resource journal so a later process can recover only this pool's containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmPoolConfig {
    /// Unique pool identity.
    pub pool_id: Uuid,
    /// Unique caller-owned lifecycle identity, suitable for a durable reaper journal.
    pub owner_id: Uuid,
    /// Maximum number of concurrently warm containers.
    pub target_size: usize,
    /// Aggregate memory ceiling for warm containers.
    pub memory_budget_bytes: u64,
}

impl WarmPoolConfig {
    /// Create a new pool identity after validating its bounded resource budget.
    pub fn new(target_size: usize, memory_budget_bytes: u64) -> Result<Self, DockerError> {
        validate_pool_budget(target_size, memory_budget_bytes)?;
        Ok(Self {
            pool_id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            target_size,
            memory_budget_bytes,
        })
    }

    /// Restore persisted identities from the caller's durable resource journal.
    pub fn from_ids(
        pool_id: Uuid,
        owner_id: Uuid,
        target_size: usize,
        memory_budget_bytes: u64,
    ) -> Result<Self, DockerError> {
        validate_pool_budget(target_size, memory_budget_bytes)?;
        Ok(Self {
            pool_id,
            owner_id,
            target_size,
            memory_budget_bytes,
        })
    }

    fn validate(&self, spec: &ContainerSpec) -> Result<(), DockerError> {
        validate_pool_budget(self.target_size, self.memory_budget_bytes)?;
        let aggregate = spec
            .memory_bytes
            .checked_mul(u64::try_from(self.target_size).map_err(|_| {
                DockerError::InvalidSpec(
                    "warm pool size does not fit resource accounting".to_owned(),
                )
            })?)
            .ok_or_else(|| DockerError::InvalidSpec("warm pool memory overflow".to_owned()))?;
        if aggregate > self.memory_budget_bytes {
            return Err(DockerError::InvalidSpec(
                "warm pool exceeds its aggregate memory budget".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Running, pre-started containers reserved for one-use commands. A consumed container is always
/// removed; replenishment creates a fresh container, so no process or filesystem state is reused.
#[derive(Debug)]
pub struct WarmContainerPool {
    docker: Docker,
    spec: ContainerSpec,
    config: WarmPoolConfig,
    ready: VecDeque<String>,
    healthy: bool,
    journal: Option<Arc<dyn PoolJournal>>,
    registration: Option<PoolRegistration>,
}

impl WarmContainerPool {
    /// Durable identity values that the caller should keep in its reaper journal.
    #[must_use]
    pub const fn config(&self) -> &WarmPoolConfig {
        &self.config
    }

    /// Number of currently ready containers.
    #[must_use]
    pub fn ready_count(&self) -> usize {
        self.ready.len()
    }
}

/// A bounded, path-checked tar archive accepted by Docker's image build API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildContext {
    tar_bytes: Bytes,
    dockerfile: String,
}

impl BuildContext {
    /// Validate a raw tar archive, requiring a regular `Dockerfile` at its root.
    pub fn from_tar(bytes: Bytes, max_bytes: usize) -> Result<Self, DockerError> {
        if bytes.is_empty() || bytes.len() > max_bytes {
            return Err(DockerError::InvalidSpec(
                "build archive is empty or over its byte limit".to_owned(),
            ));
        }
        let mut archive = tar::Archive::new(Cursor::new(bytes.as_ref()));
        let entries = archive
            .entries()
            .map_err(|error| DockerError::InvalidSpec(format!("invalid build tar: {error}")))?;
        let mut count = 0_usize;
        let mut dockerfile = None;
        for entry_result in entries {
            let mut entry = entry_result.map_err(|error| {
                DockerError::InvalidSpec(format!("invalid build tar entry: {error}"))
            })?;
            count = count.saturating_add(1);
            if count > MAX_BUILD_ENTRIES {
                return Err(DockerError::InvalidSpec(
                    "build archive has too many entries".to_owned(),
                ));
            }
            let path = entry.path().map_err(|error| {
                DockerError::InvalidSpec(format!("invalid build path: {error}"))
            })?;
            let path_text = path.to_string_lossy();
            if path_text.is_empty()
                || path_text.starts_with('/')
                || path_text.contains('\\')
                || path_text.contains(':')
                || path_text
                    .split('/')
                    .enumerate()
                    .any(|(index, component)| component == ".." || (component == "." && index > 0))
                || path_text.chars().any(char::is_control)
            {
                return Err(DockerError::InvalidSpec(
                    "build archive contains an unsafe or platform-specific path".to_owned(),
                ));
            }
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                return Err(DockerError::InvalidSpec(
                    "build archive may contain only regular files and directories".to_owned(),
                ));
            }
            let normalized_path = match path_text.strip_prefix("./") {
                Some(path) => path,
                None => path_text.as_ref(),
            };
            if normalized_path == "Dockerfile" && kind.is_file() {
                let mut contents = String::new();
                entry.read_to_string(&mut contents).map_err(|error| {
                    DockerError::InvalidSpec(format!("Dockerfile must be UTF-8 text: {error}"))
                })?;
                dockerfile = Some(contents);
            }
        }
        let dockerfile = dockerfile.ok_or_else(|| {
            DockerError::InvalidSpec("build archive has no root Dockerfile".to_owned())
        })?;
        Ok(Self {
            tar_bytes: bytes,
            dockerfile,
        })
    }
}

/// A Dockerfile BuildKit cache mount that must already appear in a single-line `RUN` instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCacheMount {
    /// Stable BuildKit cache identifier.
    pub id: String,
    /// Absolute POSIX path inside the build container.
    pub target: String,
}

/// Cache mounts that a bounded BuildKit build must find in its Dockerfile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Cache mounts expected in the Dockerfile, with stable IDs.
    pub cache_mounts: Vec<BuildCacheMount>,
}

/// Docker connection, validation, and execution failure.
#[derive(Debug, Error)]
pub enum DockerError {
    /// Endpoint is unsupported on the current target or malformed.
    #[error("invalid Docker endpoint: {0}")]
    InvalidEndpoint(String),
    /// Invalid container or command configuration.
    #[error("invalid Docker specification: {0}")]
    InvalidSpec(String),
    /// Docker Engine API failure.
    #[error("Docker Engine API failure: {0}")]
    Engine(#[from] bollard::errors::Error),
    /// Docker reported a build failure in its streamed build response.
    #[error("Docker image build failed: {0}")]
    Build(String),
    /// Docker Engine did not provide an exit code after command completion.
    #[error("Docker Engine omitted the exec exit code")]
    MissingExitCode,
    /// Command exceeded its deadline; the owned container was removed.
    #[error("Docker command timed out")]
    Timeout,
    /// Command was detached when an attached stream was required.
    #[error("Docker Engine detached an attached command")]
    DetachedExec,
    /// The command failed and Docker could not remove its container.
    #[error("Docker operation failed: {primary}; container cleanup failed: {cleanup}")]
    Cleanup { primary: String, cleanup: String },
    /// The asynchronous cleanup task could not be joined.
    #[error("Docker cleanup task failed: {0}")]
    CleanupTask(String),
    /// A previous operation had an uncertain cleanup result; recover the exact owner labels first.
    #[error("warm pool requires orphan recovery before it can be used again")]
    PoolRecoveryRequired,
    /// One or more currently owned pool containers could not be removed.
    #[error("warm pool cleanup failed: {0}")]
    PoolCleanup(String),
    /// Durable pool ownership could not be maintained.
    #[error(transparent)]
    PoolJournal(#[from] PoolJournalError),
}

/// Connected Docker Engine client with the negotiated daemon version.
#[derive(Debug, Clone)]
pub struct Docker {
    client: BollardClient,
    /// Engine version reported by the daemon.
    pub engine_version: String,
    /// API version reported by the daemon.
    pub api_version: String,
}

impl Docker {
    /// Connect to one explicit local Unix socket or Windows named pipe and negotiate versions.
    pub async fn connect(endpoint: &DockerEndpoint) -> Result<Self, DockerError> {
        let client = match endpoint {
            DockerEndpoint::NamedPipe(path) => {
                #[cfg(windows)]
                {
                    BollardClient::connect_with_named_pipe(path, 30, &bollard::API_DEFAULT_VERSION)
                }
                #[cfg(not(windows))]
                {
                    let _ = path;
                    return Err(DockerError::InvalidEndpoint(
                        "Windows named pipes are available only on Windows".to_owned(),
                    ));
                }
            }
            DockerEndpoint::UnixSocket(path) => {
                #[cfg(unix)]
                {
                    let socket_path = path.strip_prefix("unix://").ok_or_else(|| {
                        DockerError::InvalidEndpoint(
                            "Unix endpoint must start with unix://".to_owned(),
                        )
                    })?;
                    BollardClient::connect_with_unix(socket_path, 30, &bollard::API_DEFAULT_VERSION)
                }
                #[cfg(not(unix))]
                {
                    let _ = path;
                    return Err(DockerError::InvalidEndpoint(
                        "Unix sockets are available only on Unix".to_owned(),
                    ));
                }
            }
        }?;
        let version = client.version().await?;
        let engine_version = match version.version {
            Some(value) => value,
            None => "unknown".to_owned(),
        };
        let api_version = match version.api_version {
            Some(value) => value,
            None => "unknown".to_owned(),
        };
        Ok(Self {
            client,
            engine_version,
            api_version,
        })
    }

    /// Execute with caller-persisted ownership IDs so a Reaper can recover a crash orphan.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) async fn exec_once_owned(
        &self,
        spec: &ContainerSpec,
        request: &ExecRequest,
        owner: &WarmPoolConfig,
    ) -> Result<ExecOutput, DockerError> {
        spec.validate()?;
        owner.validate(spec)?;
        validate_argv(&request.argv)?;
        validate_env(&request.env)?;
        if request.timeout.is_zero() || request.output_limit == 0 {
            return Err(DockerError::InvalidSpec(
                "timeout and output limit must be positive".to_owned(),
            ));
        }
        self.exec_once_inner(spec, request, owner.pool_id, owner.owner_id)
            .await
    }

    /// Execute one command with a durable cleanup registration written before container creation.
    /// `owner` must be exclusive to this execution so cleanup cannot address another active pool.
    ///
    /// # Errors
    /// Returns validation, journal, Engine, execution, or cleanup failures.
    pub async fn exec_once_journaled(
        &self,
        spec: &ContainerSpec,
        request: &ExecRequest,
        owner: &WarmPoolConfig,
        journal: Arc<dyn PoolJournal>,
    ) -> Result<ExecOutput, DockerError> {
        spec.validate()?;
        owner.validate(spec)?;
        validate_argv(&request.argv)?;
        validate_env(&request.env)?;
        if request.timeout.is_zero() || request.output_limit == 0 {
            return Err(DockerError::InvalidSpec(
                "timeout and output limit must be positive".to_owned(),
            ));
        }
        let identity = PoolIdentity {
            pool_id: owner.pool_id,
            owner_id: owner.owner_id,
        };
        let registration = journal.register(
            identity,
            Box::new(DockerPoolCleanup {
                docker: self.clone(),
            }),
        )?;
        let execution = self
            .exec_once_inner(spec, request, owner.pool_id, owner.owner_id)
            .await;
        let cleanup = self
            .recover_pool_orphans(owner.pool_id, owner.owner_id)
            .await;
        if let Err(cleanup_error) = cleanup {
            let primary = match execution {
                Ok(_) => "command completed".to_owned(),
                Err(error) => error.to_string(),
            };
            return Err(DockerError::Cleanup {
                primary,
                cleanup: cleanup_error.to_string(),
            });
        }
        if let Err(release_error) = journal.release(&registration) {
            let primary = match execution {
                Ok(_) => "container destruction was confirmed".to_owned(),
                Err(error) => error.to_string(),
            };
            return Err(DockerError::Cleanup {
                primary,
                cleanup: release_error.to_string(),
            });
        }
        execution
    }

    /// Create and pre-start a resource-bounded pool for one-use commands.
    ///
    /// The caller must durably record both IDs in [`WarmPoolConfig`] before relying on crash
    /// recovery. Containers are tagged with those exact IDs before they are created.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) async fn create_warm_pool(
        &self,
        spec: ContainerSpec,
        config: WarmPoolConfig,
    ) -> Result<WarmContainerPool, DockerError> {
        spec.validate()?;
        config.validate(&spec)?;
        let mut pool = WarmContainerPool {
            docker: self.clone(),
            spec,
            config,
            ready: VecDeque::new(),
            healthy: true,
            journal: None,
            registration: None,
        };
        if let Err(primary) = pool.fill_to_target().await {
            let cleanup = pool.drain().await;
            let recovery = pool
                .docker
                .recover_pool_orphans(pool.config.pool_id, pool.config.owner_id)
                .await;
            return match (cleanup, recovery) {
                (Ok(()), Ok(_)) => Err(primary),
                (drain, recover) => Err(DockerError::Cleanup {
                    primary: primary.to_string(),
                    cleanup: format!("drain={drain:?}; exact-label recovery={recover:?}"),
                }),
            };
        }
        Ok(pool)
    }

    /// Durably register exact-label cleanup before creating any container in the pool.
    ///
    /// # Errors
    /// Returns validation, journal, Engine, or pool cleanup failures.
    pub async fn create_warm_pool_journaled(
        &self,
        spec: ContainerSpec,
        config: WarmPoolConfig,
        journal: Arc<dyn PoolJournal>,
    ) -> Result<WarmContainerPool, DockerError> {
        spec.validate()?;
        config.validate(&spec)?;
        let identity = PoolIdentity {
            pool_id: config.pool_id,
            owner_id: config.owner_id,
        };
        let registration = journal.register(
            identity,
            Box::new(DockerPoolCleanup {
                docker: self.clone(),
            }),
        )?;
        let mut pool = WarmContainerPool {
            docker: self.clone(),
            spec,
            config,
            ready: VecDeque::new(),
            healthy: true,
            journal: Some(journal),
            registration: Some(registration),
        };
        if let Err(primary) = pool.fill_to_target().await {
            return match pool.drain().await {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(DockerError::Cleanup {
                    primary: primary.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            };
        }
        Ok(pool)
    }

    /// Recover only confirmed orphan entries, acknowledging each after exact-label cleanup.
    ///
    /// # Errors
    /// Returns journal, Engine listing, or Docker removal failures. A failed cleanup is not
    /// acknowledged and remains available for a later retry.
    pub async fn recover_journaled_orphans(
        &self,
        journal: &dyn PoolJournal,
    ) -> Result<usize, DockerError> {
        let orphans = journal.pending_orphans()?;
        let mut recovered = 0_usize;
        for orphan in orphans {
            self.recover_pool_orphans(orphan.identity.pool_id, orphan.identity.owner_id)
                .await?;
            journal.acknowledge_recovered(&orphan.registration)?;
            recovered = recovered.saturating_add(1);
        }
        Ok(recovered)
    }

    /// Remove only containers carrying both exact Polaris ownership labels.
    ///
    /// The owner ID is expected to be a stale identity recovered from the caller's durable
    /// resource journal. This does not enumerate or mutate other containers.
    pub async fn recover_pool_orphans(
        &self,
        pool_id: Uuid,
        owner_id: Uuid,
    ) -> Result<usize, DockerError> {
        let labels = pool_labels(pool_id, owner_id);
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_owned(),
            labels
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect(),
        );
        let options = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let containers = self.client.list_containers(Some(options)).await?;
        let mut removed = 0_usize;
        for container in containers {
            let Some(id) = container.id else { continue };
            let has_exact_labels = container
                .labels
                .as_ref()
                .is_some_and(|actual| owns_pool_labels(actual, pool_id, owner_id));
            if has_exact_labels {
                self.client
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
        }
        Ok(removed)
    }

    /// Build an image with Bollard's BuildKit Engine API, using only preloaded base/frontend images.
    ///
    /// The request disables build-step networking. Cache mount declarations are validated against
    /// the Dockerfile; cache state remains managed by the local BuildKit daemon. This method reports
    /// the actual image content ID after the daemon completes the build.
    pub async fn build_image(
        &self,
        context: &BuildContext,
        tag: &ImageRef,
        options: &BuildOptions,
    ) -> Result<String, DockerError> {
        validate_build_options(context, options)?;
        for base_image in dockerfile_image_references(&context.dockerfile)? {
            if base_image != "scratch" {
                self.client.inspect_image(&base_image).await.map_err(|error| {
                    DockerError::InvalidSpec(format!(
                        "base image {base_image} must already exist locally; this adapter never pulls images: {error}"
                    ))
                })?;
            }
        }
        if let Some(frontend) = dockerfile_frontend(&context.dockerfile)? {
            self.client.inspect_image(&frontend).await.map_err(|error| {
                DockerError::InvalidSpec(format!(
                    "Dockerfile frontend {frontend} must already exist locally; this adapter never pulls images: {error}"
                ))
            })?;
        }
        let session_id = Uuid::now_v7().to_string();
        let options_builder = bollard::query_parameters::BuildImageOptionsBuilder::default()
            .dockerfile("Dockerfile")
            .t(tag.as_str())
            .version(bollard::query_parameters::BuilderVersion::BuilderBuildKit)
            .session(&session_id)
            .networkmode("none")
            .rm(true)
            .forcerm(true);
        let options = options_builder.build();
        self.client
            .build_image(
                options,
                None,
                Some(bollard::body_full(context.tar_bytes.clone())),
            )
            .try_fold((), |(), progress| async move {
                if let Some(details) = progress.error_detail {
                    let message = match details.message {
                        Some(message) => message,
                        None => "unspecified build error".to_owned(),
                    };
                    return Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 500,
                        message,
                    });
                }
                Ok(())
            })
            .await
            .map_err(DockerError::Engine)?;
        let image = self.client.inspect_image(tag.as_str()).await?;
        image
            .id
            .ok_or_else(|| DockerError::Build("engine returned no image content ID".to_owned()))
    }

    async fn exec_once_inner(
        &self,
        spec: &ContainerSpec,
        request: &ExecRequest,
        pool_id: Uuid,
        owner_id: Uuid,
    ) -> Result<ExecOutput, DockerError> {
        let config = container_config(spec, Some(pool_labels(pool_id, owner_id)))?;
        let created = self.client.create_container(None, config).await?;
        let (cleanup_trigger, cleanup_signal) = tokio::sync::oneshot::channel();
        let cleanup_client = self.client.clone();
        let cleanup_id = created.id.clone();
        let cleanup_task = tokio::spawn(async move {
            match cleanup_signal.await {
                Ok(()) | Err(_) => {}
            }
            tokio::time::timeout(
                Duration::from_secs(15),
                cleanup_client.remove_container(
                    &cleanup_id,
                    Some(
                        bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .build(),
                    ),
                ),
            )
            .await
        });
        let result =
            tokio::time::timeout(request.timeout, self.start_and_exec(&created.id, request))
                .await
                .map_err(|_| DockerError::Timeout)
                .and_then(|execution| execution);
        if cleanup_trigger.send(()).is_err() {
            return Err(DockerError::CleanupTask(
                "cleanup task stopped before receiving its signal".to_owned(),
            ));
        }
        let cleanup = cleanup_task
            .await
            .map_err(|error| DockerError::CleanupTask(error.to_string()))?;
        match (result, cleanup) {
            (Ok(output), Ok(Ok(()))) => Ok(output),
            (Err(error), Ok(Ok(()))) => Err(error),
            (Ok(_), Ok(Err(error))) => Err(DockerError::Engine(error)),
            (Err(primary), Ok(Err(cleanup))) => Err(DockerError::Cleanup {
                primary: primary.to_string(),
                cleanup: cleanup.to_string(),
            }),
            (_, Err(_)) => Err(DockerError::Timeout),
        }
    }

    async fn create_pool_container(
        &self,
        spec: &ContainerSpec,
        pool_id: Uuid,
        owner_id: Uuid,
    ) -> Result<String, DockerError> {
        let config = container_config(spec, Some(pool_labels(pool_id, owner_id)))?;
        let created = self.client.create_container(None, config).await?;
        if let Err(primary) = self.client.start_container(&created.id, None).await {
            return match self
                .client
                .remove_container(
                    &created.id,
                    Some(
                        bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .build(),
                    ),
                )
                .await
            {
                Ok(()) => Err(DockerError::Engine(primary)),
                Err(cleanup) => Err(DockerError::Cleanup {
                    primary: primary.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            };
        }
        Ok(created.id)
    }

    async fn exec_started_with_cleanup(
        &self,
        container_id: &str,
        request: &ExecRequest,
    ) -> Result<ExecOutput, DockerError> {
        let (cleanup_trigger, cleanup_signal) = tokio::sync::oneshot::channel();
        let cleanup_client = self.client.clone();
        let cleanup_id = container_id.to_owned();
        let cleanup_task = tokio::spawn(async move {
            let _ = cleanup_signal.await;
            tokio::time::timeout(
                Duration::from_secs(15),
                cleanup_client.remove_container(
                    &cleanup_id,
                    Some(
                        bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .build(),
                    ),
                ),
            )
            .await
        });
        let execution = tokio::time::timeout(
            request.timeout,
            self.exec_in_started_container(container_id, request),
        )
        .await
        .map_err(|_| DockerError::Timeout)
        .and_then(|result| result);
        let _ = cleanup_trigger.send(());
        let cleanup = cleanup_task
            .await
            .map_err(|error| DockerError::CleanupTask(error.to_string()))?;
        match (execution, cleanup) {
            (Ok(output), Ok(Ok(()))) => Ok(output),
            (Err(error), Ok(Ok(()))) => Err(error),
            (Ok(_), Ok(Err(error))) => Err(DockerError::Engine(error)),
            (Err(primary), Ok(Err(cleanup))) => Err(DockerError::Cleanup {
                primary: primary.to_string(),
                cleanup: cleanup.to_string(),
            }),
            (_, Err(_)) => Err(DockerError::Timeout),
        }
    }

    async fn exec_in_started_container(
        &self,
        container_id: &str,
        request: &ExecRequest,
    ) -> Result<ExecOutput, DockerError> {
        let exec = self
            .client
            .create_exec(
                container_id,
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
                    privileged: Some(false),
                    tty: Some(false),
                    ..CreateExecOptions::default()
                },
            )
            .await?;
        let StartExecResults::Attached { output, .. } =
            self.client.start_exec(&exec.id, None).await?
        else {
            return Err(DockerError::DetachedExec);
        };
        let mut stdout = BytesMut::new();
        let mut stderr = BytesMut::new();
        let mut captured = 0_usize;
        let mut truncated = false;
        let mut stream = output;
        while let Some(log) = stream.try_next().await? {
            let (destination, bytes) = match log {
                LogOutput::StdOut { message } => (&mut stdout, message),
                LogOutput::StdErr { message } => (&mut stderr, message),
                LogOutput::Console { message } | LogOutput::StdIn { message } => {
                    (&mut stdout, message)
                }
            };
            let remaining = request.output_limit.saturating_sub(captured);
            let keep = remaining.min(bytes.len());
            if keep > 0 {
                destination.extend_from_slice(bytes.get(..keep).ok_or_else(|| {
                    DockerError::InvalidSpec("invalid Engine output range".to_owned())
                })?);
                captured = captured.saturating_add(keep);
            }
            truncated |= keep < bytes.len();
        }
        let inspected = self.client.inspect_exec(&exec.id).await?;
        let exit_code = inspected.exit_code.ok_or(DockerError::MissingExitCode)?;
        Ok(ExecOutput {
            stdout: stdout.freeze(),
            stderr: stderr.freeze(),
            exit_code,
            truncated,
        })
    }

    async fn start_and_exec(
        &self,
        container_id: &str,
        request: &ExecRequest,
    ) -> Result<ExecOutput, DockerError> {
        self.client.start_container(container_id, None).await?;
        let exec = self
            .client
            .create_exec(
                container_id,
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
                    privileged: Some(false),
                    tty: Some(false),
                    ..CreateExecOptions::default()
                },
            )
            .await?;
        let StartExecResults::Attached { output, .. } =
            self.client.start_exec(&exec.id, None).await?
        else {
            return Err(DockerError::DetachedExec);
        };
        let mut stdout = BytesMut::new();
        let mut stderr = BytesMut::new();
        let mut captured = 0_usize;
        let mut truncated = false;
        let mut stream = output;
        while let Some(log) = stream.try_next().await? {
            let (destination, bytes) = match log {
                LogOutput::StdOut { message } => (&mut stdout, message),
                LogOutput::StdErr { message } => (&mut stderr, message),
                LogOutput::Console { message } | LogOutput::StdIn { message } => {
                    (&mut stdout, message)
                }
            };
            let remaining = request.output_limit.saturating_sub(captured);
            let keep = remaining.min(bytes.len());
            if keep > 0 {
                destination.extend_from_slice(bytes.get(..keep).ok_or_else(|| {
                    DockerError::InvalidSpec("invalid Engine output range".to_owned())
                })?);
                captured = captured.saturating_add(keep);
            }
            truncated |= keep < bytes.len();
        }
        let inspected = self.client.inspect_exec(&exec.id).await?;
        let exit_code = inspected.exit_code.ok_or(DockerError::MissingExitCode)?;
        Ok(ExecOutput {
            stdout: stdout.freeze(),
            stderr: stderr.freeze(),
            exit_code,
            truncated,
        })
    }
}

impl WarmContainerPool {
    /// Execute once in a warm container, destroy it, and replenish the pool with a fresh one.
    pub async fn execute(&mut self, request: &ExecRequest) -> Result<ExecOutput, DockerError> {
        if !self.healthy {
            return Err(DockerError::PoolRecoveryRequired);
        }
        validate_argv(&request.argv)?;
        validate_env(&request.env)?;
        if request.timeout.is_zero() || request.output_limit == 0 {
            return Err(DockerError::InvalidSpec(
                "timeout and output limit must be positive".to_owned(),
            ));
        }
        if self.ready.is_empty() {
            self.fill_to_target().await?;
        }
        let container_id = self.ready.pop_front().ok_or_else(|| {
            DockerError::InvalidSpec("warm pool has no ready container".to_owned())
        })?;
        let execution = self
            .docker
            .exec_started_with_cleanup(&container_id, request)
            .await;
        // Replace only after the consumed container is gone. If removal failed, report it and
        // leave the pool short so a later call can recover from the durable labels.
        match execution {
            Ok(output) => {
                if let Err(error) = self.fill_to_target().await {
                    self.healthy = false;
                    return Err(error);
                }
                Ok(output)
            }
            Err(error) => {
                self.healthy = false;
                Err(error)
            }
        }
    }

    /// Remove every container carrying this pool's exact durable labels and rebuild the pool.
    /// Call after a failure or at startup with an owner ID confirmed stale by the caller's journal.
    pub async fn recover_and_refill(&mut self) -> Result<usize, DockerError> {
        let removed = self
            .docker
            .recover_pool_orphans(self.config.pool_id, self.config.owner_id)
            .await?;
        self.ready.clear();
        self.healthy = true;
        if let Err(error) = self.fill_to_target().await {
            self.healthy = false;
            return Err(error);
        }
        Ok(removed)
    }

    /// Remove all currently ready containers owned by this in-memory pool.
    pub async fn drain(&mut self) -> Result<(), DockerError> {
        let mut failures = Vec::new();
        while let Some(id) = self.ready.pop_front() {
            if let Err(error) = self
                .docker
                .client
                .remove_container(
                    &id,
                    Some(
                        bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                            .force(true)
                            .build(),
                    ),
                )
                .await
            {
                failures.push(error.to_string());
            }
        }
        let recovery = self
            .docker
            .recover_pool_orphans(self.config.pool_id, self.config.owner_id)
            .await;
        match recovery {
            Ok(_removed) => failures.clear(),
            Err(error) => {
                self.healthy = false;
                failures.push(error.to_string());
            }
        }
        if !failures.is_empty() {
            return Err(DockerError::PoolCleanup(failures.join("; ")));
        }
        if let (Some(journal), Some(registration)) = (&self.journal, &self.registration) {
            journal.release(registration)?;
        }
        self.registration = None;
        Ok(())
    }

    async fn fill_to_target(&mut self) -> Result<(), DockerError> {
        while self.ready.len() < self.config.target_size {
            let id = self
                .docker
                .create_pool_container(&self.spec, self.config.pool_id, self.config.owner_id)
                .await?;
            self.ready.push_back(id);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DockerPoolCleanup {
    docker: Docker,
}

impl PoolCleanup for DockerPoolCleanup {
    fn cleanup<'a>(
        &'a self,
        identity: &'a PoolIdentity,
    ) -> futures::future::BoxFuture<'a, Result<(), PoolCleanupError>> {
        Box::pin(async move {
            self.docker
                .recover_pool_orphans(identity.pool_id, identity.owner_id)
                .await
                .map(|_removed| ())
                .map_err(|error| PoolCleanupError(error.to_string()))
        })
    }
}

fn validate_pool_budget(target_size: usize, memory_budget_bytes: u64) -> Result<(), DockerError> {
    if target_size == 0 || target_size > 16 {
        return Err(DockerError::InvalidSpec(
            "warm pool size must be between 1 and 16".to_owned(),
        ));
    }
    if memory_budget_bytes < MIN_MEMORY_BYTES {
        return Err(DockerError::InvalidSpec(
            "warm pool memory budget must be at least 64 MiB".to_owned(),
        ));
    }
    Ok(())
}

fn pool_labels(pool_id: Uuid, owner_id: Uuid) -> HashMap<String, String> {
    HashMap::from([
        ("io.polaris.devenv.managed".to_owned(), "true".to_owned()),
        ("io.polaris.devenv.pool".to_owned(), pool_id.to_string()),
        ("io.polaris.devenv.owner".to_owned(), owner_id.to_string()),
    ])
}

fn owns_pool_labels(actual: &HashMap<String, String>, pool_id: Uuid, owner_id: Uuid) -> bool {
    pool_labels(pool_id, owner_id)
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

fn container_config(
    spec: &ContainerSpec,
    labels: Option<HashMap<String, String>>,
) -> Result<ContainerCreateBody, DockerError> {
    let env = spec
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    Ok(ContainerCreateBody {
        image: Some(spec.image.0.clone()),
        cmd: if spec.command.is_empty() {
            None
        } else {
            Some(spec.command.clone())
        },
        env: Some(env),
        user: Some("1000:1000".to_owned()),
        network_disabled: Some(true),
        labels,
        host_config: Some(HostConfig {
            memory: Some(
                i64::try_from(spec.memory_bytes)
                    .map_err(|_| DockerError::InvalidSpec("memory limit overflow".to_owned()))?,
            ),
            nano_cpus: Some(
                i64::try_from(spec.nano_cpus)
                    .map_err(|_| DockerError::InvalidSpec("CPU limit overflow".to_owned()))?,
            ),
            network_mode: Some("none".to_owned()),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
            pids_limit: Some(256),
            tmpfs: Some(HashMap::from([
                ("/tmp".to_owned(), "rw,noexec,nosuid,size=64m".to_owned()),
                ("/run".to_owned(), "rw,noexec,nosuid,size=16m".to_owned()),
            ])),
            ..HostConfig::default()
        }),
        ..ContainerCreateBody::default()
    })
}

fn validate_argv(argv: &[String]) -> Result<(), DockerError> {
    if argv.is_empty()
        || argv
            .iter()
            .any(|arg| arg.is_empty() || arg.chars().any(char::is_control))
    {
        return Err(DockerError::InvalidSpec(
            "argv must contain a program and no empty/control arguments".to_owned(),
        ));
    }
    Ok(())
}

fn validate_build_options(
    context: &BuildContext,
    options: &BuildOptions,
) -> Result<(), DockerError> {
    let mut ids = std::collections::BTreeSet::new();
    let mut targets = std::collections::BTreeSet::new();
    for mount in &options.cache_mounts {
        let valid_id = !mount.id.is_empty()
            && mount.id.len() <= 128
            && mount.id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            });
        let valid_target = mount.target.starts_with('/')
            && !mount.target.starts_with("//")
            && !mount.target.ends_with('/')
            && mount.target.len() <= 512
            && !mount.target.chars().any(char::is_control)
            && !mount.target.contains('\\')
            && !mount
                .target
                .split('/')
                .skip(1)
                .any(|component| component == "." || component == ".." || component.is_empty());
        if !valid_id
            || !valid_target
            || !ids.insert(mount.id.as_str())
            || !targets.insert(mount.target.as_str())
            || !dockerfile_has_cache_mount(&context.dockerfile, mount)
        {
            return Err(DockerError::InvalidSpec(format!(
                "cache mount {:?} is invalid or missing from a single-line Dockerfile RUN instruction",
                mount.id
            )));
        }
    }
    let frontend = dockerfile_frontend(&context.dockerfile)?;
    if !options.cache_mounts.is_empty() && frontend.is_none() {
        return Err(DockerError::InvalidSpec(
            "BuildKit cache mounts require an explicit local Dockerfile frontend version 1.2 or newer".to_owned(),
        ));
    }
    if dockerfile_has_remote_add(&context.dockerfile) {
        return Err(DockerError::InvalidSpec(
            "remote ADD sources are disabled in bounded builds".to_owned(),
        ));
    }
    Ok(())
}

fn dockerfile_has_cache_mount(dockerfile: &str, expected: &BuildCacheMount) -> bool {
    dockerfile.lines().any(|line| {
        let mut words = line.split_whitespace();
        if !words
            .next()
            .is_some_and(|keyword| keyword.eq_ignore_ascii_case("RUN"))
        {
            return false;
        }
        words.any(|token| {
            let Some(mount_spec) = token.strip_prefix("--mount=") else {
                return false;
            };
            let mut mount_type = None;
            let mut target = None;
            let mut id = None;
            for part in mount_spec.split(',') {
                let Some((key, value)) = part.split_once('=') else {
                    continue;
                };
                match key.to_ascii_lowercase().as_str() {
                    "type" => mount_type = Some(value),
                    "target" | "dst" | "destination" => target = Some(value),
                    "id" => id = Some(value),
                    _ => {}
                }
            }
            mount_type == Some("cache")
                && target == Some(expected.target.as_str())
                && id == Some(expected.id.as_str())
        })
    })
}

fn dockerfile_image_references(dockerfile: &str) -> Result<Vec<String>, DockerError> {
    let mut images = Vec::new();
    let mut stages = std::collections::BTreeSet::new();
    for line in dockerfile.lines() {
        let instruction = line.trim();
        let mut words = instruction.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("FROM") {
            continue;
        }
        if instruction.contains('\\') {
            return Err(DockerError::InvalidSpec(
                "Dockerfile FROM continuations are unsupported by local-image validation"
                    .to_owned(),
            ));
        }
        let mut image = None;
        for word in words {
            if word.eq_ignore_ascii_case("AS") {
                break;
            }
            if word.starts_with("--") {
                continue;
            }
            image = Some(word);
            break;
        }
        let image = image.ok_or_else(|| {
            DockerError::InvalidSpec("Dockerfile FROM instruction has no base image".to_owned())
        })?;
        if image.contains('$') || image.chars().any(char::is_control) {
            return Err(DockerError::InvalidSpec(
                "Dockerfile base image must be a literal reference".to_owned(),
            ));
        }
        images.push(image.to_owned());
        let words = instruction.split_whitespace().collect::<Vec<_>>();
        if let Some(as_index) = words
            .iter()
            .position(|word| word.eq_ignore_ascii_case("AS"))
        {
            if let Some(stage) = words.get(as_index + 1) {
                stages.insert((*stage).to_owned());
            }
        }
    }
    for line in dockerfile.lines() {
        let mut words = line.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("COPY") {
            continue;
        }
        for word in words {
            if let Some(reference) = word.strip_prefix("--from=") {
                if !stages.contains(reference) && reference.parse::<usize>().is_err() {
                    if reference.contains('$') {
                        return Err(DockerError::InvalidSpec(
                            "COPY --from image must be a literal local reference".to_owned(),
                        ));
                    }
                    images.push(reference.to_owned());
                }
            } else if word == "--from" {
                return Err(DockerError::InvalidSpec(
                    "COPY --from must use the single-token --from=<stage-or-image> form".to_owned(),
                ));
            }
        }
    }
    Ok(images)
}

fn dockerfile_frontend(dockerfile: &str) -> Result<Option<String>, DockerError> {
    let Some(line) = dockerfile.lines().find(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("# syntax=")
    }) else {
        return Ok(None);
    };
    let reference = line
        .trim_start()
        .get("# syntax=".len()..)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DockerError::InvalidSpec("invalid Dockerfile syntax directive".to_owned())
        })?;
    let Some(version) = reference.strip_prefix("docker/dockerfile:") else {
        return Err(DockerError::InvalidSpec(
            "only a locally installed docker/dockerfile frontend is accepted".to_owned(),
        ));
    };
    let numeric = version.split('@').next().unwrap_or_default();
    let mut parts = numeric.split('.');
    let major = parts.next().and_then(|value| value.parse::<u32>().ok());
    let minor = parts.next().and_then(|value| value.parse::<u32>().ok());
    if major.is_none_or(|major| major < 1)
        || (major == Some(1) && minor.is_none_or(|minor| minor < 2))
    {
        return Err(DockerError::InvalidSpec(
            "Dockerfile frontend must be version 1.2 or newer for RUN cache mounts".to_owned(),
        ));
    }
    ImageRef::new(reference.to_owned())?;
    Ok(Some(reference.to_owned()))
}

fn dockerfile_has_remote_add(dockerfile: &str) -> bool {
    let mut logical = String::new();
    for line in dockerfile.lines() {
        let trimmed = line.trim_end();
        let continues = trimmed.ends_with('\\');
        logical.push_str(trimmed.trim_end_matches('\\'));
        logical.push(' ');
        if continues {
            continue;
        }
        if logical_add_is_remote(&logical) {
            return true;
        }
        logical.clear();
    }
    logical_add_is_remote(&logical)
}

fn logical_add_is_remote(logical: &str) -> bool {
    let mut words = logical.split_whitespace();
    words
        .next()
        .is_some_and(|keyword| keyword.eq_ignore_ascii_case("ADD"))
        && logical
            .split_whitespace()
            .any(|word| word.starts_with("http://") || word.starts_with("https://"))
}

fn validate_env(env: &[(String, String)]) -> Result<(), DockerError> {
    let mut names = std::collections::BTreeSet::new();
    for (name, value) in env {
        let upper = name.to_ascii_uppercase();
        let secret_name = [
            "SECRET",
            "TOKEN",
            "PASSWORD",
            "PRIVATE_KEY",
            "API_KEY",
            "CREDENTIAL",
        ]
        .iter()
        .any(|needle| upper.contains(needle));
        let secret_value = value.to_ascii_lowercase().starts_with("sk-")
            || value.to_ascii_lowercase().starts_with("ghp_")
            || value.contains("-----BEGIN ");
        if name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
            || !names.insert(name)
            || name.contains('=')
            || value.chars().any(char::is_control)
            || secret_name
            || secret_value
        {
            return Err(DockerError::InvalidSpec(
                "invalid, duplicate, or secret-looking environment entry".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let image = ImageRef::new(
            "sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697",
        );
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
        let journal =
            std::env::temp_dir().join(format!("polaris-devenv-test-{}.txt", config.owner_id));
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
}
