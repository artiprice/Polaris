//! The single-writer owner of one session's telemetry.

use std::sync::Arc;

use arc_swap::ArcSwap;
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{ModelId, SessionId};

use crate::aggregate::Aggregates;
use crate::budget::{Budget, BudgetReport, BudgetStatus};
use crate::error::TelemetryError;
use crate::rate::RateWindow;
use crate::snapshot::{SnapshotReader, TelemetrySnapshot};

/// Telemetry of one session. One owner task mutates it (`&mut self`); any number of threads read
/// published snapshots through [`SnapshotReader`] without locks. It depends on neither the bus
/// nor a clock: the owner hands it events and the publication time.
#[derive(Debug)]
pub struct Telemetry {
    session: SessionId,
    aggregates: Aggregates,
    rate: RateWindow,
    published: Arc<ArcSwap<TelemetrySnapshot>>,
    last_record: Option<EventRecord>,
}

impl Telemetry {
    /// Empty telemetry of `session`; the initial snapshot is all zero, stamped with the Unix
    /// epoch.
    ///
    /// # Errors
    /// [`TelemetryError::Histogram`] if a latency histogram cannot be allocated.
    pub fn new(session: SessionId) -> Result<Self, TelemetryError> {
        let aggregates = Aggregates::new()?;
        let initial = Arc::new(aggregates.snapshot(session, Timestamp::UNIX_EPOCH));
        Ok(Self {
            session,
            aggregates,
            rate: RateWindow::default(),
            published: Arc::new(ArcSwap::new(initial)),
            last_record: None,
        })
    }

    /// The session this telemetry belongs to.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Folds one event without attribution or timestamp: totals and latencies are updated, the
    /// per-model table and the burn rate are not.
    pub fn record(&mut self, event: &Event) {
        self.aggregates.record(None, event);
    }

    /// Folds one record of this session, including its timestamp for the burn rate.
    ///
    /// # Errors
    /// [`TelemetryError::SessionMismatch`] (nothing is changed) for a record of another session.
    pub fn record_record(&mut self, record: &EventRecord) -> Result<(), TelemetryError> {
        self.fold(None, record, false)
    }

    /// Folds an event-log replay record with sequence-based duplicate protection.
    ///
    /// The same latest record may be replayed once without double counting. A different record
    /// with the same sequence or an older sequence is rejected. This is intended for sequential
    /// event-log replay; live callers that synthesize envelopes should use [`Self::record_record`].
    ///
    /// # Errors
    /// [`TelemetryError::SessionMismatch`] for a different session;
    /// [`TelemetryError::ConflictingSequence`] for a different record with the latest sequence;
    /// [`TelemetryError::OutOfOrder`] for an older sequence.
    pub fn record_replay_record(&mut self, record: &EventRecord) -> Result<(), TelemetryError> {
        self.fold(None, record, true)
    }

    /// Like [`Telemetry::record_record`], and also attributes a `ModelRequestCompleted` to
    /// `model` (events carry no model id; the caller reads it from the request record).
    ///
    /// # Errors
    /// [`TelemetryError::SessionMismatch`] (nothing is changed) for a record of another session.
    pub fn record_for_model(
        &mut self,
        model: &ModelId,
        record: &EventRecord,
    ) -> Result<(), TelemetryError> {
        self.fold(Some(model), record, false)
    }

    fn fold(
        &mut self,
        model: Option<&ModelId>,
        record: &EventRecord,
        replay_guard: bool,
    ) -> Result<(), TelemetryError> {
        if record.session() != self.session {
            return Err(TelemetryError::SessionMismatch {
                expected: self.session,
                found: record.session(),
            });
        }
        if replay_guard {
            if let Some(previous) = self.last_record {
                if record.seq() == previous.seq() {
                    if *record == previous {
                        return Ok(());
                    }
                    return Err(TelemetryError::ConflictingSequence(record.seq()));
                }
                if record.seq() < previous.seq() {
                    return Err(TelemetryError::OutOfOrder {
                        previous: previous.seq(),
                        found: record.seq(),
                    });
                }
            }
        }
        self.aggregates.record(model, record.event());
        if let Event::ModelRequestCompleted {
            prompt_tokens,
            completion_tokens,
            ..
        } = *record.event()
        {
            let billed =
                u64::from(prompt_tokens.get()).saturating_add(u64::from(completion_tokens.get()));
            self.rate.add(record.at().as_second(), billed);
        }
        if replay_guard {
            self.last_record = Some(*record);
        }
        Ok(())
    }

    /// Builds a snapshot stamped `at`, swaps it in for all readers and returns it. The owner
    /// calls this on its own schedule (for example every few hundred milliseconds).
    pub fn publish(&mut self, at: Timestamp) -> Arc<TelemetrySnapshot> {
        let mut snapshot = self.aggregates.snapshot(self.session, at);
        snapshot.recent_tokens_60s = self.rate.sum_at(at.as_second());
        let snapshot = Arc::new(snapshot);
        self.published.store(Arc::clone(&snapshot));
        snapshot
    }

    /// The most recently published snapshot (not the live state).
    #[must_use]
    pub fn snapshot(&self) -> Arc<TelemetrySnapshot> {
        self.published.load_full()
    }

    /// A handle other threads can use to read snapshots.
    #[must_use]
    pub fn reader(&self) -> SnapshotReader {
        SnapshotReader::new(Arc::clone(&self.published))
    }

    /// The live aggregates (for merging into a cross-session total).
    #[must_use]
    pub const fn aggregates(&self) -> &Aggregates {
        &self.aggregates
    }

    /// Both limits of `budget` against the live totals.
    #[must_use]
    pub fn budget_report(&self, budget: &Budget) -> BudgetReport {
        budget.check(self.aggregates.cost_total(), self.aggregates.tokens_used())
    }

    /// The more severe of the two budget statuses against the live totals.
    #[must_use]
    pub fn budget(&self, budget: &Budget) -> BudgetStatus {
        self.budget_report(budget).worst()
    }
}
