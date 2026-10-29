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

#[tokio::test]
async fn timeout_kills_linux_descendant_process_group() -> Result<(), Box<dyn std::error::Error>> {
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(WslError::NotInstalled) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let distro = DistroName::new("Ubuntu-24.04")?;
    if !wsl.doctor().await?.ubuntu_24_04_installed {
        return Ok(());
    }
    let file = format!(
        "/tmp/polaris-timeout-{}-{}.pid",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let result = wsl
        .exec(
            &distro,
            &[
                "/bin/sh",
                "-c",
                "sleep 300 & child=$!; printf '%s' \"$child\" > \"$1\"; wait \"$child\"",
                "polaris-test",
                &file,
            ],
            None,
            std::time::Duration::from_secs(2),
        )
        .await;
    assert!(
        matches!(result, Err(WslError::Timeout)),
        "expected timeout, got {result:?}"
    );
    let pid_output = wsl
        .exec(
            &distro,
            &["/bin/cat", &file],
            None,
            std::time::Duration::from_secs(10),
        )
        .await?;
    let pid = std::str::from_utf8(&pid_output.stdout)?.trim().to_owned();
    assert!(!pid.is_empty());
    let check = wsl
        .exec(
            &distro,
            &[
                "/bin/sh",
                "-c",
                "test ! -e \"/proc/$1\"",
                "polaris-test",
                &pid,
            ],
            None,
            std::time::Duration::from_secs(10),
        )
        .await?;
    let _remove = wsl
        .exec(
            &distro,
            &["/bin/rm", "-f", &file],
            None,
            std::time::Duration::from_secs(10),
        )
        .await;
    assert_eq!(
        check.exit_code, 0,
        "Linux descendant PID {pid} remained after timeout"
    );
    Ok(())
}

#[tokio::test]
async fn cancelling_exec_future_kills_linux_descendants_and_reader_owners()
-> Result<(), Box<dyn std::error::Error>> {
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(WslError::NotInstalled) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let distro = DistroName::new("Ubuntu-24.04")?;
    if !wsl.doctor().await?.ubuntu_24_04_installed {
        return Ok(());
    }
    let file = format!(
        "/tmp/polaris-cancel-{}-{}.pid",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let child_wsl = wsl.clone();
    let child_distro = distro.clone();
    let child_file = file.clone();
    let task = tokio::spawn(async move {
        child_wsl
            .exec(
                &child_distro,
                &[
                    "/bin/sh",
                    "-c",
                    "sleep 300 & child=$!; printf '%s' \"$child\" > \"$1\"; wait \"$child\"",
                    "polaris-test",
                    &child_file,
                ],
                None,
                std::time::Duration::from_secs(60),
            )
            .await
    });
    let mut pid = String::new();
    for _ in 0..30 {
        match wsl
            .exec(
                &distro,
                &["/bin/cat", &file],
                None,
                std::time::Duration::from_secs(5),
            )
            .await
        {
            Ok(output) if output.exit_code == 0 => {
                pid = String::from_utf8(output.stdout)?.trim().to_owned();
                break;
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    task.abort();
    let cancelled = task.await;
    assert!(
        cancelled.is_err(),
        "the exec future should have been cancelled"
    );
    let check = wsl
        .exec(
            &distro,
            &[
                "/bin/sh",
                "-c",
                "test ! -e \"/proc/$1\"",
                "polaris-test",
                &pid,
            ],
            None,
            std::time::Duration::from_secs(10),
        )
        .await?;
    let _remove = wsl
        .exec(
            &distro,
            &["/bin/rm", "-f", &file],
            None,
            std::time::Duration::from_secs(10),
        )
        .await;
    assert!(
        !pid.is_empty(),
        "the Linux descendant did not start before cancellation"
    );
    assert_eq!(
        check.exit_code, 0,
        "Linux descendant PID {pid} remained after cancellation"
    );
    Ok(())
}

#[tokio::test]
async fn dropping_keepalive_terminates_retained_linux_group()
-> Result<(), Box<dyn std::error::Error>> {
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(WslError::NotInstalled) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let distro = DistroName::new("Ubuntu-24.04")?;
    if !wsl.doctor().await?.ubuntu_24_04_installed {
        return Ok(());
    }
    let mut keepalive = wsl.keepalive(&distro).await?;
    let group = keepalive.process_group_id();
    assert!(keepalive.is_running().await?);
    drop(keepalive);
    let mut stopped = false;
    for _ in 0..20 {
        let result = wsl
            .exec(
                &distro,
                &["/bin/kill", "-0", "--", &format!("-{group}")],
                None,
                std::time::Duration::from_secs(5),
            )
            .await?;
        if result.exit_code != 0 {
            stopped = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        stopped,
        "Linux process group {group} remained after KeepAlive drop"
    );
    Ok(())
}

#[tokio::test]
async fn exec_enforces_bounded_output_and_preserves_success_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let wsl = match Wsl::locate() {
        Ok(wsl) => wsl,
        Err(WslError::NotInstalled) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let distro = DistroName::new("Ubuntu-24.04")?;
    if !wsl.doctor().await?.ubuntu_24_04_installed {
        return Ok(());
    }
    let output = wsl
        .exec(
            &distro,
            &["/usr/bin/printf", "%s", "bounded-output"],
            None,
            std::time::Duration::from_secs(10),
        )
        .await?;
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, b"bounded-output");
    assert!(output.stderr.is_empty());
    let too_large = wsl
        .exec(
            &distro,
            &["/usr/bin/head", "-c", "4194305", "/dev/zero"],
            None,
            std::time::Duration::from_secs(20),
        )
        .await;
    assert!(
        matches!(too_large, Err(WslError::OutputTooLarge)),
        "expected output cap error, got {too_large:?}"
    );
    Ok(())
}
