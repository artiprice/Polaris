//! Versioned Polaris sandbox blueprint based on the NemoClaw shape, applied through OpenShell.

use std::time::Duration;

use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec};
use polaris_openshell_client::{OpenShellBackend, PolicySource};
use serde::{Deserialize, Serialize};

/// Current blueprint schema version.
pub const BLUEPRINT_VERSION: u32 = 1;
/// OpenShell version pinned by the P2 environment.
pub const OPEN_SHELL_VERSION: &str = "0.1.4";

/// Versioned Polaris sandbox blueprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    /// Blueprint document version.
    pub version: u32,
    /// OpenShell semantic version constraint, pinned exactly for P2.
    pub min_openshell_version: String,
    /// OpenShell upper version bound, pinned exactly for P2.
    pub max_openshell_version: String,
    /// Optional content digest; computed with this field omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Sandbox workload and execution policy references.
    pub components: BlueprintComponents,
    /// Required Polaris guest handshake protocol version.
    pub guest_protocol_version: u32,
}

/// Components selected by the blueprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlueprintComponents {
    /// Sandbox runtime settings.
    pub sandbox: SandboxShape,
    /// Named OpenShell policy documents referenced by CAS CID.
    pub policies: Vec<PolicyRef>,
    /// OpenShell provider profile names to attach when the workload needs inference.
    #[serde(default)]
    pub inference_profiles: Vec<String>,
}

/// Sandbox image, driver, work directory and resource requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxShape {
    /// Immutable OCI image reference.
    pub image: String,
    /// OpenShell driver hint.
    pub driver: String,
    /// Absolute in-sandbox working directory.
    pub workdir: String,
    /// Optional CPU limit in millicores.
    pub cpu_millis: Option<u32>,
    /// Optional memory limit in MiB.
    pub memory_mib: Option<u32>,
}

/// Content-addressed sandbox policy reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRef {
    /// BLAKE3 CID in lowercase hexadecimal.
    pub cid: String,
}

/// Request fields for constructing the default blueprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintConfig {
    /// Sandbox OCI image reference including a sha256 digest.
    pub image: String,
    /// Policy CID to apply at creation.
    pub policy_cid: Cid,
    /// Work directory in the sandbox.
    pub workdir: String,
    /// Protocol version expected from `/usr/bin/polaris-guest`.
    pub guest_protocol_version: u32,
}

/// BLAKE3 digest of a canonical blueprint without the digest field populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlueprintDigest([u8; 32]);

impl BlueprintDigest {
    /// Construct from the raw BLAKE3 digest.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self { Self(bytes) }
    /// Raw digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] { &self.0 }
    /// Lowercase hexadecimal digest text.
    pub fn to_hex(self) -> String { blake3::Hash::from(self.0).to_hex().to_string() }
}

/// Construct the P2 default blueprint from explicit image and policy pins.
pub fn blueprint_default(config: &BlueprintConfig) -> Result<Blueprint, NemoclawError> {
    validate_image(&config.image)?;
    validate_workdir(&config.workdir)?;
    let mut blueprint = Blueprint {
        version: BLUEPRINT_VERSION,
        min_openshell_version: OPEN_SHELL_VERSION.to_owned(),
        max_openshell_version: OPEN_SHELL_VERSION.to_owned(),
        digest: None,
        components: BlueprintComponents {
            sandbox: SandboxShape {
                image: config.image.clone(),
                driver: "docker".to_owned(),
                workdir: config.workdir.clone(),
                cpu_millis: None,
                memory_mib: None,
            },
            policies: vec![PolicyRef { cid: config.policy_cid.to_hex() }],
            inference_profiles: Vec::new(),
        },
        guest_protocol_version: config.guest_protocol_version,
    };
    let digest = blueprint_digest(&blueprint)?;
    blueprint.digest = Some(digest.to_hex());
    Ok(blueprint)
}

/// Parse and validate the native Polaris blueprint format.
pub fn parse_blueprint(yaml: &str) -> Result<Blueprint, NemoclawError> {
    if yaml.len() > MAX_BLUEPRINT_BYTES { return Err(NemoclawError::TooLarge); }
    let mut blueprint: Blueprint = serde_saphyr::from_str(yaml).map_err(|error| NemoclawError::Yaml(error.to_string()))?;
    validate_blueprint(&blueprint)?;
    let supplied = blueprint.digest.clone().ok_or(NemoclawError::MissingDigest)?;
    let computed = blueprint_digest(&blueprint)?.to_hex();
    if supplied != computed { return Err(NemoclawError::DigestMismatch { expected: computed, found: supplied }); }
    blueprint.digest = Some(supplied);
    Ok(blueprint)
}

