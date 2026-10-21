//! Bounded FIFO dead letters. Eviction is returned to the caller and counted.
use crate::{ChildName, ExitClass};
use std::collections::VecDeque;
use std::num::NonZeroUsize;

/// Failed work plus its retry history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter<T> {
    /// Original item or CAS reference.
    pub item: T,
    /// Failed child.
    pub child: ChildName,
    /// Last observed exit.
    pub reason: ExitClass,
    /// Total runs including the initial attempt.
    pub attempts: u32,
}
/// Oldest-first queue with a hard nonzero bound.
#[derive(Debug)]
pub struct DeadLetterQueue<T> {
    items: VecDeque<DeadLetter<T>>,
    capacity: NonZeroUsize,
    dropped: u64,
}
impl<T> DeadLetterQueue<T> {
    /// Create an empty queue.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity,
            dropped: 0,
        }
    }
    /// Append, returning the oldest letter if evicted.
    pub fn push(&mut self, letter: DeadLetter<T>) -> Option<DeadLetter<T>> {
        let evicted = if self.items.len() == self.capacity.get() {
            self.dropped = self.dropped.saturating_add(1);
            self.items.pop_front()
        } else {
            None
        };
        self.items.push_back(letter);
        evicted
    }
    /// Remove all retained letters in arrival order.
    pub fn drain(&mut self) -> Vec<DeadLetter<T>> {
        self.items.drain(..).collect()
    }
    /// Current retained letters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }
    /// Whether no letter is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Maximum retained letters.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity.get()
    }
    /// Lifetime evictions, saturating at `u64::MAX`.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}
