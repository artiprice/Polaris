// Unit tests of crates/polaris-openshell/crates/policy/src/implementation.rs (`tests`); compiled into that module via #[path].
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
fn canonical_yaml_round_trips_and_network_delta_rejects_other_sections()
-> Result<(), Box<dyn std::error::Error>> {
    let policy = policy_for(AgentRole::Builder, ExecTier::Docker)?;
    let yaml = policy.canonical_yaml()?;
    let round_trip = Policy::parse_yaml(&yaml)?;
    assert_eq!(round_trip, policy);
    assert!(network_only_delta("version: 1\nnetwork_policies: {}\n").is_ok());
    assert_eq!(
        network_only_delta("version: 1\nprocess: {}\n").err(),
        Some(PolicyError::NonNetworkDelta)
    );
    Ok(())
}

#[test]
fn provider_profile_is_exact_host_and_secret_free() -> Result<(), Box<dyn std::error::Error>> {
    let profile =
        provider_profile_token_factory("api.tokenfactory.nebius.com", "/usr/bin/polaris-guest")?;
    let yaml = provider_profile_yaml(&profile)?;
    assert!(yaml.contains("api.tokenfactory.nebius.com"));
    assert!(provider_profile_token_factory("*.nebius.com", "/usr/bin/polaris-guest").is_err());
    Ok(())
}

#[test]
fn provider_profile_serializer_rejects_publicly_constructed_wildcard_hosts()
-> Result<(), Box<dyn std::error::Error>> {
    let profile =
        provider_profile_token_factory("api.tokenfactory.nebius.com", "/usr/bin/polaris-guest")?;
    let mut profile = profile;
    let endpoint = profile
        .endpoints
        .first_mut()
        .ok_or("provider profile should contain an endpoint")?;
    endpoint.host = "*.nebius.com".to_owned();
    assert!(matches!(
        provider_profile_yaml(&profile),
        Err(PolicyError::InvalidHost(_))
    ));
    Ok(())
}
