//! Real filesystem containment precedes lexical lease identity.
use polaris_runtime_leases::LeaseKey;

#[test]
fn aliases_share_key_and_missing_leaf_is_contained() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("nested"))?;
    std::fs::write(root.path().join("nested/a.rs"), b"source")?;
    let relative = LeaseKey::from_path(root.path(), std::path::Path::new("nested/a.rs"))?;
    let absolute = LeaseKey::from_path(root.path(), &root.path().join("nested/a.rs"))?;
    assert_eq!(relative, absolute, "same file has one identity");
    assert_eq!(
        LeaseKey::from_path(root.path(), std::path::Path::new("nested/new.rs"))?.as_str(),
        "nested/new.rs"
    );
    assert!(
        LeaseKey::from_path(root.path(), std::path::Path::new("missing-parent/new.rs")).is_err()
    );
    Ok(())
}

#[test]
fn outside_root_symlink_and_short_alias_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    std::fs::write(outside.path().join("target.rs"), b"outside")?;
    assert!(LeaseKey::from_path(root.path(), &outside.path().join("target.rs")).is_err());
    for hostile in [
        "../escape",
        "file:stream",
        "CON.rs",
        "short~1.rs",
        "a/../b.rs",
        "trailing. ",
    ] {
        assert!(
            LeaseKey::from_path(root.path(), std::path::Path::new(hostile)).is_err(),
            "{hostile}"
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
        assert!(
            LeaseKey::from_path(root.path(), std::path::Path::new("escape/target.rs")).is_err()
        );
        assert!(LeaseKey::from_path(root.path(), std::path::Path::new("escape/new.rs")).is_err());
    }
    Ok(())
}
