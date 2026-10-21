//! Ring Sink (descriptor: specs/descriptors/log_ring_sink.json).

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use polaris_log::{Level, LogRecordRef, RingSink};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn push(ring: &RingSink, message: &str) {
    ring.push(&LogRecordRef {
        time: SystemTime::UNIX_EPOCH,
        level: Level::INFO,
        target: "test",
        message,
    });
}

fn messages(ring: &RingSink) -> Vec<String> {
    ring.snapshot().into_iter().map(|l| l.message).collect()
}

#[test]
fn push_beyond_capacity_overwrites_oldest() {
    let ring = RingSink::new(3);
    for i in 0..5 {
        push(&ring, &format!("m{i}"));
    }
    assert_eq!(messages(&ring), ["m2", "m3", "m4"]);
    assert_eq!(ring.len(), 3);
}

#[test]
fn dropped_counter_increments() {
    let ring = RingSink::new(2);
    push(&ring, "a");
    push(&ring, "b");
    assert_eq!(ring.dropped(), 0, "no overwrite while there is room");
    for _ in 0..3 {
        push(&ring, "c");
    }
    assert_eq!(ring.dropped(), 3);
}

#[test]
fn snapshot_preserves_order() {
    let ring = RingSink::new(4);
    for i in 0..3 {
        push(&ring, &format!("m{i}"));
    }
    let seqs: Vec<u64> = ring.snapshot().iter().map(|l| l.seq).collect();
    assert_eq!(messages(&ring), ["m0", "m1", "m2"]);
    assert_eq!(seqs, [0, 1, 2]);
    // After wrapping, sequence numbers keep counting every push ever made.
    for i in 3..7 {
        push(&ring, &format!("m{i}"));
    }
    let seqs: Vec<u64> = ring.snapshot().iter().map(|l| l.seq).collect();
    assert_eq!(seqs, [3, 4, 5, 6]);
}

#[test]
fn record_fields_round_trip() -> TestResult {
    let ring = RingSink::new(2);
    let time = SystemTime::UNIX_EPOCH + Duration::from_millis(1_234);
    ring.push(&LogRecordRef {
        time,
        level: Level::WARN,
        target: "polaris::kernel",
        message: "disk almost full",
    });
    let line = ring.snapshot().into_iter().next().ok_or("empty snapshot")?;
    assert_eq!(line.time, time);
    assert_eq!(line.level, Level::WARN);
    assert_eq!(line.target, "polaris::kernel");
    assert_eq!(line.message, "disk almost full");
    assert!(!line.truncated);
    Ok(())
}

#[test]
fn long_lines_are_truncated_on_a_char_boundary() -> TestResult {
    let ring = RingSink::with_line_limit(2, 256);
    let long = "é".repeat(400);
    push(&ring, &long);
    let line = ring.snapshot().into_iter().next().ok_or("empty snapshot")?;
    assert!(line.truncated);
    assert!(line.message.len() <= 256);
    assert!(!line.message.is_empty());
    assert!(line.message.chars().all(|c| c == 'é'), "no split character");
    assert!(long.starts_with(&line.message));
    Ok(())
}

#[test]
fn zero_capacity_is_clamped_to_one() {
    let ring = RingSink::new(0);
    assert_eq!(ring.capacity(), 1);
    push(&ring, "a");
    push(&ring, "b");
    assert_eq!(messages(&ring), ["b"]);
    assert_eq!(ring.dropped(), 1);
}

#[test]
fn concurrent_pushes_are_all_accounted() -> TestResult {
    let ring = Arc::new(RingSink::new(64));
    let mut handles = Vec::new();
    for t in 0..8 {
        let ring = Arc::clone(&ring);
        handles.push(std::thread::spawn(move || {
            for i in 0..500 {
                push(&ring, &format!("t{t}-{i}"));
            }
        }));
    }
    for h in handles {
        h.join().map_err(|_| "producer panicked")?;
    }
    assert_eq!(ring.len(), 64);
    assert_eq!(ring.dropped(), 8 * 500 - 64);
    Ok(())
}

proptest! {
    #[test]
    fn prop_ring_keeps_last_capacity_records_in_order(
        capacity in 1usize..16,
        msgs in proptest::collection::vec("[a-z]{0,20}", 0..60),
    ) {
        let ring = RingSink::new(capacity);
        for m in &msgs {
            push(&ring, m);
        }
        let keep = msgs.len().saturating_sub(capacity);
        let expected: Vec<String> = msgs.iter().skip(keep).cloned().collect();
        prop_assert_eq!(messages(&ring), expected);
        prop_assert_eq!(ring.dropped(), keep as u64);
    }
}
