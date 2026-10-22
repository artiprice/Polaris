//! OpenShell v0.1.4 policy adapter and Polaris policy derivation.

use openshell_policy_schema::{
    ParseLimits, PolicyDocument, parse_policy_with_limits, serialize_policy,
};
use polaris_sdk::registry::SandboxError;
use serde::{Deserialize, Serialize};

/// Maximum encoded policy size accepted by Polaris.
pub const MAX_POLICY_BYTES: usize = 256 * 1024;
/// OpenShell policy filesystem path cap from its v0.1.4 policy contract.
pub const MAX_FILESYSTEM_PATHS: usize = 256;

/// OpenShell policy document validated by the pinned upstream schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy(PolicyDocument);

impl Policy {
    /// Parse an upstream-schema-valid policy with bounded YAML expansion.
    pub fn parse_yaml(source: &str) -> Result<Self, PolicyError> {
        if source.len() > MAX_POLICY_BYTES {
            return Err(PolicyError::TooLarge);
        }
        let limits = ParseLimits {
            max_bytes: MAX_POLICY_BYTES,
            max_depth: 32,
            max_events: 100_000,
            max_nodes: 50_000,
            max_scalar_bytes: MAX_POLICY_BYTES,
            max_alias_expansions: 20,
            max_mapping_keys: 4_096,
            max_sequence_elements: 4_096,
            max_documents: 1,
            max_merge_keys: 0,
            alias_anchor_ratio: Some(2.0),
        };
        let document = parse_policy_with_limits(source, limits)
            .map_err(|error| PolicyError::Schema(format!("{error:?}")))?;
        let policy = Self(document);
        policy.validate()?;
        Ok(policy)
    }

    /// Serialize with OpenShell's deterministic canonical YAML emitter.
    pub fn canonical_yaml(&self) -> Result<String, PolicyError> {
        serialize_policy(&self.0).map_err(|error| PolicyError::Schema(error.to_string()))
    }

    /// Return the validated upstream policy document.
    pub fn document(&self) -> &PolicyDocument {
        &self.0
    }

    /// Enforce Polaris path bounds in addition to the upstream schema.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if let Some(filesystem) = &self.0.filesystem_policy {
            let count = filesystem
                .read_only
                .len()
                .checked_add(filesystem.read_write.len())
                .ok_or(PolicyError::TooManyPaths)?;
            if count > MAX_FILESYSTEM_PATHS {
                return Err(PolicyError::TooManyPaths);
            }
            for path in filesystem.read_only.iter().chain(&filesystem.read_write) {
                if !path.starts_with('/') || path.split('/').any(|part| part == "..") {
                    return Err(PolicyError::InvalidPath(path.clone()));
                }
            }
            if filesystem.read_write.iter().any(|path| path == "/") {
                return Err(PolicyError::BareRootWrite);
            }
        }
        Ok(())
    }
}

/// Execution gateway class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecTier {
    /// Docker driver in local WSL2.
    Docker,
    /// OpenShell vm driver.
    Vm,
    /// Remote OpenShell gateway.
    Remote,
    /// OpenShell MXC driver, which omits process policy and is not available on Windows.
    Mxc,
}

/// Polaris agent role used to derive its policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// Read and inspect files without network egress.
    Reader,
    /// Build Rust and fetch only crates.io artifacts.
    Builder,
    /// Run tests and fetch only Python package artifacts.
    Tester,
    /// Browse public web pages using Chromium.
    Browser,
    /// Adversarial workload with no network egress.
    Adversary,
}

/// Derive a least-privilege policy using actual OpenShell v0.1.4 fields.
pub fn policy_for(role: AgentRole, tier: ExecTier) -> Result<Policy, PolicyError> {
    let network = match role {
        AgentRole::Reader | AgentRole::Adversary => "",
        AgentRole::Builder => BUILDER_NETWORK,
        AgentRole::Tester => TESTER_NETWORK,
        AgentRole::Browser => BROWSER_NETWORK,
    };
    let process = if tier == ExecTier::Mxc {
        ""
    } else {
        "process: {run_as_user: sandbox, run_as_group: sandbox}\n"
    };
    let network_section = if network.is_empty() {
        "network_policies: {}\n".to_owned()
    } else {
        format!("network_policies:\n{network}")
    };
    let yaml = format!(
        "version: 1\nfilesystem_policy:\n  include_workdir: true\n  read_only: [/bin, /usr, /lib, /proc, /dev/urandom, /app, /etc, /var/log]\n  read_write: [/sandbox, /tmp, /dev/null]\nlandlock: {{compatibility: best_effort}}\n{process}{network_section}"
    );
    Policy::parse_yaml(&yaml)
}

