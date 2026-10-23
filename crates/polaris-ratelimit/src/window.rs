//! Fixed-size conservative 15-minute usage ledger.

use crate::RateLimitError;
use std::sync::{Mutex, MutexGuard};

const WINDOW_BUCKETS: usize = 15;

#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    minute: Option<u64>,
    requests: u64,
    reserved_tokens: u64,
    actual_tokens: u64,
}

impl Bucket {
    const EMPTY: Self = Self {
        minute: None,
        requests: 0,
        reserved_tokens: 0,
        actual_tokens: 0,
    };
}

/// 15-minute rolling-bucket counters. Minute granularity means the exact age within the oldest
/// bucket is not available; it reports observed local use and does not invent a provider limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowSnapshot {
    /// Requests marked dispatched in the rolling bucket window.
    pub requests: u64,
    /// Sum of pre-dispatch token reservations.
    pub reserved_tokens: u64,
    /// Sum of provider-reported actual tokens reconciled before bucket expiry.
    pub actual_tokens: u64,
    /// True if a snapshot sum saturated because local counters exceeded u64.
    pub accounting_saturated: bool,
}

#[derive(Default)]
pub(crate) struct WindowTracker {
    buckets: Mutex<[Bucket; WINDOW_BUCKETS]>,
}

impl WindowTracker {
    pub(crate) fn record_dispatch(
        &self,
        minute: u64,
        reserved_tokens: u64,
    ) -> Result<(), RateLimitError> {
        let mut buckets = lock_recover(&self.buckets);
        let index = bucket_index(minute);
        let bucket = &mut buckets[index];
        if bucket.minute != Some(minute) {
            *bucket = Bucket {
                minute: Some(minute),
                ..Bucket::EMPTY
            };
        }
        bucket.requests = bucket
            .requests
            .checked_add(1)
            .ok_or(RateLimitError::Accounting)?;
        bucket.reserved_tokens = bucket
            .reserved_tokens
            .checked_add(reserved_tokens)
            .ok_or(RateLimitError::Accounting)?;
        Ok(())
    }

    pub(crate) fn record_actual(
        &self,
        minute: u64,
        actual_tokens: u64,
    ) -> Result<(), RateLimitError> {
        let mut buckets = lock_recover(&self.buckets);
        let bucket = &mut buckets[bucket_index(minute)];
        // A response older than the retained 15-minute horizon does not belong in the current view.
        if bucket.minute == Some(minute) {
            bucket.actual_tokens = bucket
                .actual_tokens
                .checked_add(actual_tokens)
                .ok_or(RateLimitError::Accounting)?;
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self, current_minute: u64) -> WindowSnapshot {
        let buckets = lock_recover(&self.buckets);
        let mut snapshot = WindowSnapshot::default();
        for bucket in buckets.iter() {
            let Some(minute) = bucket.minute else {
                continue;
            };
            if minute > current_minute || current_minute - minute >= 15 {
                continue;
            }
            snapshot.requests = add_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.requests,
                bucket.requests,
            );
            snapshot.reserved_tokens = add_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.reserved_tokens,
                bucket.reserved_tokens,
            );
            snapshot.actual_tokens = add_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.actual_tokens,
                bucket.actual_tokens,
            );
        }
        snapshot
    }
}

fn bucket_index(minute: u64) -> usize {
    let Ok(index) = usize::try_from(minute % 15) else {
        return 0;
    };
    index
}

fn add_saturating(saturated: &mut bool, left: u64, right: u64) -> u64 {
    match left.checked_add(right) {
        Some(value) => value,
        None => {
            *saturated = true;
            u64::MAX
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
