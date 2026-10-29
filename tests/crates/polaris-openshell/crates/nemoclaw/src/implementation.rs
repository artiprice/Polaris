// Unit tests of crates/polaris-openshell/crates/nemoclaw/src/implementation.rs (`tests`); compiled into that module via #[path].
use super::*;

fn blueprint() -> Result<Blueprint, NemoclawError> {
    blueprint_default(&BlueprintConfig {
        image: format!("ghcr.io/polaris/sandbox@sha256:{}", "a".repeat(64)),
        policy_cid: Cid::from_bytes([9; 32]),
        workdir: "/sandbox".to_owned(),
        guest_protocol_version: 1,
    })
}

#[test]
fn default_digest_round_trips_and_tampering_fails() -> Result<(), Box<dyn std::error::Error>> {
    let blueprint = blueprint()?;
    let yaml = to_yaml(&blueprint)?;
    let parsed = parse_blueprint(&yaml)?;
    assert_eq!(parsed, blueprint);
    let mut tampered = blueprint;
    tampered.components.sandbox.workdir = "/tmp".to_owned();
    assert!(verify(&tampered).is_err());
    Ok(())
}

#[test]
fn mutable_image_and_bad_cid_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let result = blueprint_default(&BlueprintConfig {
        image: "ubuntu:latest".to_owned(),
        policy_cid: Cid::from_bytes([1; 32]),
        workdir: "/sandbox".to_owned(),
        guest_protocol_version: 1,
    });
    assert!(matches!(result, Err(NemoclawError::ImageNotPinned)));
    let mut blueprint = blueprint()?;
    let policy = blueprint
        .components
        .policies
        .first_mut()
        .ok_or("default blueprint must contain its policy")?;
    policy.cid = "bad".to_owned();
    assert!(validate_blueprint(&blueprint).is_err());
    Ok(())
}