const BUILDER_NETWORK: &str = "  crates_io:\n    name: crates-io-readonly\n    endpoints:\n      - {host: index.crates.io, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n      - {host: static.crates.io, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n    binaries:\n      - {path: /usr/local/cargo/bin/cargo}\n";
const TESTER_NETWORK: &str = "  pypi:\n    name: pypi-readonly\n    endpoints:\n      - {host: pypi.org, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n      - {host: files.pythonhosted.org, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n    binaries:\n      - {path: /usr/bin/python3}\n";
const BROWSER_NETWORK: &str = "  browser_web:\n    name: browser-web-readonly\n    endpoints:\n      - {host: docs.nvidia.com, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n      - {host: docs.rs, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n      - {host: github.com, port: 443, protocol: rest, enforcement: enforce, access: read-only}\n    binaries:\n      - {path: /usr/bin/chromium}\n";

/// Credential-free OpenShell provider profile descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProfile {
    /// Gateway profile identifier.
    pub id: String,
    /// Human-readable profile name.
    pub display_name: String,
    /// Profile purpose.
    pub description: String,
    /// OpenShell provider category.
    pub category: String,
    /// Whether this profile provides model inference.
    pub inference_capable: bool,
    /// Secret name references only; this type intentionally has no value field.
    pub credentials: Vec<CredentialSlot>,
    /// Authorized inference destinations.
    pub endpoints: Vec<ProviderEndpoint>,
    /// Authorized binaries.
    pub binaries: Vec<String>,
}

/// Gateway-held credential reference, never the credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSlot {
    /// Profile credential identifier.
    pub name: String,
    /// Human-readable credential purpose.
    pub description: String,
    /// Environment variable name supplied to the gateway's proxy.
    pub env_vars: Vec<String>,
    /// Whether the profile requires this credential.
    pub required: bool,
    /// OpenShell auth style.
    pub auth_style: String,
    /// HTTP authorization header name.
    pub header_name: String,
}

/// Provider endpoint in OpenShell's profile schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEndpoint {
    /// Exact DNS hostname.
    pub host: String,
    /// HTTPS port.
    pub port: u16,
    /// OpenShell endpoint protocol.
    pub protocol: String,
    /// OpenShell endpoint access mode.
    pub access: String,
    /// OpenShell policy enforcement mode.
    pub enforcement: String,
}

/// Build the Polaris Token Factory profile; the API key is configured at the gateway out of band.
pub fn provider_profile_token_factory(
    host: &str,
    model_binary: &str,
) -> Result<ProviderProfile, PolicyError> {
    let host = host.trim();
    if host.is_empty() || host.contains(['*', '/', ':']) || host.chars().any(char::is_whitespace) {
        return Err(PolicyError::InvalidHost(host.to_owned()));
    }
    if !model_binary.starts_with('/') || model_binary.split('/').any(|part| part == "..") {
        return Err(PolicyError::InvalidPath(model_binary.to_owned()));
    }
    Ok(ProviderProfile {
        id: "nebius-token-factory".into(),
        display_name: "Nebius Token Factory".into(),
        description: "Token Factory OpenAI-compatible inference; credentials are substituted by the OpenShell gateway.".into(),
        category: "inference".into(),
        inference_capable: true,
        credentials: vec![CredentialSlot {
            name: "api_key".into(),
            description: "Nebius Token Factory API key".into(),
            env_vars: vec!["NEBIUS_API_KEY".into()],
            required: true,
            auth_style: "bearer".into(),
            header_name: "authorization".into(),
        }],
        endpoints: vec![ProviderEndpoint {
            host: host.to_owned(),
            port: 443,
            protocol: "rest".into(),
            access: "read-write".into(),
            enforcement: "enforce".into(),
        }],
        binaries: vec![model_binary.to_owned()],
    })
}

/// Serialize a provider profile as YAML for `openshell provider profile import`.
pub fn provider_profile_yaml(profile: &ProviderProfile) -> Result<String, PolicyError> {
    serde_saphyr::to_string(profile).map_err(|error| PolicyError::Yaml(error.to_string()))
}

