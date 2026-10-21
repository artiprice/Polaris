//! Sliding window of billed tokens per second, for the HUD burn rate.

use std::collections::VecDeque;

/// Width of the window in seconds: a sample at second `s` counts at time `now` while
/// `s > now - 60`.
pub(crate) const WINDOW_SECS: i64 = 60;

/// Tokens per wall-clock second, bounded to [`WINDOW_SECS`] buckets, ordered by second.
#[derive(Debug, Clone, Default)]
pub(crate) struct RateWindow {
    buckets: VecDeque<(i64, u64)>,
}

impl RateWindow {
    /// Adds `tokens` at second `sec`. A sample older than the window ending at the newest
    /// second seen so far is dropped; out-of-order samples inside the window are kept.
    pub(crate) fn add(&mut self, sec: i64, tokens: u64) {
        let newest = self.buckets.back().map_or(sec, |bucket| bucket.0.max(sec));
        let floor = newest.saturating_sub(WINDOW_SECS);
        if sec <= floor {
            return;
        }
        match self.buckets.binary_search_by_key(&sec, |bucket| bucket.0) {
            Ok(index) => {
                if let Some(bucket) = self.buckets.get_mut(index) {
                    bucket.1 = bucket.1.saturating_add(tokens);
                }
            }
            Err(index) => self.buckets.insert(index, (sec, tokens)),
        }
        while self.buckets.front().is_some_and(|bucket| bucket.0 <= floor) {
            self.buckets.pop_front();
        }
    }

    /// Tokens in the window ending at second `now`.
    pub(crate) fn sum_at(&self, now: i64) -> u64 {
        let floor = now.saturating_sub(WINDOW_SECS);
        self.buckets
            .iter()
            .filter(|bucket| bucket.0 > floor)
            .fold(0u64, |sum, bucket| sum.saturating_add(bucket.1))
    }
}
