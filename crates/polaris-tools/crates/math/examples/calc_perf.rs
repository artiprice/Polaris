use polaris_tools_math::{CalcLimits, calc};
use std::hint::black_box;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rounds = 1_000_000_u64;
    let expression = black_box("(1234.5 + 0.5) * 2 - 10 / 5");
    let limits = CalcLimits::default();
    let started = Instant::now();
    for _ in 0..rounds {
        black_box(calc(expression, &limits)?);
    }
    let elapsed = started.elapsed();
    let evaluations_per_second = f64::from(u32::try_from(rounds)?) / elapsed.as_secs_f64();
    println!(
        "rounds={rounds} elapsed_ms={} evaluations_per_second={evaluations_per_second:.0}",
        elapsed.as_millis()
    );
    Ok(())
}
