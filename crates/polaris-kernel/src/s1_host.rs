//! Production composition for the authenticated local System-One worker.
//!
//! The host owns the supervisor, lifecycle, readiness-fenced policy client, and generation
//! resolving decision and PII adapters. Worker credentials travel only through bootstrap stdin.

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::Duration,
};

use polaris_kernel_bus::Bus;
use polaris_kernel_supervisor::{
    BackoffSpec, ChildName, HealthPolicy, OnExhausted, RestartIntensity, RestartPolicy,
    StopOutcome, Supervisor, SupervisorConfig, TaskError, TaskSpec, WorkerSpec,
};
use polaris_sdk::{
    contracts::WorkerKind,
    ids::{Cid, SessionId},
    registry::{EventStore, PlatformError, PlatformOps, SpawnSpec},
};
use polaris_system_one::{
    policy::{PolicyClient, PolicyConfig},
    schemas::SchemaSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    KernelError,
    worker_lifecycle::{CurrentS1Engine, CurrentS1PiiDetector, S1WorkerLifecycle},
};

/// Text normalization understood by the native S1 worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S1Normalization {
    /// Pass text through unchanged.
    Raw,
    /// Apply the GLiNER2 Python 2.1-compatible normalization contract.
    Gliner2Python21,
}

impl S1Normalization {
    fn argument(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Gliner2Python21 => "gliner2-python-2.1",
        }
    }
}

/// Immutable paths and bounded operational limits for a production local S1 worker.
#[derive(Debug, Clone)]
pub struct LocalS1Config {
    /// Absolute native worker executable path.
    pub worker_executable: PathBuf,
    /// Absolute S1 model directory.
    pub model_dir: PathBuf,
    /// Absolute ONNX Runtime shared library path.
    pub ort_dylib: PathBuf,
    /// Absolute local named-pipe or Unix-domain-socket endpoint.
    pub endpoint: PathBuf,
    /// Optional absolute PII model directory served by the same worker.
    pub pii_model_dir: Option<PathBuf>,
    /// Model input normalization contract.
    pub normalization: S1Normalization,
    /// Worker model idle eviction period.
    pub idle_ttl: Duration,
    /// Authentication and service-readiness startup deadline.
    pub startup_timeout: Duration,
    /// Worker-exit polling cadence.
    pub poll_interval: Duration,
    /// Authenticated health probe cadence.
    pub health_interval: Duration,
    /// Per-probe timeout.
    pub health_timeout: Duration,
    /// Grace period for worker shutdown before forced termination.
    pub stop_grace: Duration,
    /// Maximum worker RPC deadline.
    pub client_timeout: Duration,
    /// Deadline for draining/pausing or explicitly resuming native inference around heavy work.
    pub model_pause_timeout: Duration,
    /// Maximum worker request size in UTF-8 bytes.
    pub max_input_bytes: usize,
    /// Maximum concurrent policy operations.
    pub max_policy_inflight: usize,
    /// Consecutive transport failures before the policy circuit closes.
    pub policy_failure_threshold: u32,
}