/// Serialize blueprint in stable struct field order and include its verified digest.
pub fn to_yaml(blueprint: &Blueprint) -> Result<String, NemoclawError> {
    validate_blueprint(blueprint)?;
    let digest = blueprint_digest(blueprint)?.to_hex();
    if let Some(supplied) = &blueprint.digest {
        if supplied != &digest { return Err(NemoclawError::DigestMismatch { expected: digest, found: supplied.clone() }); }
    }
    let mut output = blueprint.clone();
    output.digest = Some(digest);
    serde_saphyr::to_string(&output).map_err(|error| NemoclawError::Yaml(error.to_string()))
}

/// Compute the canonical blueprint digest with the digest field excluded.
pub fn blueprint_digest(blueprint: &Blueprint) -> Result<BlueprintDigest, NemoclawError> {
    let mut canonical = blueprint.clone();
    canonical.digest = None;
    let yaml = serde_saphyr::to_string(&canonical).map_err(|error| NemoclawError::Yaml(error.to_string()))?;
    Ok(BlueprintDigest(*blake3::hash(yaml.as_bytes()).as_bytes()))
}

/// Check the stored digest against the canonical content.
pub fn verify(blueprint: &Blueprint) -> Result<BlueprintDigest, NemoclawError> {
    validate_blueprint(blueprint)?;
    let expected = blueprint_digest(blueprint)?;
    let Some(found) = blueprint.digest.as_deref() else { return Err(NemoclawError::MissingDigest) };
    if found != expected.to_hex() { return Err(NemoclawError::DigestMismatch { expected: expected.to_hex(), found: found.to_owned() }); }
    Ok(expected)
}

/// Host facts consumed by the pure onboarding planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostFacts {
    /// WSL2 distro is installed.
    pub wsl2_installed: bool,
    /// Docker Engine is active in WSL2.
    pub docker_active: bool,
    /// OpenShell gateway is installed at the pinned version.
    pub openshell_installed: bool,
    /// Polaris sandbox image is locally available.
    pub image_available: bool,
}

/// Onboarding step with bounded action timeout and recovery guidance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardStep {
    /// Action identifier consumed by the host composition layer.
    pub action: OnboardAction,
    /// Maximum time the host action may run.
    pub timeout: Duration,
    /// Recovery instruction if execution fails.
    pub undo: String,
}

/// Supported host actions in the OpenShell onboarding plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardAction { CheckWsl2, InstallOpenShell, ConfigureDockerDriver, BuildSandboxImage, StartGateway }

/// Produce ordered idempotent onboarding actions from observed host facts.
pub fn plan_onboard(_blueprint: &Blueprint, facts: HostFacts) -> Vec<OnboardStep> {
    let mut steps = Vec::new();
    if !facts.wsl2_installed { steps.push(step(OnboardAction::CheckWsl2, 30, "Install Ubuntu 24.04 with the workspace setup script.")); }
    if !facts.openshell_installed { steps.push(step(OnboardAction::InstallOpenShell, 180, "Rerun the pinned OpenShell installer after correcting network or package errors.")); }
    if !facts.docker_active { steps.push(step(OnboardAction::ConfigureDockerDriver, 90, "Start Docker Engine and confirm the openshell service environment pins driver=docker.")); }
    if !_blueprint.components.sandbox.image.is_empty() && !facts.image_available { steps.push(step(OnboardAction::BuildSandboxImage, 1_800, "Inspect the image build log, then rebuild the digest-pinned image.")); }
    if !facts.openshell_installed || !facts.docker_active { steps.push(step(OnboardAction::StartGateway, 30, "Fix the OpenShell/Docker prerequisite before retrying gateway startup.")); }
    steps
}

fn step(action: OnboardAction, seconds: u64, undo: &str) -> OnboardStep { OnboardStep { action, timeout: Duration::from_secs(seconds), undo: undo.to_owned() } }

/// Status of one doctor probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus { Pass, Fail, Unknown }

/// Named machine prerequisite status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorItem {
    /// Probe name.
    pub name: String,
    /// Probe state.
    pub status: ProbeStatus,
    /// Evidence or remediation text.
    pub detail: String,
}

/// Results supplied by the actual host probe layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProbeResults {
    /// WSL2 facts.
    pub wsl2: Option<bool>,
    /// OpenShell version.
    pub openshell_version: Option<String>,
    /// Docker service state.
    pub docker_active: Option<bool>,
    /// KVM availability.
    pub kvm_available: Option<bool>,
    /// Image digest availability.
    pub image_digest: Option<String>,
    /// Credential presence in the host keyring (presence only).
    pub token_factory_key_present: Option<bool>,
}