#[test]
fn blueprint_driver_is_exact_and_must_match_authenticated_backend()
-> Result<(), Box<dyn std::error::Error>> {
    for driver in ["Docker", "DOCKER", "docker ", "containerd", ""] {
        let mut blueprint = blueprint()?;
        blueprint.components.sandbox.driver = driver.to_owned();
        assert!(matches!(
            validate_blueprint(&blueprint),
            Err(NemoclawError::UnsupportedDriver(found)) if found == driver
        ));
    }

    let blueprint = blueprint()?;
    assert!(matches!(
        validate_backend_driver(&blueprint, DriverKind::Vm),
        Err(NemoclawError::DriverMismatch { requested, actual })
            if requested == "docker" && actual == "vm"
    ));
    assert!(validate_backend_driver(&blueprint, DriverKind::Docker).is_ok());
    let docker_doctor = evaluate_doctor(
        &blueprint,
        &ProbeResults {
            kvm_available: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(
        docker_doctor
            .items
            .iter()
            .find(|entry| entry.name == "kvm")
            .map(|entry| entry.status),
        Some(ProbeStatus::Pass)
    );

    let mut vm_blueprint = blueprint;
    vm_blueprint.components.sandbox.driver = "vm".to_owned();
    vm_blueprint.digest = Some(blueprint_digest(&vm_blueprint)?.to_hex());
    assert!(validate_backend_driver(&vm_blueprint, DriverKind::Vm).is_ok());
    assert!(matches!(
        validate_backend_driver(&vm_blueprint, DriverKind::Docker),
        Err(NemoclawError::DriverMismatch { requested, actual })
            if requested == "vm" && actual == "docker"
    ));
    let vm_doctor = evaluate_doctor(
        &vm_blueprint,
        &ProbeResults {
            kvm_available: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(
        vm_doctor
            .items
            .iter()
            .find(|entry| entry.name == "kvm")
            .map(|entry| entry.status),
        Some(ProbeStatus::Fail)
    );
    Ok(())
}

#[test]
fn onboarding_steps_follow_the_selected_driver_and_have_checks()
-> Result<(), Box<dyn std::error::Error>> {
    let mut vm_blueprint = blueprint()?;
    vm_blueprint.components.sandbox.driver = "vm".to_owned();
    vm_blueprint.digest = Some(blueprint_digest(&vm_blueprint)?.to_hex());

    let blocked_vm = plan_onboard(&vm_blueprint, HostFacts::default())?;
    assert!(
        blocked_vm
            .iter()
            .any(|step| step.action == OnboardAction::CheckKvm)
    );
    assert!(
        !blocked_vm
            .iter()
            .any(|step| step.action == OnboardAction::ConfigureDockerDriver)
    );
    assert!(
        !blocked_vm
            .iter()
            .any(|step| step.action == OnboardAction::StartGateway)
    );

    let ready_vm_facts = HostFacts {
        wsl2_installed: true,
        docker_active: false,
        kvm_available: true,
        vm_driver_active: false,
        openshell_installed: true,
        image_available: true,
    };
    let vm_steps = plan_onboard(&vm_blueprint, ready_vm_facts)?;
    assert!(
        vm_steps
            .iter()
            .any(|step| step.action == OnboardAction::ConfigureVmDriver)
    );
    assert!(
        vm_steps
            .iter()
            .any(|step| step.action == OnboardAction::StartGateway)
    );
    assert!(
        !vm_steps
            .iter()
            .any(|step| step.action == OnboardAction::ConfigureDockerDriver)
    );
    assert!(vm_steps.iter().all(|step| step.timeout > Duration::ZERO));

    let docker_facts = HostFacts {
        wsl2_installed: true,
        docker_active: false,
        kvm_available: false,
        vm_driver_active: false,
        openshell_installed: true,
        image_available: true,
    };
    let docker_steps = plan_onboard(&blueprint()?, docker_facts)?;
    assert!(
        docker_steps
            .iter()
            .any(|step| step.action == OnboardAction::ConfigureDockerDriver)
    );
    assert!(
        !docker_steps
            .iter()
            .any(|step| step.action == OnboardAction::CheckKvm)
    );
    assert!(docker_steps.iter().all(|step| match step.action {
        OnboardAction::CheckWsl2 => step.check == OnboardCheck::Wsl2Available,
        OnboardAction::CheckKvm => step.check == OnboardCheck::KvmAvailable,
        OnboardAction::InstallOpenShell => step.check == OnboardCheck::OpenShellInstalled,
        OnboardAction::ConfigureDockerDriver => step.check == OnboardCheck::DockerDriverConfigured,
        OnboardAction::ConfigureVmDriver => step.check == OnboardCheck::VmDriverConfigured,
        OnboardAction::BuildSandboxImage => step.check == OnboardCheck::SandboxImageAvailable,
        OnboardAction::StartGateway => step.check == OnboardCheck::GatewayReady,
    }));
    Ok(())
}

#[test]
fn onboarding_and_doctor_keep_unknown_state_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let blueprint = blueprint()?;
    let steps = plan_onboard(&blueprint, HostFacts::default())?;
    assert!(
        steps
            .iter()
            .any(|step| step.action == OnboardAction::InstallOpenShell)
    );
    let report = evaluate_doctor(&blueprint, &ProbeResults::default());
    assert!(
        report
            .items
            .iter()
            .filter(|entry| [
                "wsl2",
                "openshell",
                "docker",
                "sandbox_image",
                "token_factory_key"
            ]
            .contains(&entry.name.as_str()))
            .all(|entry| entry.status == ProbeStatus::Unknown)
    );
    assert_eq!(
        report
            .items
            .iter()
            .find(|entry| entry.name == "kvm")
            .map(|entry| entry.status),
        Some(ProbeStatus::Pass)
    );
    Ok(())
}

#[test]
fn dockerfile_places_guest_in_policy_visible_usr_bin() -> Result<(), Box<dyn std::error::Error>> {
    let blueprint = blueprint()?;
    let recipe = dockerfile_for(&blueprint)?;
    assert!(recipe.contains("/usr/bin/polaris-guest"));
    assert_eq!(recipe.matches("useradd --create-home --uid").count(), 1);
    assert!(recipe.contains("test \"$(id -u sandbox)\" = 10001"));
    assert!(recipe.lines().any(|line| line == "COPY . /workspace"));
    assert!(recipe.contains("build-context.py"));
    Ok(())
}
