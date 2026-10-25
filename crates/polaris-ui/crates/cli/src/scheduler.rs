//! Bounded, ordered semantic input scheduling and demand-driven frame deadlines.
use crate::input::NormalizedInput;
use crossterm::event::MouseEventKind;
use std::{collections::VecDeque, time::Duration};
use thiserror::Error;
/// Maximum ordinary semantic events retained.
pub const MAX_QUEUE_RECORDS: usize = 1024;
/// Maximum aggregate ordinary payload bytes retained.
pub const MAX_QUEUE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Reserved cancellation events accepted beyond the ordinary queue capacity.
pub const MAX_CANCEL_RECORDS: usize = 8;
/// Maximum events yielded in one application turn.
pub const MAX_BATCH_EVENTS: usize = 256;
/// Maximum elapsed time allowed for one event batch.
pub const MAX_BATCH_TIME: Duration = Duration::from_millis(4);
/// Minimum interval between presented frames (60 fps ceiling).
pub const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
/// Backpressure and sequence errors returned by the scheduler.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchedulerError {
    /// Ordinary queue has reached its record limit.
    #[error("semantic input queue is full")]
    QueueFull,
    #[error("reserved cancellation queue is full")]
    /// Cancellation reserve has reached its record limit.
    CancelQueueFull,
    #[error("semantic input payload budget exceeded")]
    /// Aggregate retained payload budget would be exceeded.
    PayloadFull,
    #[error("event sequence exhausted")]
    /// Monotonic event sequence cannot be incremented safely.
    SequenceExhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalesceKind {
    Resize,
    MouseMoved,
}
#[derive(Debug)]
struct Entry {
    seq: u64,
    input: NormalizedInput,
}
/// Ordered bounded event queue with a separate cancellation reserve.
#[derive(Debug)]
pub struct InputScheduler {
    regular: VecDeque<Entry>,
    cancel: VecDeque<Entry>,
    payload: usize,
    next_seq: u64,
    last_coalescible: Option<(u64, CoalesceKind)>,
    dirty: bool,
    last_frame: Option<Duration>,
}
impl Default for InputScheduler {
    fn default() -> Self {
        Self::new()
    }
}
impl InputScheduler {
    /// Creates an empty scheduler with the fixed plan limits.
    pub fn new() -> Self {
        Self {
            regular: VecDeque::new(),
            cancel: VecDeque::new(),
            payload: 0,
            next_seq: 0,
            last_coalescible: None,
            dirty: false,
            last_frame: None,
        }
    }
    /// Enqueues an event or returns explicit backpressure.
    pub fn enqueue(&mut self, input: NormalizedInput) -> Result<(), SchedulerError> {
        let sequence = self.next_seq;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(SchedulerError::SequenceExhausted)?;
        let special = input.is_cancel_priority();
        if special {
            self.last_coalescible = None;
            if self.cancel.len() >= MAX_CANCEL_RECORDS {
                return Err(SchedulerError::CancelQueueFull);
            }
            self.next_seq = next_sequence;
            self.cancel.push_back(Entry {
                seq: sequence,
                input,
            });
            self.last_coalescible = None;
            return Ok(());
        }

        let coalesce_kind = match &input {
            NormalizedInput::Resize { .. } => Some(CoalesceKind::Resize),
            NormalizedInput::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => {
                Some(CoalesceKind::MouseMoved)
            }
            _ => None,
        };
        let can_replace_tail = match (self.last_coalescible, coalesce_kind, self.regular.back()) {
            (Some((last_sequence, last_kind)), Some(kind), Some(tail)) => {
                last_sequence == tail.seq && last_kind == kind
            }
            _ => false,
        };
        if can_replace_tail {
            if let Some(tail) = self.regular.back_mut() {
                tail.seq = sequence;
                tail.input = input;
            } else {
                self.last_coalescible = None;
                return Err(SchedulerError::QueueFull);
            }
            self.next_seq = next_sequence;
            self.last_coalescible = coalesce_kind.map(|kind| (sequence, kind));
            return Ok(());
        }

        self.last_coalescible = None;
        if self.regular.len() >= MAX_QUEUE_RECORDS {
            return Err(SchedulerError::QueueFull);
        }
        let payload_bytes = input.retained_payload_bytes();
        if self
            .payload
            .checked_add(payload_bytes)
            .filter(|total| *total <= MAX_QUEUE_PAYLOAD_BYTES)
            .is_none()
        {
            return Err(SchedulerError::PayloadFull);
        }
        self.next_seq = next_sequence;
        self.payload += payload_bytes;
        self.regular.push_back(Entry {
            seq: sequence,
            input,
        });
        self.last_coalescible = coalesce_kind.map(|kind| (sequence, kind));
        Ok(())
    }

    /// Starts a batch using the caller's monotonic timestamp.
    pub fn begin_batch(&mut self, now: Duration) -> EventBatch<'_> {
        EventBatch {
            scheduler: self,
            deadline: now.saturating_add(MAX_BATCH_TIME),
            count: 0,
        }
    }
    /// Marks visible state as requiring a frame.
    pub fn mark_dirty(&mut self) {
        self.dirty = true
    }
    /// Returns the next frame deadline only while rendering is dirty.
    pub fn next_frame_due(&self) -> Option<Duration> {
        if !self.dirty {
            return None;
        }
        Some(
            self.last_frame
                .map_or(Duration::ZERO, |t| t.saturating_add(FRAME_INTERVAL)),
        )
    }
    /// Reports whether a dirty frame may be emitted at `now`.
    pub fn frame_due(&self, now: Duration) -> bool {
        self.next_frame_due().is_some_and(|d| now >= d)
    }
    /// Records a completed frame and clears the dirty flag.
    pub fn mark_presented(&mut self, now: Duration) {
        self.dirty = false;
        self.last_frame = Some(now)
    }
    /// Returns queued ordinary and cancellation event count.
    pub fn pending(&self) -> usize {
        self.regular.len() + self.cancel.len()
    }
}
/// Limited view of one ordered scheduler batch.
#[derive(Debug)]
pub struct EventBatch<'a> {
    scheduler: &'a mut InputScheduler,
    deadline: Duration,
    count: usize,
}
impl EventBatch<'_> {
    /// Yields the next event unless the 256-event or caller timestamp deadline is reached.
    pub fn next(&mut self, now: Duration) -> Option<NormalizedInput> {
        if self.count >= MAX_BATCH_EVENTS || now >= self.deadline {
            return None;
        }
        let r = self.scheduler.regular.front().map(|x| x.seq);
        let c = self.scheduler.cancel.front().map(|x| x.seq);
        let entry = match (r, c) {
            (Some(a), Some(b)) => {
                if a < b {
                    self.scheduler.regular.pop_front()
                } else {
                    self.scheduler.cancel.pop_front()
                }
            }
            (Some(_), None) => self.scheduler.regular.pop_front(),
            (None, Some(_)) => self.scheduler.cancel.pop_front(),
            (None, None) => None,
        }?;
        self.count += 1;
        if !entry.input.is_cancel_priority() {
            self.scheduler.payload = self
                .scheduler
                .payload
                .saturating_sub(entry.input.retained_payload_bytes())
        }
        Some(entry.input)
    }
    /// Returns the number of events yielded so far.
    pub fn len(&self) -> usize {
        self.count
    }
    /// Returns whether this batch has yielded any events.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}
