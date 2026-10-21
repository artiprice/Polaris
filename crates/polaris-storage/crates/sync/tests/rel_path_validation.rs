//! Rel Path Validation (descriptor: specs/descriptors/sync_rel_path_validation.json).

use std::error::Error;

use polaris_storage_sync::{RelPath, SyncError};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn accepts_normal_relative_paths() -> TestResult {
    for ok in [
        "a/b.rs",
        ".github/workflows/ci.yml",
        "dossier/été.md",
        "dir/a:b",
        "nul.txt",
        "ab:c",
    ] {
        assert_eq!(RelPath::new(ok)?.as_str(), ok);
    }
    Ok(())
}

#[test]
fn rejects_parent_absolute_and_empty() {
    // `a:b` as the first component is drive-relative on Windows (`A:` prefix).
    for bad in [
        "..",
        "a/../b",
        "/x",
        "C:/x",
        "c:",
        "a:b",
        "",
        "a//b",
        "./a",
        "a/",
        "nul\0byte",
    ] {
        assert!(
            matches!(
                RelPath::new(bad),
                Err(SyncError::InvalidPath { .. } | SyncError::RootEscape(_))
            ),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn rejects_backslash() {
    assert!(RelPath::new("a\\b").is_err());
}
