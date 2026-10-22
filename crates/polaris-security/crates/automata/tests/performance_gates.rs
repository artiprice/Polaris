use std::time::{Duration, Instant};

use polaris_security_automata::{Category, CompileLimits, RuleSetBuilder};

fn scanner(pattern_count: u32) -> (polaris_security_automata::DfaScanner, Duration, usize) {
    let mut builder = RuleSetBuilder::default();
    let mut source_bytes = 0_usize;
    for index in 0..pattern_count {
        let pattern = format!("POLARIS_SECRET_{index:04}");
        source_bytes += pattern.len();
        let name = format!("rule_{index:04}");
        let added = builder.add(&name, &pattern, Category::Secret);
        assert!(added.is_ok(), "fixture pattern must be accepted");
    }
    let start = Instant::now();
    let result = builder.build(&CompileLimits::default());
    let build_time = start.elapsed();
    let compiled = result.expect("bounded literal rule pack must compile");
    (compiled, build_time, source_bytes)
}

#[test]
fn reports_automata_compile_scan_and_4k_tail_latency() {
    let sample = vec![b'x'; 4096];
    for count in [10_u32, 100, 1000] {
        let (scanner, build_time, pattern_source_bytes) = scanner(count);
        let iterations = 1000_u32;
        let start = Instant::now();
        for _ in 0..iterations {
            let result = scanner.any_match(&sample);
            assert_eq!(result, Ok(false));
        }
        let small_scan_elapsed = start.elapsed();
        let small_scan_bytes = sample.len() as u128 * u128::from(iterations);
        let small_scan_mib_per_second =
            (small_scan_bytes as f64 / 1_048_576.0) / small_scan_elapsed.as_secs_f64();

        let mut latencies = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            let start = Instant::now();
            let result = scanner.any_match(&sample);
            assert_eq!(result, Ok(false));
            latencies.push(start.elapsed());
        }
        latencies.sort_unstable();
        let p99 = latencies[9_899];

        if count == 10 {
            assert!(
                p99 < Duration::from_micros(2),
                "10-rule 4 KiB any_match p99 is over 2 us: {p99:?}"
            );
        }

        eprintln!(
            "rule_count={count};pattern_source_bytes={pattern_source_bytes};engine_memory_bytes={};build_ms={};scan_mib_s={small_scan_mib_per_second:.2};any_match_4k_p99_us={:.3}",
            scanner.engine_memory_bytes(),
            build_time.as_secs_f64() * 1000.0,
            p99.as_secs_f64() * 1_000_000.0,
        );
    }

    if cfg!(debug_assertions) {
        eprintln!("large throughput gate requires an optimized release build");
        return;
    }

    let (scanner, _, _) = scanner(10);
    let sample = vec![b'x'; 1024 * 1024];
    let iterations = 512_u32;
    let start = Instant::now();
    for _ in 0..iterations {
        let result = scanner.any_match(&sample);
        assert_eq!(result, Ok(false));
    }
    let elapsed = start.elapsed();
    let bytes = sample.len() as u128 * u128::from(iterations);
    let mib_per_second = (bytes as f64 / 1_048_576.0) / elapsed.as_secs_f64();
    eprintln!(
        "stream_bytes={bytes};elapsed_ms={};scan_mib_s={mib_per_second:.2}",
        elapsed.as_secs_f64() * 1000.0
    );
    assert!(
        mib_per_second >= 500.0,
        "10-rule scanner throughput below 500 MiB/s: {mib_per_second:.2} MiB/s"
    );
}
