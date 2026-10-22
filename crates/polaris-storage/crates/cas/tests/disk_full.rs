//! Explicit ENOSPC exercise on a disposable, externally size-limited test filesystem.
use polaris_storage_cas::{Cas, CasConfig, CasError, content_id};
#[test]
#[ignore = "requires CAS_TEST_FULL_DIRECTORY pointing at a disposable <=64KiB filesystem"]
fn disk_full_leaves_no_partial_blob() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("CAS_TEST_FULL_DIRECTORY")
        .ok_or("missing explicit limited test filesystem")?;
    let directory = tempfile::tempdir_in(root)?;
    let mut config = CasConfig::new(directory.path());
    config.min_compress_bytes = usize::MAX;
    let cas = Cas::open(config)?;
    let bytes = vec![0xa5; 256 << 10];
    let cid = content_id(&bytes);
    assert!(
        matches!(cas.put(&bytes), Err(CasError::Io(_))),
        "actual write must fail on the deliberately exhausted filesystem"
    );
    assert!(
        !cas.blob_path(&cid).exists(),
        "failed write must not publish a partial blob"
    );
    fn check_no_temp(path: &std::path::Path) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                check_no_temp(&entry.path())?;
            } else {
                assert!(
                    !entry.file_name().to_string_lossy().starts_with(".tmp-"),
                    "failed temporary write must be removed"
                );
            }
        }
        Ok(())
    }
    check_no_temp(directory.path())?;
    Ok(())
}
