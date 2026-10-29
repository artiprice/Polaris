//! Optimized workload for PLAN throughput/tail targets; raw timings are retained by the caller.
use polaris_runtime_leases::{LeaseConfig, LeaseKey, LeaseManager, LeaseRequest, LeaseScope};
use polaris_sdk::ids::FrameId;
use std::time::Instant;

#[ignore = "run explicitly with --release --nocapture for actual performance targets"]
#[test]
fn lookup_release_throughput_and_tail_latency() -> Result<(), Box<dyn std::error::Error>> {
    let manager = LeaseManager::new(LeaseConfig::default());
    let mut guards = Vec::new();
    for index in 0..1024 {
        guards.push(manager.try_acquire(LeaseRequest {
            agent: FrameId::new_v7(),
            key: LeaseKey::from_resolved_relative(&format!("source-{index:04}.rs"))?,
            scope: LeaseScope::EntireFile,
            base: None,
        })?);
    }
    let key = LeaseKey::from_resolved_relative("source-1023.rs")?;
    let started = Instant::now();
    for _ in 0..1_000_000 {
        std::hint::black_box(
            manager.holder_of(std::hint::black_box(&key), &LeaseScope::EntireFile),
        );
    }
    let elapsed = started.elapsed();
    let per_second = 1_000_000_f64 / elapsed.as_secs_f64();
    let mut samples = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let start = Instant::now();
        std::hint::black_box(manager.holder_of(&key, &LeaseScope::EntireFile));
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let p99 = *samples.get(9_899).ok_or("sample missing")?;
    println!(
        "lease_lookup holders=1024 calls=1000000 elapsed_ns={} lookups_per_second={per_second:.0} p99_ns={p99}",
        elapsed.as_nanos()
    );
    assert!(
        per_second >= 10_000_000_f64,
        "PLAN throughput target missed: {per_second}"
    );
    assert!(p99 < 1000, "PLAN p99 target missed: {p99} ns");
    assert_eq!(guards.len(), 1024);
    Ok(())
}