/// Full doctor report; unknown remains explicit rather than treated as success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    /// Per-prerequisite outcomes.
    pub items: Vec<DoctorItem>,
}

/// Evaluate injected observations without accessing the host or printing secrets.
pub fn evaluate_doctor(blueprint: &Blueprint, probes: &ProbeResults) -> DoctorReport {
    let version_ok = probes.openshell_version.as_deref() == Some(OPEN_SHELL_VERSION);
    let image_ok = probes.image_digest.as_deref().is_some_and(|digest| blueprint.components.sandbox.image.ends_with(digest));
    DoctorReport { items: vec![
        item("wsl2", probes.wsl2.map(status), "WSL2 Ubuntu 24.04"),
        item("openshell", probes.openshell_version.as_ref().map(|value| if value == OPEN_SHELL_VERSION { ProbeStatus::Pass } else { ProbeStatus::Fail }), "OpenShell must be 0.1.4"),
        item("docker", probes.docker_active.map(status), "Docker Engine service"),
        item("kvm", probes.kvm_available.map(status), "KVM is optional for the docker driver and required for vm"),
        item("sandbox_image", probes.image_digest.as_ref().map(|_| if image_ok { ProbeStatus::Pass } else { ProbeStatus::Fail }), "Image digest must match the blueprint pin"),
        item("token_factory_key", probes.token_factory_key_present.map(status), "Keyring presence only; no credential value is read or displayed"),
        item("blueprint", Some(if verify(blueprint).is_ok() { ProbeStatus::Pass } else { ProbeStatus::Fail }), "Blueprint schema, image and policy pins"),
        item("openshell_version_gate", Some(if version_ok { ProbeStatus::Pass } else { ProbeStatus::Fail }), OPEN_SHELL_VERSION),
    ] }
}

fn status(value: bool) -> ProbeStatus { if value { ProbeStatus::Pass } else { ProbeStatus::Fail } }
fn item(name: &str, status: Option<ProbeStatus>, detail: &str) -> DoctorItem {
    let status = match status { Some(value) => value, None => ProbeStatus::Unknown };
    DoctorItem { name: name.to_owned(), status, detail: detail.to_owned() }
}

/// Apply a blueprint and confirm the baked guest protocol before returning a sandbox handle.
pub trait BlueprintBackend: SandboxBackend {
    /// Create a sandbox with explicit OpenShell provider profile names.
    fn create_with_profiles(&self, spec: &SandboxSpec, profiles: &[String]) -> impl std::future::Future<Output = Result<SandboxHandle, SandboxError>> + Send;
}

impl<P: PolicySource> BlueprintBackend for OpenShellBackend<P> {
    fn create_with_profiles(&self, spec: &SandboxSpec, profiles: &[String]) -> impl std::future::Future<Output = Result<SandboxHandle, SandboxError>> + Send {
        OpenShellBackend::create_with_profiles(self, spec, profiles)
    }
}

/// Apply a blueprint, attach its named profiles, and verify the baked guest protocol before returning.
pub async fn apply<B: BlueprintBackend>(backend: &B, blueprint: &Blueprint) -> Result<AppliedBlueprint, NemoclawError> {
    let digest = verify(blueprint)?;
    if blueprint.components.policies.len() != 1 { return Err(NemoclawError::PolicyCount(blueprint.components.policies.len())); }
    let policy_ref = blueprint.components.policies.first().ok_or(NemoclawError::PolicyCount(0))?;
    let policy = Cid::from_hex(&policy_ref.cid).map_err(|error| NemoclawError::Policy(error.to_string()))?;
    let sandbox = &blueprint.components.sandbox;
    let spec = SandboxSpec { image: Some(sandbox.image.clone()), cpu_millis: sandbox.cpu_millis,
        memory_mib: sandbox.memory_mib, policy: Some(policy), ..Default::default() };
    let handle = backend.create_with_profiles(&spec, &blueprint.components.inference_profiles).await.map_err(NemoclawError::Backend)?;
    let request = ExecRequest { argv: vec!["/usr/bin/polaris-guest".to_owned(), "--protocol-version".to_owned()],
        workdir: Some(sandbox.workdir.clone()), timeout: Some(Duration::from_secs(15)), ..Default::default() };
    let response = backend.exec(&handle, &request).await;
    let result = match response {
        Ok(output) if output.exit_code == 0 => {
            let text = std::str::from_utf8(&output.stdout).map_err(|error| NemoclawError::GuestHandshake(error.to_string()));
            match text { Ok(value) => match value.trim().parse::<u32>() { Ok(version) if version == blueprint.guest_protocol_version => Ok(version), Ok(version) => Err(NemoclawError::GuestProtocolMismatch { expected: blueprint.guest_protocol_version, found: version }), Err(error) => Err(NemoclawError::GuestHandshake(error.to_string())) }, Err(error) => Err(error) }
        }
        Ok(output) => Err(NemoclawError::GuestHandshake(format!("guest exited {}", output.exit_code))),
        Err(error) => Err(NemoclawError::Backend(error)),
    };
    let protocol_version = match result {
        Ok(version) => version,
        Err(error) => {
            let cleanup = backend.destroy(&handle).await;
            return match cleanup { Ok(()) => Err(error), Err(cleanup_error) => Err(NemoclawError::Cleanup { primary: error.to_string(), cleanup: cleanup_error.to_string() }) };
        }
    };
    Ok(AppliedBlueprint { sandbox: handle, digest, guest_protocol_version: protocol_version })
}

