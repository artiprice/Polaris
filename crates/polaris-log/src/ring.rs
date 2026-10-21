//! Fixed-capacity, overwrite-oldest ring of log lines (the TUI log pane and the telemetry ring).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::entry::{Entry, LogLine, LogRecordRef, MIN_LINE_BYTES};

/// Default line limit of [`RingSink::new`], in bytes (target and message together).
pub const DEFAULT_RING_LINE_BYTES: usize = 1024;

/// Locks a mutex; a poisoned lock (a panic elsewhere) must not take logging down with it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A bounded in-memory ring of the most recent log lines.
///
/// All slots are allocated in the constructor; [`push`](Self::push) copies into an existing slot
/// (cutting over-long lines to the line limit) and therefore never allocates. When the ring is
/// full the oldest line is overwritten and [`dropped`](Self::dropped) is incremented. The only
/// lock is held for one memory copy; no I/O ever happens under it.
#[derive(Debug)]
pub struct RingSink {
    inner: Mutex<RingInner>,
    dropped: AtomicU64,
    capacity: usize,
    line_limit: usize,
}

#[derive(Debug)]
struct RingInner {
    slots: Vec<Entry>,
    start: usize,
    len: usize,
    next_seq: u64,
}

impl RingSink {
    /// A ring of `capacity` lines (at least 1) with the default line limit
    /// ([`DEFAULT_RING_LINE_BYTES`]).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::with_line_limit(capacity, DEFAULT_RING_LINE_BYTES)
    }

    /// A ring of `capacity` lines (at least 1) whose lines are cut to `line_limit` bytes (at least
    /// [`MIN_LINE_BYTES`]).
    #[must_use]
    pub fn with_line_limit(capacity: usize, line_limit: usize) -> Self {
        let capacity = capacity.max(1);
        let line_limit = line_limit.max(MIN_LINE_BYTES);
        let slots = (0..capacity).map(|_| Entry::new(line_limit)).collect();
        Self {
            inner: Mutex::new(RingInner {
                slots,
                start: 0,
                len: 0,
                next_seq: 0,
            }),
            dropped: AtomicU64::new(0),
            capacity,
            line_limit,
        }
    }

    /// Stores a record, overwriting the oldest line when full. Hot path: zero-alloc (log enqueue).
    pub fn push(&self, rec: &LogRecordRef<'_>) {
        let mut inner = lock(&self.inner);
        let RingInner {
            slots,
            start,
            len,
            next_seq,
        } = &mut *inner;
        let index = if *len == self.capacity {
            // Full: the oldest slot becomes the newest.
            let oldest = *start;
            *start = (*start + 1) % self.capacity;
            self.dropped.fetch_add(1, Ordering::Relaxed);
            oldest
        } else {
            let free = (*start + *len) % self.capacity;
            *len += 1;
            free
        };
        match slots.get_mut(index) {
            Some(slot) => slot.fill(*next_seq, rec, self.line_limit),
            None => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        *next_seq = next_seq.wrapping_add(1);
    }

    /// The stored lines, oldest first (cold path; allocates).
    #[must_use]
    pub fn snapshot(&self) -> Vec<LogLine> {
        let inner = lock(&self.inner);
        (0..inner.len)
            .filter_map(|i| inner.slots.get((inner.start + i) % self.capacity))
            .map(Entry::to_line)
            .collect()
    }

    /// How many lines were overwritten because the ring was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Number of lines currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.inner).len
    }

    /// Whether the ring holds no lines.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Maximum number of lines.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Longest stored line in bytes (target and message together).
    #[must_use]
    pub fn line_limit(&self) -> usize {
        self.line_limit
    }
}
