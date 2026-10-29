// Unit tests of crates/polaris-ratelimit/src/window.rs (`tests`); compiled into that module via #[path].
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
