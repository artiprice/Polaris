use polaris_kernel_directives::{ParseLimits, parse};
use std::time::Instant;

const CORPUS: &str = include_str!("fixtures/directives/mixed.txt");

#[test]
fn mixed_corpus_preserves_literal_code_and_email() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = parse(CORPUS, &ParseLimits::default())?;
    assert_eq!(parsed.directives.len(), 2);
    assert!(parsed.prose.contains("`literal $effort(low)`"));
    assert!(parsed.prose.contains("dev@example.com"));
    assert_eq!(parsed.mentions.len(), 1);
    Ok(())
}

#[test]
#[ignore = "release throughput workload; run explicitly with --release --ignored --nocapture"]
fn one_mebibyte_mixed_prose_throughput() -> Result<(), Box<dyn std::error::Error>> {
    let repeats = (1024usize * 1024).div_ceil(CORPUS.len());
    let input = CORPUS.repeat(repeats);
    let limits = ParseLimits {
        max_input_bytes: input.len(),
        ..ParseLimits::default()
    };
    for count in [repeats / 4, repeats / 2, repeats] {
        let size = count.saturating_mul(CORPUS.len());
        let prefix = input.get(..size).ok_or("non-ASCII corpus size boundary")?;
        let started = Instant::now();
        let mut directives = 0usize;
        for _ in 0..20 {
            directives = parse(prefix, &limits)?.directives.len();
        }
        let elapsed = started.elapsed();
        let measured_bytes = u32::try_from(prefix.len().saturating_mul(20))?;
        eprintln!(
            "directive parse bytes={size} repeats=20 directives={directives} elapsed={elapsed:?} MB/s={}",
            f64::from(measured_bytes) / elapsed.as_secs_f64() / 1_000_000.0
        );
    }
    assert_eq!(
        parse(&input, &limits)?.directives.len(),
        repeats.saturating_mul(2)
    );
    Ok(())
}
