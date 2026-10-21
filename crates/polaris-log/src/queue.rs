//! Bounded queue between the producers (any thread) and the single drain thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::entry::{Entry, LogLine, LogRecordRef, MIN_LINE_BYTES};
use crate::ring::lock;

/// A bounded multi-producer, single-consumer queue of reusable slots.
///
/// **Overflow policy: drop the newest.** When every slot is in use a new record is discarded and
/// [`dropped`](Self::dropped) is incremented; the producer never waits for the consumer or for
/// I/O. All slots are allocated up front and recycled, so [`push`](Self::push) does not allocate.
/// The lock is held only for one memory copy.
#[derive(Debug)]
pub struct DrainQueue {
    state: Mutex<State>,
    item_ready: Condvar,
    flushed_cv: Condvar,
    dropped: AtomicU64,
    line_limit: usize,
}

#[derive(Debug)]
struct State {
    filled: VecDeque<Entry>,
    free: Vec<Entry>,
    closed: bool,
    accepted: u64,
    popped: u64,
    flushed: u64,
    consumer_waiting: bool,
}

impl DrainQueue {
    /// A queue of `capacity` slots (at least 1) whose lines are cut to `line_limit` bytes (at
    /// least [`MIN_LINE_BYTES`]).
    #[must_use]
    pub fn new(capacity: usize, line_limit: usize) -> Self {
        let capacity = capacity.max(1);
        let line_limit = line_limit.max(MIN_LINE_BYTES);
        Self {
            state: Mutex::new(State {
                filled: VecDeque::with_capacity(capacity),
                free: (0..capacity).map(|_| Entry::new(line_limit)).collect(),
                closed: false,
                accepted: 0,
                popped: 0,
                flushed: 0,
                consumer_waiting: false,
            }),
            item_ready: Condvar::new(),
            flushed_cv: Condvar::new(),
            dropped: AtomicU64::new(0),
            line_limit,
        }
    }

    /// Enqueues a record, or drops it (and counts the drop) when the queue is full or closed.
    /// Hot path: zero-alloc (log enqueue); never waits for the consumer.
    pub fn push(&self, rec: &LogRecordRef<'_>) {
        let wake = {
            let mut state = lock(&self.state);
            if state.closed {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let Some(mut entry) = state.free.pop() else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            };
            entry.fill(state.accepted, rec, self.line_limit);
            state.filled.push_back(entry);
            state.accepted = state.accepted.saturating_add(1);
            state.consumer_waiting
        };
        if wake {
            self.item_ready.notify_one();
        }
    }

    /// Takes the oldest record, if any (consumer side). The slot is recycled immediately.
    #[must_use]
    pub fn try_recv(&self) -> Option<LogLine> {
        let entry = {
            let mut state = lock(&self.state);
            let entry = state.filled.pop_front()?;
            state.popped = state.popped.saturating_add(1);
            entry
        };
        let line = entry.to_line();
        lock(&self.state).free.push(entry);
        Some(line)
    }

    /// Blocks the consumer until a record is available (`true`) or the queue is closed and empty
    /// (`false`).
    #[must_use]
    pub fn wait_for_items(&self) -> bool {
        let mut state = lock(&self.state);
        loop {
            if !state.filled.is_empty() {
                return true;
            }
            if state.closed {
                return false;
            }
            state.consumer_waiting = true;
            state = self
                .item_ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
            state.consumer_waiting = false;
        }
    }

    /// Consumer side: declares that everything received so far has been written and flushed.
    pub fn mark_flushed(&self) {
        let mut state = lock(&self.state);
        state.flushed = state.popped;
        drop(state);
        self.flushed_cv.notify_all();
    }

    /// Waits until the consumer has flushed every record accepted before this call. `false` if
    /// `timeout` elapsed first (for example because nothing consumes the queue).
    #[must_use]
    pub fn flush(&self, timeout: Duration) -> bool {
        // Clamp so absurd timeouts cannot overflow `Instant`.
        let deadline = Instant::now() + timeout.min(Duration::from_secs(86_400));
        let mut state = lock(&self.state);
        let target = state.accepted;
        while state.flushed < target {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .flushed_cv
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Rejects further pushes and wakes the consumer so it can drain what is left and stop.
    pub fn close(&self) {
        lock(&self.state).closed = true;
        self.item_ready.notify_all();
    }

    /// Records discarded because the queue was full or closed.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Records accepted so far.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        lock(&self.state).accepted
    }

    /// Records waiting for the consumer.
    #[must_use]
    pub fn pending(&self) -> usize {
        lock(&self.state).filled.len()
    }

    /// Longest stored line in bytes (target and message together).
    #[must_use]
    pub fn line_limit(&self) -> usize {
        self.line_limit
    }
}
