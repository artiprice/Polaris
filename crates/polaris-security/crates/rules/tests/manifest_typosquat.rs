//! Trusted-baseline manifest-name review contract.
use std::error::Error;
use std::path::Path;

use polaris_security_rules::{
    ManifestAuditStatus, PackageEcosystem, RuleEngine, TrustedPackageBaseline,
};

type TestResult = Result<(), Box<dyn Error>>;

fn baseline(
    ecosystem: PackageEcosystem,
    names: &[&str],
) -> Result<TrustedPackageBaseline, Box<dyn Error>> {
    Ok(TrustedPackageBaseline::new(
        ecosystem,
        names.iter().map(|name| (*name).to_owned()),
    )?)
}

#[test]
fn missing_trusted_baseline_is_explicit_and_scans_nothing() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let report = engine.audit_manifest(Path::new("Cargo.toml"), b"not valid TOML", None)?;
    assert_eq!(
        report.status(),
        ManifestAuditStatus::BaselineUnavailable(PackageEcosystem::Cargo)
    );
    assert_eq!(report.findings().len(), 0);
    Ok(())
}

#[test]
fn exact_known_package_is_excluded() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["serde"])?;
    let report = engine.audit_manifest(
        Path::new("Cargo.toml"),
        b"[dependencies]\nserde = \"1\"\n",
        Some(&trusted),
    )?;
    assert_eq!(report.status(), ManifestAuditStatus::Analyzed);
    assert_eq!(report.findings().len(), 0);
    Ok(())
}

#[test]
fn near_package_name_reports_without_cwe() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["reqwest"])?;
    let report = engine.audit_manifest(
        Path::new("Cargo.toml"),
        b"[dependencies]\nreqwset = \"1\"\n",
        Some(&trusted),
    )?;
    let finding = report
        .findings()
        .first()
        .ok_or("missing typo review finding")?;
    assert_eq!(finding.package_name(), "reqwset");
    assert_eq!(finding.nearest_known_names(), &[String::from("reqwest")]);
    assert_eq!(finding.distance(), 1);
    assert!(!finding.ambiguous());
    assert_eq!(finding.cwe(), None);
    Ok(())
}

#[test]
fn ambiguous_equal_distance_candidates_are_sorted_and_marked() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Npm, &["food", "fool"])?;
    let report = engine.audit_manifest(
        Path::new("package.json"),
        br#"{"dependencies":{"fooz":"1.0.0"}}"#,
        Some(&trusted),
    )?;
    let finding = report.findings().first().ok_or("missing tied finding")?;
    assert_eq!(
        finding.nearest_known_names(),
        &[String::from("food"), String::from("fool")]
    );
    assert_eq!(finding.distance(), 1);
    assert!(finding.ambiguous());
    Ok(())
}

#[test]
fn cargo_renamed_dependency_uses_declared_package_name() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["reqwest"])?;
    let report = engine.audit_manifest(
        Path::new("Cargo.toml"),
        b"[dependencies]\nhttp-client = { package = \"reqwset\", version = \"1\" }\n",
        Some(&trusted),
    )?;
    let finding = report
        .findings()
        .first()
        .ok_or("missing renamed package finding")?;
    assert_eq!(finding.package_name(), "reqwset");
    Ok(())
}

#[test]
fn npm_and_python_manifests_extract_dependency_names() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let npm_trusted = baseline(PackageEcosystem::Npm, &["react"])?;
    let npm = engine.audit_manifest(
        Path::new("package.json"),
        br#"{"devDependencies":{"raect":"18.0.0"}}"#,
        Some(&npm_trusted),
    )?;
    assert_eq!(
        npm.findings()
            .first()
            .map(polaris_security_rules::TyposquatFinding::package_name),
        Some("raect")
    );

    let python_trusted = baseline(PackageEcosystem::Python, &["requests"])?;
    let python = engine.audit_manifest(
        Path::new("pyproject.toml"),
        b"[project]\ndependencies = [\"requsets>=2\"]\n",
        Some(&python_trusted),
    )?;
    assert_eq!(
        python
            .findings()
            .first()
            .map(polaris_security_rules::TyposquatFinding::package_name),
        Some("requsets")
    );
    Ok(())
}

#[test]
fn python_distribution_name_separators_normalize_to_one_dash() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Python, &["some-package"])?;
    let report = engine.audit_manifest(
        Path::new("requirements.txt"),
        b"Some.._Package>=1\n",
        Some(&trusted),
    )?;
    assert_eq!(report.status(), ManifestAuditStatus::Analyzed);
    assert_eq!(report.findings().len(), 0);
    Ok(())
}

#[test]
fn oversized_inputs_and_baselines_fail_before_partial_output() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["serde"])?;
    let oversized = vec![b'x'; 600 * 1024];
    let result = engine.audit_manifest(Path::new("Cargo.toml"), &oversized, Some(&trusted));
    assert!(matches!(
        result,
        Err(polaris_security_rules::RulesError::Bound(_))
    ));
    let too_many_names = vec![String::from("serde"); 20_001];
    assert!(matches!(
        TrustedPackageBaseline::new(PackageEcosystem::Cargo, too_many_names),
        Err(polaris_security_rules::RulesError::Bound(_))
    ));
    Ok(())
}

#[test]
fn overlong_manifest_path_is_rejected_before_findings_copy_it() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["serde"])?;
    let path = std::path::PathBuf::from("a".repeat(5000));
    let result = engine.audit_manifest(&path, b"[dependencies]\nserde = \"1\"\n", Some(&trusted));
    assert!(matches!(
        result,
        Err(polaris_security_rules::RulesError::Bound(_))
    ));
    Ok(())
}

#[test]
fn unsupported_manifest_has_explicit_status() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let trusted = baseline(PackageEcosystem::Cargo, &["serde"])?;
    let report =
        engine.audit_manifest(Path::new("notes.txt"), b"arbitrary text", Some(&trusted))?;
    assert_eq!(report.status(), ManifestAuditStatus::UnsupportedManifest);
    assert_eq!(report.findings().len(), 0);
    Ok(())
}
