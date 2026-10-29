//! Drain Queue (descriptor: specs/descriptors/log_drain_queue.json).

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use polaris_log::{DrainQueue, Level, LogRecordRef};

type TestResult = Result<(), Box<dyn Error>>;

fn push(queue: &DrainQueue, message: &str) {
    queue.push(&LogRecordRef {
        time: SystemTime::UNIX_EPOCH,
        level: Level::INFO,
        target: "test",
        message,
    });
}

fn recv_all(queue: &DrainQueue) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(line) = queue.try_recv() {
        out.push(line.message);
    }
    out
}

#[test]
fn records_come_out_in_fifo_order() {
    let queue = DrainQueue::new(8, 256);
    for i in 0..5 {
        push(&queue, &format!("m{i}"));
    }
    assert_eq!(queue.pending(), 5);
    assert_eq!(recv_all(&queue), ["m0", "m1", "m2", "m3", "m4"]);
    assert_eq!(queue.pending(), 0);
    assert_eq!(queue.dropped(), 0);
}

#[test]
fn overflow_drops_the_newest_and_counts() {
    let queue = DrainQueue::new(3, 256);
    for i in 0..10 {
        push(&queue, &format!("m{i}"));
    }
    assert_eq!(queue.accepted(), 3);
    assert_eq!(queue.dropped(), 7);
    assert_eq!(recv_all(&queue), ["m0", "m1", "m2"]);
}

#[test]
fn push_never_blocks_without_a_consumer() {
    let queue = DrainQueue::new(4, 256);
    let start = Instant::now();
    for i in 0..50_000 {
        push(&queue, if i % 2 == 0 { "even" } else { "odd" });
    }
    assert!(start.elapsed() < Duration::from_secs(30), "push blocked");
    assert_eq!(queue.accepted() + queue.dropped(), 50_000);
}

#[test]
fn consumed_slots_are_recycled() {
    let queue = DrainQueue::new(2, 256);
    for round in 0..20 {
        push(&queue, &format!("a{round}"));
        push(&queue, &format!("b{round}"));
        assert_eq!(recv_all(&queue), [format!("a{round}"), format!("b{round}")]);
    }
    assert_eq!(queue.dropped(), 0, "slots returned after each receive");
}

#[test]
fn close_wakes_the_consumer_after_draining() -> TestResult {
    let queue = Arc::new(DrainQueue::new(8, 256));
    let seen = Arc::new(AtomicU64::new(0));
    let consumer = {
        let (queue, seen) = (Arc::clone(&queue), Arc::clone(&seen));
        std::thread::spawn(move || {
            loop {
                while queue.try_recv().is_some() {
                    seen.fetch_add(1, Ordering::SeqCst);
                }
                queue.mark_flushed();
                if !queue.wait_for_items() {
                    break;
                }
            }
        })
    };
    for i in 0..5 {
        push(&queue, &format!("m{i}"));
    }
    queue.close();
    consumer.join().map_err(|_| "consumer panicked")?;
    assert_eq!(
        seen.load(Ordering::SeqCst),
        5,
        "everything queued before close is delivered"
    );
    Ok(())
}

#[test]
fn push_after_close_is_dropped_and_counted() {
    let queue = DrainQueue::new(4, 256);
    queue.close();
    push(&queue, "late");
    assert_eq!(queue.dropped(), 1);
    assert_eq!(queue.accepted(), 0);
    assert!(
        !queue.wait_for_items(),
        "closed and empty ends the consumer"
    );
}

#[test]
fn flush_waits_for_the_consumer() -> TestResult {
    let queue = Arc::new(DrainQueue::new(8, 256));
    push(&queue, "pending");
    // No consumer yet: the flush times out instead of blocking forever.
    assert!(!queue.flush(Duration::from_millis(50)));

    let consumer = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            loop {
                while queue.try_recv().is_some() {}
                queue.mark_flushed();
                if !queue.wait_for_items() {
                    break;
                }
            }
        })
    };
    assert!(queue.flush(Duration::from_secs(10)));
    queue.close();
    consumer.join().map_err(|_| "consumer panicked")?;
    Ok(())
}

#[test]
fn oversize_lines_are_truncated_not_grown() -> TestResult {
    let queue = DrainQueue::new(2, 256);
    push(&queue, &"y".repeat(5_000));
    let line = queue.try_recv().ok_or("nothing queued")?;
    assert!(line.truncated);
    assert!(line.message.len() <= 256);
    Ok(())
}
