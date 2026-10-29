//! Short Reference Folding (descriptor: specs/descriptors/cas_short_reference_folding.json).

use std::error::Error;
use std::fs;

use polaris_sdk::ids::{Cid, ShortRef};
use polaris_storage_cas::{Cas, CasConfig, CasError, FOLD_REF_LEN, fold_ref};

type TestResult = Result<(), Box<dyn Error>>;

fn store() -> Result<(tempfile::TempDir, Cas), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    Ok((dir, cas))
}

/// Plants a fake blob whose name shares the first 12 hex characters with `cid` (a real 48-bit
/// collision is not findable in a test).
fn plant_twin(cas: &Cas, cid: &Cid) -> Result<(), Box<dyn Error>> {
    let mut twin = *cid.as_bytes();
    if let Some(last) = twin.last_mut() {
        *last ^= 0x01;
    }
    let twin = Cid::from_bytes(twin);
    fs::write(cas.blob_path(&twin), b"planted")?;
    Ok(())
}

#[test]
fn short_ref_is_12_hex_prefix() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(b"short")?;
    let short = cas.short_ref_checked(&cid)?;
    assert_eq!(short.to_string(), cid.to_hex().get(..12).ok_or("hex")?);
    assert_eq!(short, ShortRef::of(&cid));
    Ok(())
}

#[test]
fn resolve_unique_ok() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(b"resolve me")?;
    cas.put(b"another blob")?;
    assert_eq!(cas.resolve(&ShortRef::of(&cid))?, cid);
    Ok(())
}

#[test]
fn resolve_ambiguous_errors() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(b"ambiguous")?;
    plant_twin(&cas, &cid)?;
    assert!(
        matches!(cas.resolve(&ShortRef::of(&cid)), Err(CasError::Ambiguous(s)) if s == ShortRef::of(&cid))
    );
    Ok(())
}

#[test]
fn unknown_short_ref_errors() -> TestResult {
    let (_dir, cas) = store()?;
    let short: ShortRef = "0123456789ab".parse()?;
    assert!(matches!(cas.resolve(&short), Err(CasError::UnknownShortRef(s)) if s == short));
    Ok(())
}

#[test]
fn collision_returns_error() -> TestResult {
    let (_dir, cas) = store()?;
    let cid = cas.put(b"collide")?;
    plant_twin(&cas, &cid)?;
    assert!(
        matches!(cas.short_ref_checked(&cid), Err(CasError::ShortRefCollision(s)) if s == ShortRef::of(&cid))
    );
    Ok(())
}

#[test]
fn fold_ref_writes_marker() {
    let cid = Cid::from_bytes([0xAB; 32]);
    let mut out = [0u8; FOLD_REF_LEN];
    assert_eq!(fold_ref(&cid, &mut out), FOLD_REF_LEN);
    assert_eq!(std::str::from_utf8(&out).ok(), Some("§ref:abababababab"));
    assert_eq!(FOLD_REF_LEN, 18);
}
