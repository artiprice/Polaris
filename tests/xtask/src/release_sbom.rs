// Unit tests of xtask/src/release_sbom.rs (`tests`); compiled into that module via #[path].
use std::error::Error;
use std::fs;

use super::write_atomic;

#[test]
fn atomic_output_preserves_existing_file_without_force() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("release.spdx.json");
    fs::write(&output, b"user data")?;
    let result = write_atomic(&output, b"new SBOM", false);
    assert!(result.is_err(), "existing output must be protected");
    assert_eq!(fs::read(&output)?, b"user data");
    Ok(())
}

#[test]
fn atomic_output_replaces_existing_file_only_when_forced() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("release.spdx.json");
    fs::write(&output, b"user data")?;
    write_atomic(&output, b"new SBOM", true)?;
    assert_eq!(fs::read(&output)?, b"new SBOM");
    Ok(())
}

#[test]
fn failed_atomic_replacement_preserves_existing_path() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("release.spdx.json");
    fs::create_dir(&output)?;
    let sentinel = output.join("preserve.txt");
    fs::write(&sentinel, b"existing destination")?;
    let result = write_atomic(&output, b"replacement", true);
    assert!(
        result.is_err(),
        "a directory cannot be replaced with a file"
    );
    assert!(
        output.is_dir(),
        "failed replacement must preserve destination"
    );
    assert_eq!(fs::read(sentinel)?, b"existing destination");
    Ok(())
}

#[cfg(windows)]
#[test]
fn failed_atomic_replacement_preserves_locked_existing_file() -> Result<(), Box<dyn Error>> {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = tempfile::tempdir()?;
    let output = directory.path().join("release.spdx.json");
    fs::write(&output, b"existing artifact")?;
    let locked_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&output)?;
    let result = write_atomic(&output, b"replacement", true);
    assert!(
        result.is_err(),
        "a destination locked against rename must fail"
    );
    drop(locked_file);
    assert_eq!(fs::read(output)?, b"existing artifact");
    Ok(())
}

#[test]
fn cargo_lock_change_during_metadata_is_rejected() -> Result<(), Box<dyn Error>> {
    use super::ensure_lock_unchanged;

    let result = ensure_lock_unchanged(b"lock before", b"lock after");
    assert!(
        result.is_err(),
        "changed lock bytes must invalidate the result"
    );
    ensure_lock_unchanged(b"same lock", b"same lock")?;
    Ok(())
}
