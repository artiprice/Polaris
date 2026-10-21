//! Blob Store Round Trip (descriptor: specs/descriptors/cas_blob_store_round_trip.json).

use std::error::Error;
use std::fs;
use std::time::{Duration, SystemTime};

use polaris_storage_cas::codec::Codec;
use polaris_storage_cas::{Cas, CasConfig, CasError, GcPolicy, content_id};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.to_le_bytes().first().copied().unwrap_or(0)
        })
        .collect()
}

fn store() -> Result<(tempfile::TempDir, Cas), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    Ok((dir, cas))
}

#[test]
fn put_get_roundtrip_byte_exact() -> TestResult {
    let (_dir, cas) = store()?;
    let compressible = "polaris ".repeat(128 * 1024).into_bytes();
    for blob in [Vec::new(), vec![42u8], compressible, noise(64 * 1024, 7)] {
        let cid = cas.put(&blob)?;
        assert_eq!(
            cas.get(&cid)?.as_ref(),
            blob.as_slice(),
            "len {}",
            blob.len()
        );
        assert!(cas.contains(&cid)?);
    }
    Ok(())
}

#[test]
fn put_is_idempotent() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(b"same bytes")?;
    let path = cas.blob_path(&cid);
    let past = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(&path)?
        .set_modified(past)?;
    assert_eq!(cas.put(b"same bytes")?, cid);
    assert_eq!(
        fs::metadata(&path)?.modified()?,
        past,
        "second put must not rewrite the blob"
    );
    Ok(())
}

#[test]
fn get_missing_is_not_found() -> TestResult {
    let (_dir, cas) = store()?;
    let missing = content_id(b"never stored");
    assert!(matches!(cas.get(&missing), Err(CasError::NotFound(c)) if c == missing));
    assert!(!cas.contains(&missing)?);
    Ok(())
}

#[test]
fn incompressible_stored_raw() -> TestResult {
    let (_dir, cas) = store()?;
    let random = cas.put(&noise(4096, 99))?;
    let tiny = cas.put(b"tiny")?;
    let text = cas.put("aaaa".repeat(4096).as_bytes())?;
    let codec = |cid| -> Result<Option<Codec>, Box<dyn Error>> {
        Ok(polaris_storage_cas::codec::codec_of(&fs::read(
            cas.blob_path(cid),
        )?))
    };
    assert_eq!(codec(&random)?, Some(Codec::Raw));
    assert_eq!(codec(&tiny)?, Some(Codec::Raw));
    assert_eq!(codec(&text)?, Some(Codec::Zstd));
    Ok(())
}

#[test]
fn corrupt_blob_detected_on_verify() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(&noise(1024, 3))?;
    let path = cas.blob_path(&cid);
    let mut bytes = fs::read(&path)?;
    if let Some(last) = bytes.last_mut() {
        *last ^= 0xFF;
    }
    fs::write(&path, bytes)?;
    assert!(matches!(cas.get(&cid), Err(CasError::Corrupt { cid: c }) if c == cid));
    Ok(())
}

#[test]
fn content_id_is_blake3_and_layout_is_ab_cd_hex() -> TestResult {
    let (dir, cas) = store()?;
    let cid = cas.put(b"layout")?;
    assert_eq!(cid.as_bytes(), blake3::hash(b"layout").as_bytes());
    assert_eq!(content_id(b"layout"), cid);
    let hex = cid.to_hex();
    let expected = dir
        .path()
        .join("cas")
        .join(hex.get(0..2).ok_or("hex")?)
        .join(hex.get(2..4).ok_or("hex")?)
        .join(&hex);
    assert_eq!(cas.blob_path(&cid), expected);
    assert!(expected.is_file());
    Ok(())
}

#[test]
fn put_file_streams_and_matches_put() -> TestResult {
    let (dir, cas) = store()?;
    let data = noise(300_000, 5);
    let file = dir.path().join("input.bin");
    fs::write(&file, &data)?;
    let from_file = cas.put_file(&file)?;
    assert_eq!(from_file, content_id(&data));
    assert_eq!(cas.get(&from_file)?.as_ref(), data.as_slice());
    Ok(())
}

#[test]
fn too_large_is_rejected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut cfg = CasConfig::new(dir.path().join("cas"));
    cfg.max_blob_bytes = 1000;
    let cas = Cas::open(cfg)?;
    assert!(matches!(
        cas.put(&[0u8; 1001]),
        Err(CasError::TooLarge { limit: 1000, .. })
    ));
    assert!(cas.put(&[0u8; 1000]).is_ok());
    Ok(())
}

#[test]
fn gc_removes_dead_blobs_after_grace() -> TestResult {
    let (_dir, cas) = store()?;
    let live = cas.put(b"live")?;
    let dead_old = cas.put(b"dead and old")?;
    let dead_young = cas.put(b"dead but young")?;
    let past = SystemTime::now() - Duration::from_secs(7200);
    for cid in [&live, &dead_old] {
        fs::File::options()
            .write(true)
            .open(cas.blob_path(cid))?
            .set_modified(past)?;
    }
    let policy = GcPolicy {
        grace: Duration::from_secs(3600),
        dry_run: true,
    };
    let dry = cas.gc([live], policy)?;
    assert_eq!(
        (dry.scanned, dry.removed, dry.kept_live, dry.kept_young),
        (3, 1, 1, 1)
    );
    assert!(cas.contains(&dead_old)?, "dry run removes nothing");

    let report = cas.gc(
        [live],
        GcPolicy {
            dry_run: false,
            ..policy
        },
    )?;
    assert_eq!(report.removed, 1);
    assert!(report.bytes_freed > 0);
    assert!(cas.contains(&live)? && cas.contains(&dead_young)?);
    assert!(!cas.contains(&dead_old)?);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn prop_put_get_roundtrip(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let dir = tempfile::tempdir();
        prop_assert!(dir.is_ok());
        if let Ok(dir) = dir {
            let cas = Cas::open(CasConfig::new(dir.path().join("cas")));
            prop_assert!(cas.is_ok());
            if let Ok(cas) = cas {
                let cid = cas.put(&bytes);
                prop_assert!(cid.is_ok());
                if let Ok(cid) = cid {
                    prop_assert_eq!(cas.get(&cid).ok().map(|b| b.to_vec()), Some(bytes));
                }
            }
        }
    }
}
