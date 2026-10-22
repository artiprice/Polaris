//! Pure contract tests for WSL output parsing, path mapping, and config validation.

use polaris_platform_wsl::{
    DistroName, KvmStatus, LinuxPath, Wsl, WslError, decode_wsl_output, parse_distro_list,
    to_linux_path, to_windows_path, validate_wslconfig,
};
use std::path::PathBuf;

#[test]
fn decodes_utf16le_bom_and_strips_embedded_nuls() -> Result<(), Box<dyn std::error::Error>> {
    let text = "NAME\tSTATE\tVERSION\r\n* Ubuntu-24.04 Running 2\r\n";
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(decode_wsl_output(&bytes)?, text);
    assert!(decode_wsl_output(&[0xff, 0xfe, 0x41]).is_err());
    Ok(())
}

#[test]
fn parses_default_running_distro_and_version() {
    let rows = parse_distro_list(
        "NAME STATE VERSION\n* Ubuntu-24.04 Running 2\ndocker-desktop Stopped 2\n",
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.first().map(|row| row.name.as_str()),
        Some("Ubuntu-24.04")
    );
    assert_eq!(rows.first().map(|row| row.is_default), Some(true));
    assert_eq!(rows.get(1).and_then(|row| row.version), Some(2));
}

#[test]
fn validates_memory_units_nested_virtualization_and_duplicate_keys()
-> Result<(), Box<dyn std::error::Error>> {
    let report = validate_wslconfig("[wsl2]\nmemory=6GB\nnestedVirtualization=true\n")?;
    assert_eq!(report.memory_mib, Some(6144));
    assert_eq!(report.nested_virtualization, Some(true));
    assert!(validate_wslconfig("[wsl2]\nmemory=6TB\n").is_err());
    assert!(validate_wslconfig("[wsl2]\nmemory=6GB\nmemory=2GB\n").is_err());
    assert!(validate_wslconfig("[wsl2]\nmadeUp=true\n").is_err());
    Ok(())
}

#[test]
fn maps_drive_paths_and_wsl_unc_paths() -> Result<(), Box<dyn std::error::Error>> {
    let linux = to_linux_path(&polaris_platform_wsl::WinPath::new(PathBuf::from(
        r"C:\Users\dev\src",
    )))?;
    assert_eq!(linux.as_str(), "/mnt/c/Users/dev/src");
    let from_unc = to_linux_path(&polaris_platform_wsl::WinPath::new(PathBuf::from(
        r"\\wsl.localhost\Ubuntu-24.04\home\dev\src",
    )))?;
    assert_eq!(from_unc.as_str(), "/home/dev/src");
    let windows = to_windows_path(
        &LinuxPath::new("/home/dev/src")?,
        &DistroName::new("Ubuntu-24.04")?,
    );
    assert_eq!(
        windows?.as_path().to_string_lossy(),
        r"\\wsl.localhost\Ubuntu-24.04\home\dev\src"
    );
    assert!(
        to_linux_path(&polaris_platform_wsl::WinPath::new(PathBuf::from(
            r"C:\work\..\secret"
        )))
        .is_err()
    );
    assert!(LinuxPath::new("relative/path").is_err());
    Ok(())
}

#[test]
fn doctor_report_api_is_serializable_and_provisioning_is_plan_only()
-> Result<(), Box<dyn std::error::Error>> {
    let report = polaris_platform_wsl::WslReport {
        available: true,
        version: Some("WSL version: 2".to_owned()),
        distributions: Vec::new(),
        ubuntu_24_04_installed: false,
        kvm: KvmStatus::NotProbed,
        diagnostics: vec!["Ubuntu-24.04 is not installed".to_owned()],
    };
    let json = serde_json::to_string(&report)?;
    assert!(json.contains("ubuntu_24_04_installed"));
    let plan = Wsl::provision_ubuntu_plan();
    assert!(plan.requires_explicit_confirmation);
    assert!(plan.may_require_elevation);
    assert_eq!(plan.argv.first().map(String::as_str), Some("--install"));
    Ok(())
}

#[tokio::test]
async fn doctor_collects_a_real_read_only_host_report() -> Result<(), Box<dyn std::error::Error>> {
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(WslError::NotInstalled) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let report = wsl.doctor().await?;
    assert!(report.available);
    let json = serde_json::to_string(&report)?;
    assert!(json.contains("distributions"));
    if report.ubuntu_24_04_installed {
        let output = wsl
            .exec(
                &DistroName::new("Ubuntu-24.04")?,
                &["/usr/bin/printf", "%s", "polaris-wsl-exec"],
                None,
                std::time::Duration::from_secs(10),
            )
            .await?;
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, b"polaris-wsl-exec");
    }
    Ok(())
}
