//! Explicit, measured 100 MiB throughput checks (run with `cargo test -p polaris-tools-rtk --test load -- --ignored --nocapture`).

use std::time::Instant;

use polaris_tools_rtk::{StreamFormat, filter_text, parse_diagnostics};

const LOAD_BYTES: usize = 100 * 1024 * 1024;

fn profile_name() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn repeated_to_size(line: &str, size: usize) -> String {
    let count = size / line.len() + if size % line.len() == 0 { 0 } else { 1 };
    let mut text = String::with_capacity(count.saturating_mul(line.len()));
    for _ in 0..count {
        text.push_str(line);
    }
    text.truncate(size);
    text
}

#[test]
#[ignore = "100 MiB throughput measurement; invoke explicitly"]
fn measures_text_filter_throughput() -> Result<(), Box<dyn std::error::Error>> {
    let input = repeated_to_size("Compiling example v0.1.0\n", LOAD_BYTES);
    let started = Instant::now();
    let filtered = filter_text(input.as_bytes())?;
    let elapsed = started.elapsed();
    let input_mib = f64::from(u32::try_from(LOAD_BYTES)?) / (1024.0 * 1024.0);
    let mib_per_second = input_mib / elapsed.as_secs_f64();
    eprintln!(
        "RTK_LOAD text_filter input_bytes={LOAD_BYTES} elapsed_ms={} throughput_mib_s={mib_per_second:.2} target_mib_s=200 rust_profile={}",
        elapsed.as_millis(),
        profile_name()
    );
    assert!(
        filtered.is_empty(),
        "compiler progress lines should be filtered"
    );
    Ok(())
}

#[test]
#[ignore = "100 MiB throughput measurement; invoke explicitly"]
fn measures_cargo_json_parse_throughput() -> Result<(), Box<dyn std::error::Error>> {
    let input = repeated_to_size(
        "{\"reason\":\"compiler-message\",\"message\":{\"level\":\"note\",\"message\":\"note\",\"spans\":[],\"children\":[]}}\n",
        LOAD_BYTES,
    );
    let started = Instant::now();
    let diagnostics = parse_diagnostics(input.as_bytes(), StreamFormat::CargoJson)?;
    let elapsed = started.elapsed();
    let input_mib = f64::from(u32::try_from(LOAD_BYTES)?) / (1024.0 * 1024.0);
    let mib_per_second = input_mib / elapsed.as_secs_f64();
    eprintln!(
        "RTK_LOAD cargo_json input_bytes={LOAD_BYTES} elapsed_ms={} throughput_mib_s={mib_per_second:.2} target_mib_s=100 rust_profile={}",
        elapsed.as_millis(),
        profile_name()
    );
    assert!(
        diagnostics.is_empty(),
        "note-level messages are not diagnostics"
    );
    Ok(())
}
