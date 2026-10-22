//! The on-disk encoded file is bounded before reading attacker-controlled bytes into memory.
use polaris_storage_cas::{Cas, CasConfig, CasError};
#[test]
fn oversized_encoded_blob_is_rejected_before_decode() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut config = CasConfig::new(root.path());
    config.max_blob_bytes = 64;
    let cas = Cas::open(config)?;
    let cid = cas.put(b"small")?;
    std::fs::write(cas.blob_path(&cid), vec![0_u8; 4096])?;
    assert!(
        matches!(
            cas.get(&cid),
            Err(CasError::TooLarge {
                limit: 66,
                size: 4096
            })
        ),
        "reject encoded file size before allocation/decoding"
    );
    Ok(())
}

#[test]
fn streaming_hash_matches_slice_and_rejects_over_limit() -> Result<(), Box<dyn std::error::Error>> {
    use polaris_storage_cas::{content_id, content_id_reader};
    let bytes = vec![0xa5; (9 << 20) + 17];
    assert_eq!(
        content_id_reader(bytes.as_slice(), u64::try_from(bytes.len())?)?,
        content_id(&bytes)
    );
    assert!(matches!(
        content_id_reader(bytes.as_slice(), 32),
        Err(CasError::TooLarge { .. })
    ));
    assert_eq!(content_id_reader(&b""[..], 0)?, content_id(b""));
    Ok(())
}