impl LocalS1Config {
    /// Create a local worker profile with bounded production defaults.
    #[must_use]
    pub fn new(
        worker_executable: impl AsRef<Path>,
        model_dir: impl AsRef<Path>,
        ort_dylib: impl AsRef<Path>,
        endpoint: impl AsRef<Path>,
    ) -> Self {
        Self {
            worker_executable: worker_executable.as_ref().to_owned(),
            model_dir: model_dir.as_ref().to_owned(),
            ort_dylib: ort_dylib.as_ref().to_owned(),
            endpoint: endpoint.as_ref().to_owned(),
            pii_model_dir: None,
            normalization: S1Normalization::Raw,
            idle_ttl: Duration::from_secs(300),
            startup_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(100),
            health_interval: Duration::from_secs(2),
            health_timeout: Duration::from_secs(2),
            stop_grace: Duration::from_secs(5),
            client_timeout: Duration::from_secs(10),
            model_pause_timeout: Duration::from_secs(120),
            max_input_bytes: 262_144,
            max_policy_inflight: 16,
            policy_failure_threshold: 3,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), KernelError> {
        let valid_file = |path: &Path| path.is_absolute() && path.is_file();
        let valid_directory = |path: &Path| path.is_absolute() && path.is_dir();
        let valid_pii = self
            .pii_model_dir
            .as_ref()
            .is_none_or(|path| valid_directory(path));
        let idle_millis_valid = self.idle_ttl >= Duration::from_millis(1)
            && self.idle_ttl <= Duration::from_secs(86_400)
            && u64::try_from(self.idle_ttl.as_millis()).is_ok();
        if !valid_file(&self.worker_executable)
            || !valid_directory(&self.model_dir)
            || !valid_file(&self.ort_dylib)
            || !valid_endpoint(&self.endpoint)
            || !valid_pii
            || !idle_millis_valid
            || self.startup_timeout.is_zero()
            || self.startup_timeout > Duration::from_secs(120)
            || self.poll_interval.is_zero()
            || self.poll_interval > Duration::from_secs(120)
            || self.health_interval.is_zero()
            || self.health_interval > Duration::from_secs(120)
            || self.health_timeout.is_zero()
            || self.health_timeout > Duration::from_secs(120)
            || self.stop_grace.is_zero()
            || self.stop_grace > Duration::from_secs(120)
            || self.client_timeout.is_zero()
            || self.client_timeout > Duration::from_secs(120)
            || self.model_pause_timeout.is_zero()
            || self.model_pause_timeout > Duration::from_secs(120)
            || self.max_input_bytes == 0
            || self.max_input_bytes > 262_144
            || self.max_policy_inflight == 0
            || self.max_policy_inflight > 4096
            || self.policy_failure_threshold == 0
            || self.policy_failure_threshold > 0x7fff_ffff
        {
            return Err(KernelError::Configuration(
                "invalid local S1 worker configuration",
            ));
        }
        Ok(())
    }

    fn worker_spec(&self, session: SessionId) -> Result<WorkerSpec, KernelError> {
        let idle_ttl = u64::try_from(self.idle_ttl.as_millis())
            .map_err(|_| KernelError::Configuration("invalid local S1 worker configuration"))?;
        let mut args = vec![
            OsString::from("--mode"),
            OsString::from("s1"),
            OsString::from("--model-dir"),
            self.model_dir.clone().into_os_string(),
            OsString::from("--ort-dylib"),
            self.ort_dylib.clone().into_os_string(),
            OsString::from("--normalization"),
            OsString::from(self.normalization.argument()),
            OsString::from("--idle-ttl-ms"),
            OsString::from(idle_ttl.to_string()),
        ];
        #[cfg(windows)]
        args.extend([
            OsString::from("--pipe"),
            self.endpoint.clone().into_os_string(),
        ]);
        #[cfg(unix)]
        args.extend([
            OsString::from("--socket"),
            self.endpoint.clone().into_os_string(),
        ]);
        #[cfg(not(any(windows, unix)))]
        return Err(KernelError::Configuration(
            "local S1 transport is unsupported",
        ));
        if let Some(directory) = self.pii_model_dir.as_ref() {
            args.extend([
                OsString::from("--pii-model-dir"),
                directory.clone().into_os_string(),
            ]);
        }
        let mut task_digest = [0; 32];
        let (first_half, second_half) = task_digest.split_at_mut(16);
        first_half.copy_from_slice(session.as_uuid().as_bytes());
        second_half.copy_from_slice(session.as_uuid().as_bytes());
        let task = TaskSpec {
            name: ChildName::new("s1-worker", Cid::from_bytes(task_digest)),
            policy: RestartPolicy::Permanent,
            intensity: RestartIntensity::default(),
            backoff: BackoffSpec::default(),
            on_exhausted: OnExhausted::StopChild,
            worker: Some(WorkerKind::S1),
        };
        Ok(WorkerSpec {
            task,
            process: SpawnSpec {
                program: self.worker_executable.clone(),
                args,
                cwd: Some(self.model_dir.clone()),
                // The engines refuse to load unless the device is explicit; CPU is the only
                // provider verified by S0-4, and the host environment must not change it.
                env: vec![(OsString::from("GLINER2_DEVICE"), OsString::from("cpu"))],
            },
            poll_interval: self.poll_interval,
            stop_grace: self.stop_grace,
        })
    }
}

fn valid_endpoint(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Some(parent) = path.parent() else {
            return false;
        };
        if path.as_os_str().len() > 100 {
            return false;
        }
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) => {
                metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o077 == 0
            }
            Err(_) => false,
        }
    }
    #[cfg(windows)]
    {
        let Some(name) = path.to_str() else {
            return false;
        };
        let prefix = r"\\.\pipe\polaris-";
        let Some(suffix) = name.strip_prefix(prefix) else {
            return false;
        };
        !suffix.is_empty()
            && suffix.len() <= 128
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Kernel-owned S1 supervisor, policy, and current-generation adapters.
pub struct S1Host<S: EventStore + 'static> {
    supervisor: Arc<Supervisor<S>>,
    lifecycle: Arc<S1WorkerLifecycle<polaris_platform::process::ProcessPlatform>>,
    platform: Arc<polaris_platform::process::ProcessPlatform>,
    policy:
        Arc<PolicyClient<CurrentS1Engine<polaris_platform::process::ProcessPlatform>, SchemaSet>>,
    engine: Arc<CurrentS1Engine<polaris_platform::process::ProcessPlatform>>,
    detector: Arc<CurrentS1PiiDetector<polaris_platform::process::ProcessPlatform>>,
    config: LocalS1Config,
}