/// Validate a policy update as a network-only hot-reload delta.
pub fn network_only_delta(source: &str) -> Result<String, PolicyError> {
    let policy = Policy::parse_yaml(source)?;
    if policy.0.filesystem_policy.is_some()
        || policy.0.landlock.is_some()
        || policy.0.process.is_some()
    {
        return Err(PolicyError::NonNetworkDelta);
    }
    policy.canonical_yaml()
}

/// Policy parsing and validation failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Policy document is larger than the configured input bound.
    #[error("policy exceeds the {MAX_POLICY_BYTES} byte limit")]
    TooLarge,
    /// The pinned OpenShell parser or schema rejected the document.
    #[error("OpenShell policy schema: {0}")]
    Schema(String),
    /// Filesystem path is not absolute or contains traversal.
    #[error("invalid filesystem path: {0}")]
    InvalidPath(String),
    /// Root filesystem writes are forbidden.
    #[error("filesystem policy cannot grant write access to /")]
    BareRootWrite,
    /// Filesystem allow-list exceeds the documented path limit.
    #[error("filesystem policy exceeds {MAX_FILESYSTEM_PATHS} paths")]
    TooManyPaths,
    /// Network hot reload may not mutate other policy sections.
    #[error("hot reload may change only network_policies")]
    NonNetworkDelta,
    /// Provider host must be an exact DNS name.
    #[error("invalid provider host: {0}")]
    InvalidHost(String),
    /// Provider YAML serialization failed.
    #[error("YAML serialization: {0}")]
    Yaml(String),
}

impl From<PolicyError> for SandboxError {
    fn from(value: PolicyError) -> Self {
        Self::PolicyDenied(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_and_tier_produces_valid_upstream_policy() {
        for role in [
            AgentRole::Reader,
            AgentRole::Builder,
            AgentRole::Tester,
            AgentRole::Browser,
            AgentRole::Adversary,
        ] {
            for tier in [
                ExecTier::Docker,
                ExecTier::Vm,
                ExecTier::Remote,
                ExecTier::Mxc,
            ] {
                let policy = policy_for(role, tier);
                assert!(policy.is_ok(), "role {role:?}, tier {tier:?}: {policy:?}");
                if role == AgentRole::Adversary {
                    assert!(
                        policy
                            .map(|p| p.document().network_policies.is_empty())
                            .unwrap_or(false)
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_unknown_fields_root_write_and_relative_paths() {
        assert!(Policy::parse_yaml("version: 1\nfuture: true\n").is_err());
        assert!(matches!(
            Policy::parse_yaml("version: 1\nfilesystem_policy: {read_write: [/]}\n"),
            Err(PolicyError::BareRootWrite)
        ));
        assert!(matches!(
            Policy::parse_yaml("version: 1\nfilesystem_policy: {read_only: [relative]}\n"),
            Err(PolicyError::InvalidPath(_))
        ));
    }

    #[test]
    fn canonical_yaml_round_trips_and_network_delta_rejects_other_sections() {
        let Some(policy) = policy_for(AgentRole::Builder, ExecTier::Docker).ok() else {
            panic!("valid policy expected")
        };
        let Some(yaml) = policy.canonical_yaml().ok() else {
            panic!("canonical serialization expected")
        };
        let Some(round_trip) = Policy::parse_yaml(&yaml).ok() else {
            panic!("canonical policy must parse")
        };
        assert_eq!(round_trip, policy);
        assert!(network_only_delta("version: 1\nnetwork_policies: {}\n").is_ok());
        assert_eq!(
            network_only_delta("version: 1\nprocess: {}\n").err(),
            Some(PolicyError::NonNetworkDelta)
        );
    }

    #[test]
    fn provider_profile_is_exact_host_and_secret_free() {
        let profile =
            provider_profile_token_factory("api.tokenfactory.nebius.com", "/usr/bin/polaris-guest");
        assert!(profile.is_ok());
        let Some(profile) = profile.ok() else { return };
        let yaml = provider_profile_yaml(&profile);
        assert!(yaml.is_ok());
        assert!(
            yaml.map(|value| value.contains("api.tokenfactory.nebius.com"))
                .unwrap_or(false)
        );
        assert!(provider_profile_token_factory("*.nebius.com", "/usr/bin/polaris-guest").is_err());
    }
}
