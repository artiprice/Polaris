//! Bounded-page replay over an immutable append-only prefix.
use crate::{EventLog, EventLogError};
use polaris_sdk::{contracts::EventRecord, ids::Seq};
use std::collections::VecDeque;

/// Lazy journal replay. Holds at most 256 records, captures its upper bound at construction,
/// and closes after the first storage/codec/gap error.
#[derive(Debug)]
pub struct Replay<'a> {
    log: &'a EventLog,
    cursor: Option<u64>,
    until: u64,
    page: VecDeque<EventRecord>,
}
impl EventLog {
    /// Replay through `until` with bounded memory, excluding records appended after construction.
    /// # Errors
    /// Reading the current head failed. Later I/O or corruption failures are iterator items.
    pub fn replay_iter(&self, until: Seq) -> Result<Replay<'_>, EventLogError> {
        let until = until.get().min(self.head()?.get());
        Ok(Replay {
            log: self,
            cursor: if until == 0 { None } else { Some(1) },
            until,
            page: VecDeque::new(),
        })
    }
}
impl Iterator for Replay<'_> {
    type Item = Result<EventRecord, EventLogError>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(record) = self.page.pop_front() {
            return Some(Ok(record));
        }
        let from = self.cursor?;
        let end = from.saturating_add(255).min(self.until);
        let records = match self.log.read_range(Seq::new(from), Seq::new(end)) {
            Ok(records) => records,
            Err(error) => {
                self.cursor = None;
                return Some(Err(error));
            }
        };
        let expected = end.saturating_sub(from).saturating_add(1);
        if u64::try_from(records.len()).ok() != Some(expected)
            || records
                .iter()
                .zip(from..=end)
                .any(|(record, expected)| record.seq().get() != expected)
        {
            self.cursor = None;
            return Some(Err(EventLogError::Codec("gap in replay prefix".to_owned())));
        }
        self.cursor = if end == self.until {
            None
        } else {
            end.checked_add(1)
        };
        self.page.extend(records);
        self.page.pop_front().map(Ok)
    }
}
