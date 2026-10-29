//! Entropy Candidate Detection (descriptor: specs/descriptors/automata_entropy_candidate_detection.json).

use std::error::Error;

use polaris_security_automata::{EntropyCfg, ScanBuf, high_entropy_spans, shannon_entropy_milli};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Deterministic pseudo-random base64 text (xorshift), standing in for a leaked key.
fn pseudo_random_base64(len: usize, mut state: u64) -> String {
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let idx = usize::try_from(state % 64).unwrap_or(0);
        out.push(char::from(*B64.get(idx).unwrap_or(&b'A')));
    }
    out
}

#[test]
fn random_base64_flagged() -> TestResult {
    let secret = pseudo_random_base64(40, 0x9E37_79B9_7F4A_7C15);
    let hay = format!("config: {secret} end");
    let mut buf = ScanBuf::with_capacity(4);
    assert_eq!(
        high_entropy_spans(hay.as_bytes(), &EntropyCfg::default(), &mut buf)?,
        1
    );
    let m = buf.matches().first().ok_or("no span")?;
    assert_eq!(
        hay.get(m.start as usize..m.end as usize),
        Some(secret.as_str())
    );
    Ok(())
}

#[test]
fn english_prose_not_flagged() -> TestResult {
    let prose = "Internationalization and characteristically incomprehensible counterrevolutionaries notwithstanding.";
    let mut buf = ScanBuf::with_capacity(4);
    assert_eq!(
        high_entropy_spans(prose.as_bytes(), &EntropyCfg::default(), &mut buf)?,
        0
    );
    Ok(())
}

#[test]
fn short_runs_ignored() -> TestResult {
    let short = pseudo_random_base64(20, 7);
    let mut buf = ScanBuf::with_capacity(4);
    let cfg = EntropyCfg {
        min_len: 21,
        threshold_milli: 1000,
    };
    assert_eq!(high_entropy_spans(short.as_bytes(), &cfg, &mut buf)?, 0);
    Ok(())
}

#[test]
fn hex_hashes_and_uuids_not_flagged() -> TestResult {
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let uuid = "01920000-0000-7000-8000-0000000000a1";
    let mut buf = ScanBuf::with_capacity(4);
    assert_eq!(
        high_entropy_spans(
            format!("{sha} {uuid}").as_bytes(),
            &EntropyCfg::default(),
            &mut buf
        )?,
        0
    );
    Ok(())
}

#[test]
fn entropy_of_constant_is_zero() {
    assert_eq!(shannon_entropy_milli(b"aaaaaaaaaaaaaaaa"), 0);
    assert_eq!(shannon_entropy_milli(b""), 0);
}

#[test]
fn entropy_of_all_bytes_is_eight_bits() {
    let all: Vec<u8> = (0..=255u8).collect();
    let e = shannon_entropy_milli(&all);
    assert!((7999..=8000).contains(&e), "got {e}");
    assert!(
        (999..=1000).contains(&shannon_entropy_milli(b"abab")),
        "two symbols = 1 bit"
    );
}

proptest! {
    #[test]
    fn prop_entropy_bounded_by_log2_len(bytes in proptest::collection::vec(any::<u8>(), 1..512)) {
        let e = shannon_entropy_milli(&bytes);
        let distinct = {
            let mut seen = [false; 256];
            for b in &bytes { if let Some(s) = seen.get_mut(usize::from(*b)) { *s = true; } }
            seen.iter().filter(|s| **s).count()
        };
        // H <= log2(distinct symbols) bits.
        let bound = (f64::from(u32::try_from(distinct).unwrap_or(256)).log2() * 1000.0).ceil();
        prop_assert!(f64::from(e) <= bound + 1.0, "e={} bound={}", e, bound);
        prop_assert!(e <= 8000);
    }
}
