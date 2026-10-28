//! Fixed-size conservative 15-minute usage ledger.

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
struct WindowData {
    buckets: [Bucket; WINDOW_BUCKETS],
    saturated: bool,
}

#[derive(Default)]
pub(crate) struct WindowTracker {
    data: Mutex<WindowData>,
}

impl WindowTracker {
    pub(crate) fn record_dispatch(&self, minute: u64, reserved_tokens: u64) {
        let mut data = lock_recover(&self.data);
        let Some(bucket) = data.buckets.get_mut(bucket_index(minute)) else {
            data.saturated = true;
            return;
        };
        if bucket.minute != Some(minute) {
            *bucket = Bucket {
                minute: Some(minute),
                ..Bucket::EMPTY
            };
        }
        let (requests, request_overflow) = add_saturating(bucket.requests, 1);
        let (reserved, reserved_overflow) = add_saturating(bucket.reserved_tokens, reserved_tokens);
        bucket.requests = requests;
        bucket.reserved_tokens = reserved;
        data.saturated |= request_overflow || reserved_overflow;
    }

    pub(crate) fn record_actual(&self, minute: u64, actual_tokens: u64) {
        let mut data = lock_recover(&self.data);
        let Some(bucket) = data.buckets.get_mut(bucket_index(minute)) else {
            data.saturated = true;
            return;
        };
        // A response older than the retained 15-minute horizon does not belong in the current view.
        if bucket.minute == Some(minute) {
            let (actual, overflow) = add_saturating(bucket.actual_tokens, actual_tokens);
            bucket.actual_tokens = actual;
            data.saturated |= overflow;
        }
    }

    pub(crate) fn snapshot(&self, current_minute: u64) -> WindowSnapshot {
        let data = lock_recover(&self.data);
        let mut snapshot = WindowSnapshot {
            accounting_saturated: data.saturated,
            ..WindowSnapshot::default()
        };
        for bucket in data.buckets.iter() {
            let Some(minute) = bucket.minute else {
                continue;
            };
            if minute > current_minute || current_minute.saturating_sub(minute) >= 15 {
                continue;
            }
            snapshot.requests = add_snapshot_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.requests,
                bucket.requests,
            );
            snapshot.reserved_tokens = add_snapshot_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.reserved_tokens,
                bucket.reserved_tokens,
            );
            snapshot.actual_tokens = add_snapshot_saturating(
                &mut snapshot.accounting_saturated,
                snapshot.actual_tokens,
                bucket.actual_tokens,
            );
        }
        snapshot
    }
}

fn bucket_index(minute: u64) -> usize {
    match usize::try_from(minute.rem_euclid(15)) {
        Ok(index) => index,
        Err(_) => 0,
    }
}

fn add_saturating(left: u64, right: u64) -> (u64, bool) {
    match left.checked_add(right) {
        Some(value) => (value, false),
        None => (u64::MAX, true),
    }
}

fn add_snapshot_saturating(saturated: &mut bool, left: u64, right: u64) -> u64 {
    let (value, overflow) = add_saturating(left, right);
    *saturated |= overflow;
    value
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_overflow_saturates_without_partial_dispatch() {
        let tracker = WindowTracker::default();
        let mut data = lock_recover(&tracker.data);
        let Some(bucket) = data.buckets.get_mut(bucket_index(0)) else {
            return;
        };
        *bucket = Bucket {
            minute: Some(0),
            requests: u64::MAX,
            reserved_tokens: u64::MAX,
            actual_tokens: 0,
        };
        drop(data);
        tracker.record_dispatch(0, 1);
        let snapshot = tracker.snapshot(0);
        assert_eq!(snapshot.requests, u64::MAX);
        assert_eq!(snapshot.reserved_tokens, u64::MAX);
        assert!(snapshot.accounting_saturated);
    }
}