/// Concrete S1 host type owned by the local kernel event log.
pub type LocalS1Host = S1Host<Arc<polaris_storage::eventlog::EventLog>>;

impl<S: EventStore + 'static> fmt::Debug for S1Host<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S1Host")
            .field("config", &"<local S1 paths configured>")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<S: EventStore + 'static> S1Host<S> {
    /// Spawn, bootstrap, authenticate, and health-check the configured S1 worker before returning.
    /// # Errors
    /// Invalid profile, startup timeout, supervisor registration, or authenticated readiness failure.
    pub async fn start(
        config: LocalS1Config,
        platform: Arc<polaris_platform::process::ProcessPlatform>,
        bus: Arc<Bus<S>>,
        session: SessionId,
        root: CancellationToken,
    ) -> Result<Self, KernelError> {
        config.validate()?;
        let worker_spec = config.worker_spec(session)?;
        let client_config = polaris_system_one::client::ClientConfig::new(
            config.client_timeout,
            config.max_input_bytes,
        )
        .map_err(|_| KernelError::Configuration("invalid local S1 client bounds"))?;
        let lifecycle = Arc::new(
            S1WorkerLifecycle::new(
                Arc::clone(&platform),
                config.endpoint.clone(),
                client_config,
                config.startup_timeout,
            )
            .map_err(|_| KernelError::Configuration("invalid local S1 lifecycle bounds"))?,
        );
        let engine = Arc::new(CurrentS1Engine::new(Arc::clone(&lifecycle)));
        let detector = Arc::new(CurrentS1PiiDetector::new(Arc::clone(&lifecycle)));
        let schemas = SchemaSet::load_builtin()
            .map_err(|_| KernelError::Configuration("built-in System One schemas unavailable"))?;
        let policy = Arc::new(
            PolicyClient::new(
                CurrentS1Engine::new(Arc::clone(&lifecycle)),
                schemas,
                PolicyConfig {
                    failure_threshold: config.policy_failure_threshold,
                    max_inflight: config.max_policy_inflight,
                    max_deadline: config.client_timeout,
                },
            )
            .map_err(|_| KernelError::Configuration("invalid System One policy bounds"))?,
        );
        let weak_policy: Weak<_> = Arc::downgrade(&policy);
        lifecycle
            .set_readiness_observer(Arc::new(move |ready| {
                if let Some(policy) = weak_policy.upgrade() {
                    let _reported = policy.report_worker_readiness(ready);
                }
            }))
            .map_err(|_| KernelError::Configuration("S1 lifecycle state unavailable"))?;
        let weak_policy: Weak<_> = Arc::downgrade(&policy);
        lifecycle
            .set_model_ready_observer(Arc::new(move || {
                if let Some(policy) = weak_policy.upgrade() {
                    let _recovered = policy.recover_if_model_ready(true);
                }
            }))
            .map_err(|_| KernelError::Configuration("S1 lifecycle state unavailable"))?;
        policy
            .report_worker_readiness(false)
            .map_err(|_| KernelError::Configuration("S1 policy epoch unavailable"))?;

        let supervisor = Arc::new(Supervisor::new(
            bus,
            session,
            root,
            SupervisorConfig::default(),
        ));
        supervisor
            .spawn_worker_with_lifecycle(
                Arc::clone(&platform),
                worker_spec,
                Arc::clone(&lifecycle),
                HealthPolicy {
                    interval: config.health_interval,
                    timeout: config.health_timeout,
                },
                config.startup_timeout,
            )
            .map_err(|_| KernelError::Configuration("S1 worker supervisor unavailable"))?;

        let readiness = tokio::time::timeout(config.startup_timeout, async {
            loop {
                if lifecycle
                    .connection()
                    .is_ok_and(|connection| connection.is_some())
                {
                    return Ok::<(), KernelError>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        match readiness {
            Ok(Ok(())) => Ok(Self {
                supervisor,
                lifecycle,
                platform,
                policy,
                engine,
                detector,
                config,
            }),
            Ok(Err(error)) => {
                match stop_supervisor(&supervisor, &platform, &lifecycle, config.stop_grace).await {
                    Ok(_) => Err(error),
                    Err(cleanup) => Err(cleanup),
                }
            }
            Err(_) => {
                match stop_supervisor(&supervisor, &platform, &lifecycle, config.stop_grace).await {
                    Ok(_) => Err(KernelError::Runtime(
                        "S1 worker startup timed out".to_owned(),
                    )),
                    Err(cleanup) => Err(cleanup),
                }
            }
        }
    }

    /// Clone the readiness-fenced policy owner used by production consumers.
    #[must_use]
    pub fn policy(
        &self,
    ) -> Arc<PolicyClient<CurrentS1Engine<polaris_platform::process::ProcessPlatform>, SchemaSet>>
    {
        Arc::clone(&self.policy)
    }

    /// Clone the adapter that resolves the current worker tree for each decision.
    #[must_use]
    pub fn engine(&self) -> Arc<CurrentS1Engine<polaris_platform::process::ProcessPlatform>> {
        Arc::clone(&self.engine)
    }

    /// Clone the adapter that resolves the current worker tree for each PII request.
    #[must_use]
    pub fn detector(
        &self,
    ) -> Arc<CurrentS1PiiDetector<polaris_platform::process::ProcessPlatform>> {
        Arc::clone(&self.detector)
    }

    /// Request orderly worker shutdown and join its process monitor.
    pub async fn shutdown(&self) -> Result<StopOutcome, KernelError> {
        stop_supervisor(
            &self.supervisor,
            &self.platform,
            &self.lifecycle,
            self.config.stop_grace,
        )
        .await
    }

    /// Terminate the current owned worker and wait until the supervisor publishes its replacement.
    ///
    /// This uses the retained process-tree handle, so it cannot signal a different process that
    /// later reuses the worker PID. The replacement is returned only after bootstrap and
    /// authenticated readiness complete.
    ///
    /// # Errors
    /// Returns a sanitized lifecycle error when there is no active generation, process termination
    /// is not confirmed, or the replacement does not become ready before the startup deadline.
    pub async fn restart_worker(&self) -> Result<polaris_sdk::registry::ProcessTree, KernelError> {
        let retired = self
            .lifecycle
            .active_tree()
            .map_err(|_| KernelError::Runtime("S1 worker generation state unavailable".to_owned()))?
            .ok_or_else(|| KernelError::Runtime("S1 worker is not active".to_owned()))?;
        match self.platform.kill_tree(&retired) {
            Ok(()) | Err(PlatformError::NotFound(_)) | Err(PlatformError::Io(_)) => {}
            Err(_) => {
                return Err(KernelError::Runtime(
                    "S1 worker termination was not confirmed".to_owned(),
                ));
            }
        }
        tokio::time::timeout(self.config.startup_timeout, async {
            loop {
                if let Some(current) = self
                    .lifecycle
                    .connection()
                    .map_err(|_| {
                        KernelError::Runtime("S1 worker generation state unavailable".to_owned())
                    })?
                    .map(|worker| worker.tree())
                    && current != retired
                {
                    return Ok(current);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| KernelError::Runtime("S1 worker restart timed out".to_owned()))?
    }

    /// Current authenticated process tree, if the worker is ready.
    pub fn current_tree(&self) -> Result<Option<polaris_sdk::registry::ProcessTree>, TaskError> {
        Ok(self.lifecycle.connection()?.map(|worker| worker.tree()))
    }

    /// Drain/evict the actual native owner and hold admission closed through heavy host work.
    /// # Errors
    /// Pause ownership, deadline, worker generation or native acknowledgement failure.
    pub async fn pause_models(
        &self,
    ) -> Result<crate::S1ModelPause<polaris_platform::process::ProcessPlatform>, KernelError> {
        self.lifecycle
            .pause_models(self.config.model_pause_timeout)
            .await
            .map_err(|_| {
                KernelError::Runtime("System One model pause was not acknowledged".to_owned())
            })
    }

    /// Recover abandoned model admission only when no heavy lease is alive.
    /// # Errors
    /// An active owner, unavailable worker, stale epoch or unconfirmed native resume.
    pub async fn recover_model_admission(&self) -> Result<(), KernelError> {
        self.lifecycle
            .recover_model_admission(self.config.model_pause_timeout)
            .await
            .map_err(|_| {
                KernelError::Runtime("System One model admission recovery failed".to_owned())
            })
    }
}

async fn stop_supervisor<S: EventStore + 'static>(
    supervisor: &Supervisor<S>,
    platform: &polaris_platform::process::ProcessPlatform,
    lifecycle: &S1WorkerLifecycle<polaris_platform::process::ProcessPlatform>,
    grace: Duration,
) -> Result<StopOutcome, KernelError> {
    let tree = lifecycle
        .active_tree()
        .map_err(|_| KernelError::Runtime("S1 worker generation state unavailable".to_owned()))?;
    let outcome = supervisor.shutdown(grace).await;
    if let Some(tree) = tree {
        match platform.try_wait_tree(&tree).await {
            Ok(Some(_)) | Err(PlatformError::NotFound(_)) => {}
            Ok(None) | Err(_) => {
                return Err(KernelError::Runtime(
                    "S1 worker termination was not confirmed".to_owned(),
                ));
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::PathBuf;
    use std::{error::Error, fs};

    use super::{LocalS1Config, S1Normalization};
    use polaris_sdk::{ids::SessionId, registry::SpawnSpec};

    fn configured_profile() -> Result<(tempfile::TempDir, LocalS1Config), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let worker = root.path().join("polaris-worker.exe");
        let runtime = root.path().join("onnxruntime.dll");
        let model = root.path().join("model");
        fs::create_dir(&model)?;
        fs::write(&worker, b"worker fixture")?;
        fs::write(&runtime, b"runtime fixture")?;
        #[cfg(unix)]
        let endpoint = root.path().join("s1-worker.sock");
        #[cfg(windows)]
        let endpoint = PathBuf::from(r"\\.\pipe\polaris-host-test");
        let profile = LocalS1Config::new(worker, model, runtime, endpoint);
        Ok((root, profile))
    }

    fn process_spec(profile: &LocalS1Config) -> Result<SpawnSpec, Box<dyn Error>> {
        Ok(profile.worker_spec(SessionId::from_u128(7))?.process)
    }

    #[test]
    fn configured_worker_has_immutable_paths_and_no_bootstrap_material()
    -> Result<(), Box<dyn Error>> {
        let (_root, mut profile) = configured_profile()?;
        profile.normalization = S1Normalization::Gliner2Python21;
        profile.validate()?;
        let spec = process_spec(&profile)?;
        assert_eq!(
            spec.program, profile.worker_executable,
            "worker executable must remain configured"
        );
        assert_eq!(
            spec.cwd.as_ref(),
            Some(&profile.model_dir),
            "worker cwd must be the configured model directory"
        );
        assert_eq!(
            spec.env,
            [(
                std::ffi::OsString::from("GLINER2_DEVICE"),
                std::ffi::OsString::from("cpu")
            )],
            "worker receives only the fixed CPU device pin, never ambient environment"
        );
        assert!(
            spec.args
                .iter()
                .all(|argument| !argument.to_string_lossy().contains("secret")),
            "bootstrap material must never appear in process arguments"
        );
        assert!(
            spec.args
                .iter()
                .any(|argument| argument == "gliner2-python-2.1"),
            "the configured normalization contract must reach the worker"
        );
        Ok(())
    }

    #[test]
    fn invalid_paths_and_unbounded_deadlines_are_rejected_before_spawn()
    -> Result<(), Box<dyn Error>> {
        let (_root, mut profile) = configured_profile()?;
        profile.worker_executable = profile.worker_executable.with_file_name("missing-worker");
        assert!(
            profile.validate().is_err(),
            "missing executable must reject the profile"
        );
        let (_root, mut profile) = configured_profile()?;
        profile.startup_timeout = std::time::Duration::from_secs(121);
        assert!(
            profile.validate().is_err(),
            "startup timeout above the hard cap must reject the profile"
        );
        let (_root, mut profile) = configured_profile()?;
        profile.idle_ttl = std::time::Duration::from_micros(500);
        assert!(
            profile.validate().is_err(),
            "submillisecond idle TTL must not be rounded to a zero worker argument"
        );
        let (_root, mut profile) = configured_profile()?;
        profile.poll_interval = std::time::Duration::MAX;
        assert!(
            profile.validate().is_err(),
            "unbounded process polling interval must be rejected before Tokio scheduling"
        );
        Ok(())
    }
}
