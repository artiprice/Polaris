//! Sync Plan (descriptor: specs/descriptors/sync_plan.json).

use std::error::Error;

use polaris_sdk::ids::Cid;
use polaris_storage_sync::{EntryKind, FileEntry, Manifest, RelPath, plan, plan_two_way};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn entry(byte: u8, executable: bool) -> FileEntry {
    FileEntry {
        size: 1,
        hash: Cid::from_bytes([byte; 32]),
        executable,
        kind: EntryKind::File,
    }
}

fn manifest(items: &[(&str, u8, bool)]) -> Result<Manifest, Box<dyn Error>> {
    let mut m = Manifest::default();
    for (path, byte, exec) in items {
        m.entries.insert(RelPath::new(path)?, entry(*byte, *exec));
    }
    Ok(m)
}

fn strs(paths: &[RelPath]) -> Vec<&str> {
    paths.iter().map(RelPath::as_str).collect()
}

#[test]
fn equal_manifests_empty_plan() -> TestResult {
    let m = manifest(&[("a", 1, false), ("b/c", 2, true)])?;
    assert!(plan(&m, &m).is_empty());
    Ok(())
}

#[test]
fn new_file_uploaded() -> TestResult {
    let p = plan(
        &manifest(&[("a", 1, false), ("n", 9, false)])?,
        &manifest(&[("a", 1, false)])?,
    );
    assert_eq!(strs(&p.upload), ["n"]);
    assert!(p.delete.is_empty());
    Ok(())
}

#[test]
fn removed_file_deleted() -> TestResult {
    let p = plan(
        &manifest(&[("a", 1, false)])?,
        &manifest(&[("a", 1, false), ("gone", 3, false)])?,
    );
    assert_eq!(strs(&p.delete), ["gone"]);
    assert!(p.upload.is_empty());
    Ok(())
}

#[test]
fn two_way_plan_moves_only_local_changes_and_keeps_remote_only_changes() -> TestResult {
    let base = manifest(&[("local", 1, false), ("remote", 2, false)])?;
    let local = manifest(&[("local", 3, false), ("remote", 2, false)])?;
    let remote = manifest(&[("local", 1, false), ("remote", 4, false)])?;
    let p = plan_two_way(&base, &local, &remote);
    assert_eq!(strs(&p.upload), ["local"]);
    assert!(p.delete.is_empty());
    assert!(p.conflicts.is_empty());
    assert_eq!(
        p.expected_remote.get(&RelPath::new("local")?),
        Some(&Some(entry(1, false)))
    );
    Ok(())
}

#[test]
fn two_way_plan_coalesces_identical_changes_and_reports_edit_races() -> TestResult {
    let base = manifest(&[("same", 1, false), ("edit", 2, false)])?;
    let local = manifest(&[("same", 3, false), ("edit", 4, false)])?;
    let remote = manifest(&[("same", 3, false), ("edit", 5, false)])?;
    let p = plan_two_way(&base, &local, &remote);
    assert_eq!(p.conflicts.len(), 1);
    assert_eq!(p.conflicts[0].path, RelPath::new("edit")?);
    assert_eq!(p.conflicts[0].base.as_ref(), Some(&entry(2, false)));
    assert_eq!(p.conflicts[0].local.as_ref(), Some(&entry(4, false)));
    assert_eq!(p.conflicts[0].remote.as_ref(), Some(&entry(5, false)));
    assert!(
        p.upload.is_empty(),
        "identical concurrent edits are already converged"
    );
    assert!(!p.expected_remote.contains_key(&RelPath::new("same")?));
    Ok(())
}

#[test]
fn two_way_plan_reports_edit_delete_and_delete_edit_conflicts() -> TestResult {
    let base = manifest(&[("edit-delete", 1, false), ("delete-edit", 2, false)])?;
    let local = manifest(&[("edit-delete", 3, false)])?;
    let remote = manifest(&[("delete-edit", 4, false)])?;
    let p = plan_two_way(&base, &local, &remote);
    assert_eq!(p.conflicts.len(), 2);
    assert!(p.upload.is_empty());
    assert!(p.delete.is_empty());
    assert_eq!(p.conflicts[0].path, RelPath::new("delete-edit")?);
    assert_eq!(p.conflicts[0].local, None);
    assert_eq!(p.conflicts[0].remote, Some(entry(4, false)));
    assert_eq!(p.conflicts[1].path, RelPath::new("edit-delete")?);
    assert_eq!(p.conflicts[1].local, Some(entry(3, false)));
    assert_eq!(p.conflicts[1].remote, None);
    Ok(())
}

#[test]
fn changed_hash_uploaded() -> TestResult {
    let p = plan(
        &manifest(&[("a", 2, false)])?,
        &manifest(&[("a", 1, false)])?,
    );
    assert_eq!(strs(&p.upload), ["a"]);
    Ok(())
}

#[test]
fn exec_bit_change_uploaded() -> TestResult {
    let p = plan(
        &manifest(&[("run.sh", 1, true)])?,
        &manifest(&[("run.sh", 1, false)])?,
    );
    assert_eq!(strs(&p.upload), ["run.sh"]);
    Ok(())
}

#[test]
fn rename_detected_by_hash() -> TestResult {
    let p = plan(
        &manifest(&[("new/name.rs", 5, false)])?,
        &manifest(&[("old/name.rs", 5, false)])?,
    );
    assert_eq!(
        p.rename_hints,
        vec![(RelPath::new("old/name.rs")?, RelPath::new("new/name.rs")?)]
    );
    Ok(())
}

fn arb_manifest() -> impl Strategy<Value = Vec<(u8, u8, bool)>> {
    proptest::collection::vec((0u8..12, 0u8..4, any::<bool>()), 0..12)
}

fn build(items: &[(u8, u8, bool)]) -> Manifest {
    let mut m = Manifest::default();
    for (name, hash, exec) in items {
        if let Ok(path) = RelPath::new(&format!("d{}/f{name}", name % 3)) {
            m.entries.insert(path, entry(*hash, *exec));
        }
    }
    m
}

proptest! {
    #[test]
    fn prop_plan_applied_to_remote_yields_local(local in arb_manifest(), remote in arb_manifest()) {
        let (local, remote) = (build(&local), build(&remote));
        let p = plan(&local, &remote);
        let mut model = remote.clone();
        for path in &p.delete {
            model.entries.remove(path);
        }
        for path in &p.upload {
            if let Some(e) = local.entries.get(path) {
                model.entries.insert(path.clone(), e.clone());
            }
        }
        prop_assert_eq!(model, local);
    }
}
