//! Bounded stream scan contracts: finite-width overlap, exact global spans, and 1 GiB throughput.

use std::time::Instant;

use polaris_security_automata::{
    AutomataError, Category, CompileLimits, DfaScanner, ScanBuf, StreamScanner,
};

fn finite_scanner(pattern: &str) -> Result<DfaScanner, AutomataError> {
    let mut builder = DfaScanner::builder();
    builder.add("needle", pattern, Category::Secret)?;
    builder.build(&CompileLimits::default())
}

#[test]
fn emits_one_match_crossing_two_chunk_boundaries_with_global_offsets() -> Result<(), AutomataError>
{
    let scanner = finite_scanner("ABCD1234")?;
    let mut stream = StreamScanner::new(&scanner, 16, 8)?;
    let mut out = ScanBuf::with_capacity(8);
    assert_eq!(stream.push(b"xABCD", &mut out)?, 0);
    assert_eq!(stream.push(b"1234yAB", &mut out)?, 1);
    let matched = out
        .matches()
        .first()
        .ok_or(AutomataError::TooLarge { limit: 0 })?;
    assert_eq!((matched.start, matched.end), (1, 9));
    assert_eq!(stream.push(b"CD1234", &mut out)?, 1);
    let matched = out
        .matches()
        .first()
        .ok_or(AutomataError::TooLarge { limit: 0 })?;
    assert_eq!((matched.start, matched.end), (10, 18));
    Ok(())
}

#[test]
fn rejects_unbounded_patterns_and_overlarge_chunks() -> Result<(), AutomataError> {
    let unbounded = finite_scanner("A+")?;
    assert!(matches!(
        StreamScanner::new(&unbounded, 16, 8),
        Err(AutomataError::UnboundedMatch)
    ));
    let contextual = finite_scanner(r"\bword\b")?;
    assert!(matches!(
        StreamScanner::new(&contextual, 16, 8),
        Err(AutomataError::UnsupportedStreamPattern)
    ));
    let finite = finite_scanner("ABCD")?;
    let mut stream = StreamScanner::new(&finite, 2, 8)?;
    let mut out = ScanBuf::with_capacity(8);
    assert!(matches!(
        stream.push(b"three", &mut out),
        Err(AutomataError::TooLarge { limit: 2 })
    ));
    Ok(())
}

#[test]
fn streams_one_gibibyte_with_boundary_matches() -> Result<(), Box<dyn std::error::Error>> {
    let scanner = finite_scanner("ABCD1234")?;
    const CHUNK_BYTES: usize = 1 << 20;
    const CHUNK_COUNT: usize = 1 << 10;
    let mut chunk = vec![b'x'; CHUNK_BYTES];
    let prefix_at = CHUNK_BYTES.checked_sub(4).ok_or("chunk too small")?;
    chunk
        .get_mut(..4)
        .ok_or("suffix range missing")?
        .copy_from_slice(b"1234");
    chunk
        .get_mut(prefix_at..)
        .ok_or("prefix range missing")?
        .copy_from_slice(b"ABCD");
    let mut stream = StreamScanner::new(&scanner, CHUNK_BYTES, 2)?;
    let mut out = ScanBuf::with_capacity(2);
    let started = Instant::now();
    let mut match_count = 0_usize;
    for index in 0..CHUNK_COUNT {
        match_count = match_count.saturating_add(stream.push(&chunk, &mut out)?);
        if index > 0 {
            let expected_start = index
                .checked_mul(CHUNK_BYTES)
                .and_then(|offset| offset.checked_sub(4))
                .ok_or("expected offset overflow")?;
            let matched = out.matches().first().ok_or("boundary match missing")?;
            assert_eq!(usize::try_from(matched.start)?, expected_start);
            assert_eq!(usize::try_from(matched.end)?, expected_start + 8);
            assert_eq!(out.len(), 1);
        }
    }
    let elapsed = started.elapsed();
    assert_eq!(match_count, CHUNK_COUNT - 1);
    println!(
        "stream_bytes={};chunk_bytes={CHUNK_BYTES};matches={match_count};elapsed_ms={}",
        CHUNK_BYTES * CHUNK_COUNT,
        elapsed.as_millis()
    );
    Ok(())
}
