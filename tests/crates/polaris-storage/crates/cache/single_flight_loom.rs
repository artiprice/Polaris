//! Loom model for the cancellation contract of a single-flight placeholder.
//!
//! The production arbitration is quick_cache's async guard (covered by
//! `single_flight_compute::leader_cancel_wakes_waiters`). This small model
//! checks the state transition required of that primitive: dropping a leader
//! before publication releases ownership, and competing waiters elect only one
//! replacement leader.

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;

const IDLE: usize = 0;
const RUNNING: usize = 1;
const READY: usize = 2;

struct LeaderLease(Arc<AtomicUsize>);

impl Drop for LeaderLease {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(RUNNING, IDLE, Ordering::AcqRel, Ordering::Acquire);
    }
}

fn try_lead(state: &Arc<AtomicUsize>) -> Option<LeaderLease> {
    state
        .compare_exchange(IDLE, RUNNING, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| LeaderLease(Arc::clone(state)))
}

#[test]
fn cancelled_leader_releases_exactly_one_replacement() {
    let mut model = loom::model::Builder::new();
    model.max_branches = 10_000;
    model.check(|| {
        let state = Arc::new(AtomicUsize::new(IDLE));
        let computations = Arc::new(AtomicUsize::new(0));
        let cancelled = try_lead(&state);
        assert!(cancelled.is_some(), "the initial caller becomes leader");
        let Some(cancelled) = cancelled else {
            return;
        };
        // Cancellation occurs before replacement waiters are scheduled. They
        // still race to acquire the released placeholder after this point.
        drop(cancelled);
        let mut waiters = Vec::new();

        for _ in 0..2 {
            let state = Arc::clone(&state);
            let computations = Arc::clone(&computations);
            waiters.push(thread::spawn(move || {
                loop {
                    match state.load(Ordering::Acquire) {
                        READY => return,
                        IDLE => {
                            if let Some(lease) = try_lead(&state) {
                                computations.fetch_add(1, Ordering::SeqCst);
                                state.store(READY, Ordering::Release);
                                drop(lease);
                                return;
                            }
                        }
                        RUNNING => thread::yield_now(),
                        _ => return,
                    }
                }
            }));
        }

        // The cancelled leader published nothing; one waiter must take over.
        for waiter in waiters {
            assert!(waiter.join().is_ok(), "waiter thread completes");
        }
        assert_eq!(computations.load(Ordering::SeqCst), 1);
        assert_eq!(state.load(Ordering::Acquire), READY);
    });
}