/// Blueprint application result with the sandbox and guest handshake evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedBlueprint {
    /// Created OpenShell sandbox.
    pub sandbox: SandboxHandle,
    /// Verified blueprint digest.
    pub digest: BlueprintDigest,
    /// Negotiated guest protocol version.
    pub guest_protocol_version: u32,
}

/// Generate a Docker-compatible sandbox image recipe that places the guest under `/usr/bin`.
pub fn dockerfile_for(blueprint: &Blueprint) -> Result<String, NemoclawError> {
    validate_blueprint(blueprint)?;
    let mut lines = vec![
        "FROM rust:1.99.0-bookworm AS polaris-guest-build".to_owned(),
        "WORKDIR /workspace".to_owned(),
        "COPY . /workspace".to_owned(),
        "RUN cargo build --locked --release -p polaris-guest".to_owned(),
        format!("FROM {}", blueprint.components.sandbox.image),
        "USER root".to_owned(),
        "COPY --from=polaris-guest-build /workspace/target/release/polaris-guest /usr/bin/polaris-guest".to_owned(),
        "RUN chmod 0755 /usr/bin/polaris-guest && useradd --create-home --uid 1000 sandbox".to_owned(),
        format!("WORKDIR {}", blueprint.components.sandbox.workdir),
        "USER sandbox".to_owned(),
        "ENTRYPOINT [\"sleep\", \"infinity\"]".to_owned(),
    ];
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Maximum YAML document accepted by the blueprint parser.
pub const MAX_BLUEPRINT_BYTES: usize = 128 * 1024;

fn validate_blueprint(blueprint: &Blueprint) -> Result<(), NemoclawError> {
    if blueprint.version != BLUEPRINT_VERSION { return Err(NemoclawError::UnsupportedVersion(blueprint.version)); }
    if blueprint.min_openshell_version != OPEN_SHELL_VERSION || blueprint.max_openshell_version != OPEN_SHELL_VERSION {
        return Err(NemoclawError::UnsupportedOpenShellVersion);
    }
    validate_image(&blueprint.components.sandbox.image)?;
    validate_workdir(&blueprint.components.sandbox.workdir)?;
    if blueprint.guest_protocol_version == 0 { return Err(NemoclawError::InvalidGuestProtocol); }
    if blueprint.components.policies.is_empty() { return Err(NemoclawError::PolicyCount(0)); }
    for policy in &blueprint.components.policies { Cid::from_hex(&policy.cid).map_err(|error| NemoclawError::Policy(error.to_string()))?; }
    for profile in &blueprint.components.inference_profiles {
        if profile.is_empty() || profile.contains('/') || profile.contains('*') { return Err(NemoclawError::InvalidProfile(profile.clone())); }
    }
    Ok(())
}

fn validate_image(image: &str) -> Result<(), NemoclawError> {
    let Some((_, digest)) = image.rsplit_once("@sha256:") else { return Err(NemoclawError::ImageNotPinned); };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) { return Err(NemoclawError::ImageNotPinned); }
    if image.chars().any(char::is_whitespace) { return Err(NemoclawError::ImageNotPinned); }
    Ok(())
}

fn validate_workdir(path: &str) -> Result<(), NemoclawError> {
    if !path.starts_with('/') || path.split('/').any(|component| component == "..") { return Err(NemoclawError::InvalidWorkdir(path.to_owned())); }
    Ok(())
}

