//! Tests for `xtask::policy::check_witness` (descriptor: specs/descriptors/xtask_check_witness.json).

use std::error::Error;
use std::fs;
use std::path::Path;

use xtask::policy::check_witness;

type TestResult = Result<(), Box<dyn Error>>;

const CALL: &str = "let w = ApprovalWitness::new(id, cid, at);\n";

fn write(root: &Path, rel: &str, body: &str) -> TestResult {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, body)?;
    Ok(())
}

#[test]
fn verifier_and_sdk_may_construct_witnesses() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(dir.path(), "crates/polaris-sdk/src/contracts.rs", CALL)?;
    write(
        dir.path(),
        "crates/polaris-sdk/tests/command_typestate.rs",
        CALL,
    )?;
    write(
        dir.path(),
        "crates/polaris-kernel/crates/verifier/src/lib.rs",
        CALL,
    )?;
    assert!(check_witness(dir.path())?.is_empty());
    Ok(())
}

#[test]
fn other_crates_are_reported_with_line() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(
        dir.path(),
        "crates/polaris-kernel/crates/operators/src/lib.rs",
        &format!("// header\n{CALL}"),
    )?;
    write(dir.path(), "apps/polaris-cli/src/main.rs", CALL)?;
    let violations = check_witness(dir.path())?;
    assert_eq!(
        violations,
        vec![
            "apps/polaris-cli/src/main.rs:1".to_owned(),
            "crates/polaris-kernel/crates/operators/src/lib.rs:2".to_owned()
        ]
    );
    Ok(())
}

#[test]
fn build_output_is_ignored() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(
        dir.path(),
        "crates/polaris-runtime/target/debug/build/gen.rs",
        CALL,
    )?;
    assert!(check_witness(dir.path())?.is_empty());
    Ok(())
}

#[test]
fn clean_tree_has_no_violations() -> TestResult {
    let dir = tempfile::tempdir()?;
    write(
        dir.path(),
        "crates/polaris-runtime/src/lib.rs",
        "fn main() {}\n",
    )?;
    assert!(check_witness(dir.path())?.is_empty());
    Ok(())
}
