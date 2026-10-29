//! Tests for `xtask::members::check_members` (descriptor: specs/descriptors/xtask_check_members.json).

use std::error::Error;
use std::fs;
use std::path::Path;

use xtask::members::{MemberViolation, check_members};
use xtask::table::CrateTable;

type TestResult = Result<(), Box<dyn Error>>;

const TABLE: &str = r#"
[[crate]]
name = "app-a"
path = "apps/app-a"
layer = 7
kind = "app"

[[crate]]
name = "core"
path = "crates/core"
layer = 0
kind = "lib"

[[crate]]
name = "core-child"
path = "crates/core/crates/child"
parent = "core"
layer = 0
kind = "lib"

[[crate]]
name = "tests-pkg"
path = "tests"
layer = 8
kind = "test"

[[crate]]
name = "spike-one"
path = "tests/spike/one"
layer = 8
kind = "spike"

[[crate]]
name = "xtask"
path = "xtask"
layer = 8
kind = "xtask"
"#;

fn manifest(root: &Path, rel: &str) -> TestResult {
    let dir = root.join(rel);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("Cargo.toml"), "[package]\n")?;
    Ok(())
}

fn consistent_tree(root: &Path) -> TestResult {
    for rel in [
        "apps/app-a",
        "crates/core",
        "crates/core/crates/child",
        "tests",
        "tests/spike/one",
        "xtask",
    ] {
        manifest(root, rel)?;
    }
    Ok(())
}

#[test]
fn all_table_packages_present_is_ok() -> TestResult {
    let dir = tempfile::tempdir()?;
    consistent_tree(dir.path())?;
    let table = CrateTable::from_toml_str(TABLE)?;
    let violations = check_members(dir.path(), &table)?;
    assert!(
        violations.is_empty(),
        "expected no violations, got {violations:?}"
    );
    Ok(())
}

#[test]
fn glob_dir_without_manifest_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    consistent_tree(dir.path())?;
    fs::create_dir_all(dir.path().join("crates/empty"))?;
    let table = CrateTable::from_toml_str(TABLE)?;
    let violations = check_members(dir.path(), &table)?;
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, MemberViolation::MissingManifest(p) if p == "crates/empty")),
        "expected MissingManifest(crates/empty), got {violations:?}"
    );
    Ok(())
}

#[test]
fn stray_file_at_glob_level_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    consistent_tree(dir.path())?;
    fs::write(dir.path().join("crates/README.md"), "stray")?;
    let table = CrateTable::from_toml_str(TABLE)?;
    let violations = check_members(dir.path(), &table)?;
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, MemberViolation::StrayFile(p) if p == "crates/README.md")),
        "expected StrayFile(crates/README.md), got {violations:?}"
    );
    Ok(())
}

#[test]
fn table_package_missing_on_disk_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    consistent_tree(dir.path())?;
    fs::remove_dir_all(dir.path().join("tests/spike/one"))?;
    let table = CrateTable::from_toml_str(TABLE)?;
    let violations = check_members(dir.path(), &table)?;
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, MemberViolation::NotOnDisk(n) if n == "spike-one")),
        "expected NotOnDisk(spike-one), got {violations:?}"
    );
    Ok(())
}

#[test]
fn member_not_in_table_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    consistent_tree(dir.path())?;
    manifest(dir.path(), "apps/rogue")?;
    let table = CrateTable::from_toml_str(TABLE)?;
    let violations = check_members(dir.path(), &table)?;
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, MemberViolation::NotInTable(p) if p == "apps/rogue")),
        "expected NotInTable(apps/rogue), got {violations:?}"
    );
    Ok(())
}