/// Blueprint parse, planning and application failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NemoclawError {
    /// YAML failed to parse or serialize.
    #[error("blueprint YAML: {0}")]
    Yaml(String),
    /// Blueprint is larger than the parser bound.
    #[error("blueprint exceeds {MAX_BLUEPRINT_BYTES} bytes")]
    TooLarge,
    /// Blueprint schema version is unsupported.
    #[error("unsupported blueprint version {0}")]
    UnsupportedVersion(u32),
    /// OpenShell target version is not the pinned P2 version.
    #[error("blueprint must pin OpenShell {OPEN_SHELL_VERSION}")]
    UnsupportedOpenShellVersion,
    /// Image reference must carry an immutable sha256 digest.
    #[error("sandbox image must use @sha256:<64 hex digits>")]
    ImageNotPinned,
    /// Sandbox work directory is invalid.
    #[error("invalid sandbox work directory: {0}")]
    InvalidWorkdir(String),
    /// Blueprint contains no digest.
    #[error("blueprint digest is required")]
    MissingDigest,
    /// Canonical digest does not match.
    #[error("blueprint digest mismatch: expected {expected}, found {found}")]
    DigestMismatch { expected: String, found: String },
    /// Policy CID was invalid.
    #[error("invalid policy reference: {0}")]
    Policy(String),
    /// Exactly one policy is required by the SDK SandboxSpec contract.
    #[error("blueprint requires exactly one policy, got {0}")]
    PolicyCount(usize),
    /// Provider profile reference is invalid.
    #[error("invalid provider profile reference: {0}")]
    InvalidProfile(String),
    /// Guest protocol must be a positive version.
    #[error("guest protocol version must be positive")]
    InvalidGuestProtocol,
    /// Sandbox backend operation failed.
    #[error("OpenShell backend: {0}")]
    Backend(SandboxError),
    /// Guest handshake failed.
    #[error("polaris-guest protocol handshake failed: {0}")]
    GuestHandshake(String),
    /// Guest reports a protocol version other than requested.
    #[error("guest protocol mismatch: expected {expected}, found {found}")]
    GuestProtocolMismatch { expected: u32, found: u32 },
    /// Guest failure plus sandbox cleanup failure.
    #[error("{primary}; sandbox cleanup also failed: {cleanup}")]
    Cleanup { primary: String, cleanup: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blueprint() -> Option<Blueprint> {
        blueprint_default(&BlueprintConfig { image: format!("ghcr.io/polaris/sandbox@sha256:{}", "a".repeat(64)),
            policy_cid: Cid::from_bytes([9; 32]), workdir: "/sandbox".to_owned(), guest_protocol_version: 1 }).ok()
    }

    #[test]
    fn default_digest_round_trips_and_tampering_fails() {
        let Some(blueprint) = blueprint() else { panic!("valid blueprint expected") };
        let yaml = to_yaml(&blueprint);
        assert!(yaml.is_ok());
        let Some(yaml) = yaml.ok() else { return };
        let parsed = parse_blueprint(&yaml);
        assert_eq!(parsed.ok(), Some(blueprint.clone()));
        let mut tampered = blueprint;
        tampered.components.sandbox.workdir = "/tmp".to_owned();
        assert!(verify(&tampered).is_err());
    }

    #[test]
    fn mutable_image_and_bad_cid_are_rejected() {
        let result = blueprint_default(&BlueprintConfig { image: "ubuntu:latest".to_owned(), policy_cid: Cid::from_bytes([1; 32]), workdir: "/sandbox".to_owned(), guest_protocol_version: 1 });
        assert!(matches!(result, Err(NemoclawError::ImageNotPinned)));
        let mut blueprint = blueprint();
        if let Some(value) = &mut blueprint { value.components.policies[0].cid = "bad".to_owned(); }
        assert!(blueprint.as_ref().is_some_and(|value| validate_blueprint(value).is_err()));
    }

    #[test]
    fn onboarding_and_doctor_keep_unknown_state_explicit() {
        let Some(blueprint) = blueprint() else { panic!("valid blueprint expected") };
        let steps = plan_onboard(&blueprint, HostFacts::default());
        assert!(steps.iter().any(|step| step.action == OnboardAction::InstallOpenShell));
        let report = evaluate_doctor(&blueprint, &ProbeResults::default());
        assert!(report.items.iter().filter(|entry| ["wsl2", "openshell", "docker", "kvm", "sandbox_image", "token_factory_key"].contains(&entry.name.as_str())).all(|entry| entry.status == ProbeStatus::Unknown));
    }

    #[test]
    fn dockerfile_places_guest_in_policy_visible_usr_bin() {
        let Some(blueprint) = blueprint() else { panic!("valid blueprint expected") };
        let recipe = dockerfile_for(&blueprint);
        assert!(recipe.is_ok());
        assert!(recipe.map(|value| value.contains("/usr/bin/polaris-guest")).unwrap_or(false));
    }
}
