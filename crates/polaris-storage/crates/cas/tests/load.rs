//! Explicit synthetic filesystem and streaming-hash measurements; no production data is used.
use polaris_storage_cas::{Cas, CasConfig};
use std::{
    io::{Seek, SeekFrom},
    time::Instant,
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
#[ignore = "explicit 10k-blob release filesystem workload"]
fn ten_thousand_small_blobs() -> TestResult {
    let root = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(root.path()))?;
    let mut latency = Vec::with_capacity(10_000);
    let mut entries = Vec::with_capacity(10_000);
    let overall = Instant::now();
    for index in 0..10_000_u32 {
        let bytes = format!("synthetic CAS load fixture {index:08}").into_bytes();
        let start = Instant::now();
        let cid = cas.put(&bytes)?;
        latency.push(start.elapsed());
        entries.push((cid, bytes));
    }
    latency.sort_unstable();
    for (cid, bytes) in entries {
        assert_eq!(cas.get(&cid)?.as_ref(), bytes.as_slice());
    }
    eprintln!(
        "CAS synthetic 10000 unique small blobs: elapsed={:?} put_p50={:?} put_p99={:?}",
        overall.elapsed(),
        latency.get(5000).ok_or("p50")?,
        latency.get(9900).ok_or("p99")?
    );
    Ok(())
}
#[test]
#[ignore = "explicit 1 GiB zero-filled streaming hash measurement"]
fn gib_streaming_hash() -> TestResult {
    let mut file = tempfile::tempfile()?;
    file.set_len(1 << 30)?;
    file.seek(SeekFrom::Start(0))?;
    let start = Instant::now();
    let digest = polaris_storage_cas::content_id_reader(&mut file, 1 << 30)?;
    let elapsed = start.elapsed();
    eprintln!(
        "BLAKE3 Rayon streaming 1GiB zero-filled file: elapsed={elapsed:?} GB_per_second={:.3} digest={digest}",
        f64::from(1_u32 << 30) / elapsed.as_secs_f64() / 1_000_000_000.0
    );
    Ok(())
}
