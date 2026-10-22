//! Shared retry backoff helper with a provider hint as a hard floor.

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hash, Hasher},
    time::Duration,
};

/// Exponential backoff with per-call randomized jitter. A server Retry-After is a floor and is
/// never shortened. The 30-second local ceiling applies only to the generated exponential delay.
#[must_use]
pub fn jittered_backoff(attempt: u32, retry_after: Option<Duration>) -> Duration {
    let shifts = attempt.min(8);
    let base_ns = match 100_000_000_u64.checked_shl(shifts) {
        Some(value) => value,
        None => 25_600_000_000,
    }
    .min(30_000_000_000);
    let mut hasher = RandomState::new().build_hasher();
    attempt.hash(&mut hasher);
    retry_after.hash(&mut hasher);
    let jitter_range = base_ns / 2;
    let jitter = if jitter_range == u64::MAX {
        0
    } else {
        hasher.finish() % jitter_range.saturating_add(1)
    };
    let randomized_ns = (base_ns / 2).saturating_add(jitter).min(30_000_000_000);
    let randomized = Duration::from_nanos(randomized_ns);
    retry_after.map_or(randomized, |floor| randomized.max(floor))
}
