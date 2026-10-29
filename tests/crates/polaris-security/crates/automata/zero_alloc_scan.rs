//! Zero Alloc Scan (descriptor: specs/descriptors/automata_zero_alloc_scan.json; PLAN ADR-003).
//! One integration-test binary per zero-alloc check: dhat allows one profiler per process.

use polaris_sdk::registry::Redactor;
use polaris_security_automata::{DfaRedactor, DfaScanner, ScanBuf};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn scan_does_not_allocate_after_warmup() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let scanner = DfaScanner::default_pack()?;
    let redactor = DfaRedactor::default_redactor()?;
    let key = format!("{}{}", "AK", "IA") + "Q7XH2PZL4MNB8KVD";
    let hay = format!(
        "token {key} for ops@example.org; rm -rf / ; plain text {}",
        "lorem ipsum ".repeat(20)
    );
    let mut buf = ScanBuf::with_capacity(32);
    let mut out = Vec::with_capacity(hay.len() * 2);

    for _ in 0..50 {
        scanner.scan(hay.as_bytes(), &mut buf)?;
        scanner.any_match(hay.as_bytes())?;
        out.clear();
        redactor.redact_into(hay.as_bytes(), &mut out);
    }

    let before = dhat::HeapStats::get();
    for _ in 0..1000 {
        scanner.scan(hay.as_bytes(), &mut buf)?;
        scanner.any_match(hay.as_bytes())?;
        out.clear();
        redactor.redact_into(hay.as_bytes(), &mut out);
    }
    let after = dhat::HeapStats::get();
    assert_eq!(
        after.total_blocks, before.total_blocks,
        "scan/any_match/redact_into allocated after warm-up"
    );
    assert!(buf.len() >= 3);
    Ok(())
}
