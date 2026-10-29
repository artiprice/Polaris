//! Fixed-capacity, replayable RPM/TPM admission snapshots.
//!
//! Each published snapshot is immutable. Writers copy a bounded snapshot into a claimed slot and
//! publish it with one versioned head compare-exchange. Reservation intents live in a separate
//! preallocated array, so settlement remains owned by the limiter even when no snapshot slot is
//! immediately available.

use crate::{DenyReason, Denied, RateLimitError, gcra::NANOS_PER_MINUTE};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;

const SNAPSHOTS: usize = 8;
const SNAPSHOT_MASK: u64 = 7;
const MAX_VERSION: u64 = u64::MAX >> 3;
const RESERVATIONS: usize = 16;
const LOG_CAPACITY: usize = 33;
const RECLAIMING: u64 = 0x8000_0000_0000_0000;
const READER_MASK: u64 = !RECLAIMING;
const INTENT_STATE_MASK: u64 = 7;
const OUTCOME_LOW_MASK: u64 = 0xffff_ffff;

const SLOT_FREE: u8 = 0;
const SLOT_WRITING: u8 = 1;
const SLOT_PUBLISHED: u8 = 2;
const SLOT_RETIRED: u8 = 3;

const INTENT_AVAILABLE: u64 = 0;
const INTENT_RESERVED: u64 = 1;
const INTENT_READY_OWNER: u64 = 2;
const INTENT_READY_DETACHED: u64 = 3;
const INTENT_APPLIED_OWNER: u64 = 4;

const EMPTY_RECORD: Record = Record {
    kind: 0,
    id: 0,
    time_ns: 0,
    cost: 0,
    delta_ns: 0,
    floor_ns: 0,
};

#[derive(Clone, Copy)]
struct Record {
    // 0 empty, 1 active reservation, 2 settled max-plus transform.
    kind: u64,
    id: u64,
    time_ns: u64,
    cost: u64,
    delta_ns: u64,
    floor_ns: u64,
}

struct AtomicRecord {
    kind: AtomicU64,
    id: AtomicU64,
    time_ns: AtomicU64,
    cost: AtomicU64,
    delta_ns: AtomicU64,
    floor_ns: AtomicU64,
}

impl AtomicRecord {
    fn new() -> Self {
        Self {
            kind: AtomicU64::new(0),
            id: AtomicU64::new(0),
            time_ns: AtomicU64::new(0),
            cost: AtomicU64::new(0),
            delta_ns: AtomicU64::new(0),
            floor_ns: AtomicU64::new(0),
        }
    }

    fn load(&self) -> Record {
        Record {
            kind: self.kind.load(Ordering::Relaxed),
            id: self.id.load(Ordering::Relaxed),
            time_ns: self.time_ns.load(Ordering::Relaxed),
            cost: self.cost.load(Ordering::Relaxed),
            delta_ns: self.delta_ns.load(Ordering::Relaxed),
            floor_ns: self.floor_ns.load(Ordering::Relaxed),
        }
    }

    fn store(&self, record: Record) {
        self.kind.store(record.kind, Ordering::Relaxed);
        self.id.store(record.id, Ordering::Relaxed);
        self.time_ns.store(record.time_ns, Ordering::Relaxed);
        self.cost.store(record.cost, Ordering::Relaxed);
        self.delta_ns.store(record.delta_ns, Ordering::Relaxed);
        self.floor_ns.store(record.floor_ns, Ordering::Relaxed);
    }
}

struct AtomicLog {
    count: AtomicU64,
    records: [AtomicRecord; LOG_CAPACITY],
}

impl AtomicLog {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            records: std::array::from_fn(|_| AtomicRecord::new()),
        }
    }

    fn load(&self) -> Result<Log, ()> {
        let count = usize::try_from(self.count.load(Ordering::Relaxed)).map_err(|_| ())?;
        if count > LOG_CAPACITY {
            return Err(());
        }
        let mut log = Log::empty();
        log.count = count;
        for (destination, source) in log.records.iter_mut().zip(self.records.iter()).take(count) {
            *destination = source.load();
        }
        Ok(log)
    }

    fn store(&self, log: &Log) -> Result<(), ()> {
        for (destination, source) in self.records.iter().zip(log.records.iter()).take(log.count) {
            destination.store(*source);
        }
        let count = u64::try_from(log.count).map_err(|_| ())?;
        self.count.store(count, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Log {
    count: usize,
    records: [Record; LOG_CAPACITY],
}

impl Log {
    const fn empty() -> Self {
        Self {
            count: 0,
            records: [EMPTY_RECORD; LOG_CAPACITY],
        }
    }

    fn push(&mut self, record: Record) -> Result<(), ()> {
        let destination = self.records.get_mut(self.count).ok_or(())?;
        *destination = record;
        self.count = self.count.checked_add(1).ok_or(())?;
        Ok(())
    }

    fn append_active(&mut self, id: u64, now_ns: u64, cost: u64) -> Result<(), ()> {
        self.push(Record {
            kind: 1,
            id,
            time_ns: now_ns,
            cost,
            delta_ns: 0,
            floor_ns: 0,
        })
    }

    fn find(&self, id: u64) -> Option<usize> {
        self.records
            .iter()
            .take(self.count)
            .position(|record| record.kind == 1 && record.id == id)
    }

    fn erase(&mut self, index: usize) -> Result<(), ()> {
        if index >= self.count {
            return Err(());
        }
        let mut compacted = Self::empty();
        for (position, record) in self.records.iter().copied().take(self.count).enumerate() {
            if position != index {
                compacted.push(record)?;
            }
        }
        *self = compacted;
        Ok(())
    }

    fn replace_with_transform(&mut self, index: usize, period_ns: u64, cost: u64) -> Result<(), ()> {
        let old = *self.records.get(index).ok_or(())?;
        let delta_ns = period_ns.checked_mul(cost).ok_or(())?;
        let floor_ns = old.time_ns.checked_add(delta_ns).ok_or(())?;
        let Some(record) = self.records.get_mut(index) else {
            return Err(());
        };
        *record = Record {
            kind: 2,
            id: 0,
            time_ns: 0,
            cost: 0,
            delta_ns,
            floor_ns,
        };
        self.compact();
        Ok(())
    }

    fn append_forced_charge(
        &mut self,
        time_ns: u64,
        period_ns: u64,
        cost: u64,
    ) -> Result<(), ()> {
        let delta_ns = period_ns.saturating_mul(cost);
        let floor_ns = time_ns.saturating_add(delta_ns);
        self.push(Record {
            kind: 2,
            id: 0,
            time_ns: 0,
            cost: 0,
            delta_ns,
            floor_ns,
        })?;
        self.compact();
        Ok(())
    }

    fn compact(&mut self) {
        let mut compacted = Self::empty();
        for record in self.records.iter().copied().take(self.count) {
            if record.kind == 2 {
                let previous = compacted.count.checked_sub(1);
                if let Some(previous) = previous
                    && let Some(left) = compacted.records.get_mut(previous)
                    && left.kind == 2
                {
                    left.delta_ns = left.delta_ns.saturating_add(record.delta_ns);
                    left.floor_ns = left
                        .floor_ns
                        .saturating_add(record.delta_ns)
                        .max(record.floor_ns);
                    continue;
                }
            }
            if compacted.push(record).is_err() {
                return;
            }
        }
        *self = compacted;
    }

    fn replay(&self, period_ns: u64) -> u64 {
        let mut tat = 0_u64;
        for record in self.records.iter().take(self.count) {
            let next = if record.kind == 1 {
                let delta = period_ns.saturating_mul(record.cost);
                record
                    .time_ns
                    .saturating_add(delta)
                    .max(tat.saturating_add(delta))
            } else {
                tat.saturating_add(record.delta_ns).max(record.floor_ns)
            };
            tat = next;
        }
        tat
    }
}

struct Snapshot {
    rpm: AtomicLog,
    tpm: AtomicLog,
    // Result of the most recent intent applied by this snapshot. Kept until acknowledged.
    result_slot: AtomicU64,
    result_generation: AtomicU64,
    result_refund: AtomicU64,
    result_retained: AtomicU64,
    result_overrun: AtomicU64,
}

impl Snapshot {
    fn new() -> Self {
        Self {
            rpm: AtomicLog::new(),
            tpm: AtomicLog::new(),
            result_slot: AtomicU64::new(u64::MAX),
            result_generation: AtomicU64::new(0),
            result_refund: AtomicU64::new(0),
            result_retained: AtomicU64::new(0),
            result_overrun: AtomicU64::new(0),
        }
    }

    fn copy_from(&self, source: &Snapshot) -> Result<(), ()> {
        self.rpm.store(&source.rpm.load()?)?;
        self.tpm.store(&source.tpm.load()?)?;
        self.result_slot
            .store(source.result_slot.load(Ordering::Relaxed), Ordering::Relaxed);
        self.result_generation.store(
            source.result_generation.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.result_refund
            .store(source.result_refund.load(Ordering::Relaxed), Ordering::Relaxed);
        self.result_retained.store(
            source.result_retained.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.result_overrun.store(
            source.result_overrun.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        Ok(())
    }

    fn clear_result(&self) {
        self.result_slot.store(u64::MAX, Ordering::Relaxed);
        self.result_generation.store(0, Ordering::Relaxed);
        self.result_refund.store(0, Ordering::Relaxed);
        self.result_retained.store(0, Ordering::Relaxed);
        self.result_overrun.store(0, Ordering::Relaxed);
    }
}

struct Intent {
    // generation * 8 + state; generation is never reused and exhaustion returns Busy.
    state: AtomicU64,
    actual_tokens: AtomicU64,
    now_ns: AtomicU64,
    kind: AtomicU64,
    // generation * 2^32 + refunded tokens; CAS prevents stale helpers overwriting a reuse.
    outcome: AtomicU64,
}

impl Intent {
    fn new() -> Self {
        Self {
            state: AtomicU64::new(INTENT_AVAILABLE),
            actual_tokens: AtomicU64::new(0),
            now_ns: AtomicU64::new(0),
            kind: AtomicU64::new(0),
            outcome: AtomicU64::new(0),
        }
    }
}

/// Reservation identity and preallocated intent-slot generation.
#[derive(Clone, Copy)]
pub(crate) struct ReservationId {
    slot: usize,
    generation: u64,
    id: u64,
    reserved_tokens: u64,
}

#[derive(Clone, Copy)]
struct LogConfig {
    period_ns: u64,
    tolerance_ns: u64,
    burst: u32,
    reason: DenyReason,
}

pub(crate) struct QuotaLedger {
    head: AtomicU64,
    readers: AtomicU64,
    slot_states: [AtomicU8; SNAPSHOTS],
    snapshots: [Snapshot; SNAPSHOTS],
    intents: [Intent; RESERVATIONS],
    rpm: LogConfig,
    tpm: LogConfig,
}

#[derive(Clone, Copy)]
struct IntentData {
    slot: usize,
    generation: u64,
    id: u64,
    actual_tokens: u64,
    now_ns: u64,
    kind: u64,
}

#[derive(Clone, Copy)]
enum Mutation {
    Reserve { id: u64, now_ns: u64, cost: u64 },
    Dispatch { id: u64 },
    Intent(IntentData),
}

struct SnapshotRead<'a> {
    ledger: &'a QuotaLedger,
    head: u64,
    slot: usize,
}

impl Drop for SnapshotRead<'_> {
    fn drop(&mut self) {
        let _ = self.ledger.readers.fetch_sub(1, Ordering::Release);
    }
}

impl QuotaLedger {
    pub(crate) fn new(
        rpm_per_minute: u32,
        rpm_burst: u32,
        tpm_per_minute: u32,
        tpm_burst: u32,
    ) -> Result<Self, RateLimitError> {
        let rpm = config(rpm_per_minute, rpm_burst, DenyReason::RpmExhausted)?;
        let tpm = config(tpm_per_minute, tpm_burst, DenyReason::TpmExhausted)?;
        Ok(Self {
            head: AtomicU64::new(0),
            readers: AtomicU64::new(0),
            slot_states: std::array::from_fn(|index| {
                AtomicU8::new(if index == 0 { SLOT_PUBLISHED } else { SLOT_FREE })
            }),
            snapshots: std::array::from_fn(|_| Snapshot::new()),
            intents: std::array::from_fn(|_| Intent::new()),
            rpm,
            tpm,
        })
    }

    pub(crate) fn reserve(&self, now_ns: u64, cost: u64) -> Result<ReservationId, Denied> {
        self.help_pending()?;
        if cost == 0 || cost > u64::from(self.tpm.burst) {
            return Err(Denied::new(DenyReason::ExceedsBurst, Duration::ZERO));
        }
        let (slot, generation) = self.claim_intent().map_err(|_| Denied::busy())?;
        let id = reservation_key(slot, generation).ok_or_else(Denied::accounting)?;
        match self.mutate(Mutation::Reserve { id, now_ns, cost }) {
            Ok(()) => Ok(ReservationId {
                slot,
                generation,
                id,
                reserved_tokens: cost,
            }),
            Err(MutationError::Denied(denied)) => {
                self.release_unpublished_intent(slot, generation);
                Err(denied)
            }
            Err(MutationError::Busy) => {
                self.release_unpublished_intent(slot, generation);
                Err(Denied::busy())
            }
        }
    }

    pub(crate) fn help_pending(&self) -> Result<(), Denied> {
        self.help_ready().map_err(|_| Denied::busy())
    }

    pub(crate) fn dispatch(&self, reservation: ReservationId) -> Result<(), RateLimitError> {
        self.help_ready().map_err(|_| RateLimitError::Busy)?;
        self.mutate(Mutation::Dispatch { id: reservation.id })
            .map_err(|error| match error {
                MutationError::Busy => RateLimitError::Busy,
                MutationError::Denied(_) => RateLimitError::Accounting,
            })
    }

    pub(crate) fn settle(
        &self,
        reservation: ReservationId,
        actual_tokens: u64,
        now_ns: u64,
    ) -> Result<(u64, bool), RateLimitError> {
        self.publish_intent(reservation, actual_tokens, now_ns, 1, false)?;
        let _ = self.help_ready();
        for _ in 0..4 {
            let Some(intent) = self.intents.get(reservation.slot) else {
                return Err(RateLimitError::Accounting);
            };
            let state = intent.state.load(Ordering::Acquire);
            if generation_part(state) != reservation.generation {
                return Err(RateLimitError::Accounting);
            }
            match state_part(state) {
                INTENT_APPLIED_OWNER => {
                    let Some(refunded) = self.read_outcome(reservation) else {
                        return Err(RateLimitError::Accounting);
                    };
                    let _ = intent.state.compare_exchange(
                        state,
                        pack_intent(reservation.generation, INTENT_AVAILABLE)
                            .ok_or(RateLimitError::Accounting)?,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    return Ok((refunded, false));
                }
                INTENT_READY_OWNER => {
                    if intent
                        .state
                        .compare_exchange(
                            state,
                            pack_intent(reservation.generation, INTENT_READY_DETACHED)
                                .ok_or(RateLimitError::Accounting)?,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return Ok((0, true));
                    }
                }
                _ => return Err(RateLimitError::Accounting),
            }
        }
        Ok((0, true))
    }

    pub(crate) fn cancel(&self, reservation: ReservationId, now_ns: u64) {
        let _ = self.publish_intent(reservation, 0, now_ns, 2, true);
        let _ = self.help_ready();
    }

    pub(crate) fn commit_drop(&self, reservation: ReservationId, now_ns: u64) {
        let _ = self.publish_intent(
            reservation,
            reservation.reserved_tokens,
            now_ns,
            1,
            true,
        );
        let _ = self.help_ready();
    }

    pub(crate) fn utilization(&self, now_ns: u64) -> (u8, u8) {
        let Some(read) = self.pin() else {
            return (100, 100);
        };
        let Some(snapshot) = self.snapshots.get(read.slot) else {
            return (100, 100);
        };
        let (Ok(rpm), Ok(tpm)) = (snapshot.rpm.load(), snapshot.tpm.load()) else {
            return (100, 100);
        };
        (
            utilization(rpm.replay(self.rpm.period_ns), now_ns, self.rpm),
            utilization(tpm.replay(self.tpm.period_ns), now_ns, self.tpm),
        )
    }

    pub(crate) fn retry_after_pct(&self, now_ns: u64, threshold_pct: u8) -> Duration {
        let Some(read) = self.pin() else {
            return Duration::ZERO;
        };
        let Some(snapshot) = self.snapshots.get(read.slot) else {
            return Duration::ZERO;
        };
        let (Ok(rpm_log), Ok(tpm_log)) = (snapshot.rpm.load(), snapshot.tpm.load()) else {
            return Duration::ZERO;
        };
        let rpm = retry_pct(
            rpm_log.replay(self.rpm.period_ns),
            now_ns,
            threshold_pct,
            self.rpm,
        );
        let tpm = retry_pct(
            tpm_log.replay(self.tpm.period_ns),
            now_ns,
            threshold_pct,
            self.tpm,
        );
        rpm.max(tpm)
    }

    fn claim_intent(&self) -> Result<(usize, u64), ()> {
        for (index, intent) in self.intents.iter().enumerate() {
            let mut observed = intent.state.load(Ordering::Acquire);
            if state_part(observed) != INTENT_AVAILABLE {
                continue;
            }
            let generation = generation_part(observed).checked_add(1).ok_or(())?;
            if u32::try_from(generation).is_err() {
                return Err(());
            }
            let Some(next) = pack_intent(generation, INTENT_RESERVED) else {
                return Err(());
            };
            match intent.state.compare_exchange(
                observed,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let Some(marker) = generation.checked_shl(32) else {
                        self.release_unpublished_intent(index, generation);
                        return Err(());
                    };
                    intent.outcome.store(marker, Ordering::Release);
                    return Ok((index, generation));
                }
                Err(actual) => observed = actual,
            }
            let _ = observed;
        }
        Err(())
    }

    fn release_unpublished_intent(&self, slot: usize, generation: u64) {
        let Some(intent) = self.intents.get(slot) else {
            return;
        };
        let (Some(expected), Some(available)) = (
            pack_intent(generation, INTENT_RESERVED),
            pack_intent(generation, INTENT_AVAILABLE),
        ) else {
            return;
        };
        let _ = intent.state.compare_exchange(
            expected, available, Ordering::AcqRel, Ordering::Acquire,
        );
    }

    fn publish_intent(
        &self,
        reservation: ReservationId,
        actual_tokens: u64,
        now_ns: u64,
        kind: u64,
        detached: bool,
    ) -> Result<(), RateLimitError> {
        let intent = self.intents.get(reservation.slot).ok_or(RateLimitError::Accounting)?;
        let expected = pack_intent(reservation.generation, INTENT_RESERVED)
            .ok_or(RateLimitError::Accounting)?;
        if intent.state.load(Ordering::Acquire) != expected {
            return Err(RateLimitError::Accounting);
        }
        intent.actual_tokens.store(actual_tokens, Ordering::Relaxed);
        intent.now_ns.store(now_ns, Ordering::Relaxed);
        intent.kind.store(kind, Ordering::Relaxed);
        let ready_state = if detached {
            INTENT_READY_DETACHED
        } else {
            INTENT_READY_OWNER
        };
        intent
            .state
            .store(
                pack_intent(reservation.generation, ready_state)
                    .ok_or(RateLimitError::Accounting)?,
                Ordering::Release,
            );
        Ok(())
    }

    fn read_outcome(&self, reservation: ReservationId) -> Option<u64> {
        let intent = self.intents.get(reservation.slot)?;
        let state = intent.state.load(Ordering::Acquire);
        if generation_part(state) != reservation.generation
            || state_part(state) != INTENT_APPLIED_OWNER
        {
            return None;
        }
        let word = intent.outcome.load(Ordering::Acquire);
        (word >> 32 == reservation.generation).then_some(word & OUTCOME_LOW_MASK)
    }

    fn help_ready(&self) -> Result<(), ()> {
        self.help_published_result()?;
        for _ in 0..RESERVATIONS {
            let Some(intent) = self.first_ready() else {
                return Ok(());
            };
            match self.mutate(Mutation::Intent(intent)) {
                Ok(()) => self.help_published_result()?,
                Err(_) => return Err(()),
            }
        }
        if self.first_ready().is_some() { Err(()) } else { Ok(()) }
    }

    fn first_ready(&self) -> Option<IntentData> {
        let mut selected: Option<IntentData> = None;
        for (slot, intent) in self.intents.iter().enumerate() {
            let state = intent.state.load(Ordering::Acquire);
            let phase = state_part(state);
            if phase != INTENT_READY_OWNER && phase != INTENT_READY_DETACHED {
                continue;
            }
            let generation = generation_part(state);
            let Some(id) = reservation_key(slot, generation) else {
                continue;
            };
            let data = IntentData {
                slot,
                generation,
                id,
                actual_tokens: intent.actual_tokens.load(Ordering::Relaxed),
                now_ns: intent.now_ns.load(Ordering::Relaxed),
                kind: intent.kind.load(Ordering::Relaxed),
            };
            if intent.state.load(Ordering::Acquire) != state {
                continue;
            }
            let replace = match selected {
                None => true,
                Some(existing) => data.now_ns < existing.now_ns,
            };
            if replace {
                selected = Some(data);
            }
        }
        selected
    }

    fn help_published_result(&self) -> Result<(), ()> {
        let Some(read) = self.pin() else { return Err(()) };
        let snapshot = self.snapshots.get(read.slot).ok_or(())?;
        self.ack_snapshot_result(snapshot)
    }

    fn ack_snapshot_result(&self, snapshot: &Snapshot) -> Result<(), ()> {
        let slot = snapshot.result_slot.load(Ordering::Relaxed);
        if slot == u64::MAX {
            return Ok(());
        }
        let generation = snapshot.result_generation.load(Ordering::Relaxed);
        let refund = snapshot.result_refund.load(Ordering::Relaxed);
        let slot_index = usize::try_from(slot).map_err(|_| ())?;
        let intent = self.intents.get(slot_index).ok_or(())?;
        let expected_outcome = generation.checked_shl(32).ok_or(())?;
        let result_word = expected_outcome | (refund & OUTCOME_LOW_MASK);
        match intent.outcome.compare_exchange(
            expected_outcome,
            result_word,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {}
            Err(observed) if observed == result_word => {}
            Err(_) => return Ok(()),
        }
        for _ in 0..3 {
            let current = intent.state.load(Ordering::Acquire);
            if generation_part(current) != generation {
                return Ok(());
            }
            let phase = state_part(current);
            let next_phase = match phase {
                INTENT_READY_OWNER => INTENT_APPLIED_OWNER,
                INTENT_READY_DETACHED => INTENT_AVAILABLE,
                INTENT_APPLIED_OWNER | INTENT_AVAILABLE => return Ok(()),
                _ => return Ok(()),
            };
            if intent
                .state
                .compare_exchange(
                    current,
                pack_intent(generation, next_phase).ok_or(())?,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
        Ok(())
    }

    fn mutate(&self, mutation: Mutation) -> Result<(), MutationError> {
        for _ in 0..8 {
            let Some(read) = self.pin() else { return Err(MutationError::Busy) };
            let Some(source) = self.snapshots.get(read.slot) else {
                return Err(MutationError::Busy);
            };
            if self.ack_snapshot_result(source).is_err() {
                return Err(MutationError::Busy);
            }
            let Some(target) = self.claim_snapshot() else {
                drop(read);
                self.reclaim();
                continue;
            };
            let Some(candidate) = self.snapshots.get(target) else {
                return Err(MutationError::Busy);
            };
            if candidate.copy_from(source).is_err() {
                if let Some(state) = self.slot_states.get(target) {
                    state.store(SLOT_FREE, Ordering::Release);
                }
                return Err(MutationError::Busy);
            }
            if let Err(error) = apply(candidate, mutation, self.rpm, self.tpm) {
                if let Some(state) = self.slot_states.get(target) {
                    state.store(SLOT_FREE, Ordering::Release);
                }
                return Err(error);
            }
            let version = read.head >> 3;
            let Some(next_version) = version.checked_add(1).filter(|v| *v <= MAX_VERSION) else {
                if let Some(state) = self.slot_states.get(target) {
                    state.store(SLOT_FREE, Ordering::Release);
                }
                return Err(MutationError::Busy);
            };
            let Some(version_bits) = next_version.checked_mul(8) else {
                if let Some(state) = self.slot_states.get(target) {
                    state.store(SLOT_FREE, Ordering::Release);
                }
                return Err(MutationError::Busy);
            };
            let Ok(target_bits) = u64::try_from(target) else {
                if let Some(state) = self.slot_states.get(target) {
                    state.store(SLOT_FREE, Ordering::Release);
                }
                return Err(MutationError::Busy);
            };
            let next_head = version_bits | target_bits;
            let Some(target_state) = self.slot_states.get(target) else {
                return Err(MutationError::Busy);
            };
            target_state.store(SLOT_PUBLISHED, Ordering::Release);
            match self.head.compare_exchange(
                read.head,
                next_head,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    if let Some(state) = self.slot_states.get(read.slot) {
                        state.store(SLOT_RETIRED, Ordering::Release);
                    }
                    return Ok(());
                }
                Err(_) => {
                    target_state.store(SLOT_FREE, Ordering::Release);
                    continue;
                }
            }
        }
        Err(MutationError::Busy)
    }

    fn pin(&self) -> Option<SnapshotRead<'_>> {
        for _ in 0..4 {
            let observed = self.readers.load(Ordering::Acquire);
            if observed & RECLAIMING != 0 || observed & READER_MASK == READER_MASK {
                return None;
            }
            if self
                .readers
                .compare_exchange_weak(
                    observed,
                    match observed.checked_add(1) {
                        Some(next) => next,
                        None => return None,
                    },
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                continue;
            }
            let head = self.head.load(Ordering::Acquire);
            if self.readers.load(Ordering::Acquire) & RECLAIMING == 0 {
                let Ok(slot) = usize::try_from(head & SNAPSHOT_MASK) else {
                    let _ = self.readers.fetch_sub(1, Ordering::Release);
                    return None;
                };
                return Some(SnapshotRead {
                    ledger: self,
                    head,
                    slot,
                });
            }
            let _ = self.readers.fetch_sub(1, Ordering::Release);
            return None;
        }
        None
    }

    fn claim_snapshot(&self) -> Option<usize> {
        self.slot_states.iter().enumerate().find_map(|(index, state)| {
            if state
                .compare_exchange(
                    SLOT_FREE,
                    SLOT_WRITING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                Some(index)
            } else {
                None
            }
        })
    }

    fn reclaim(&self) {
        if self
            .readers
            .compare_exchange(0, RECLAIMING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let head = self.head.load(Ordering::Acquire) & SNAPSHOT_MASK;
        let Ok(head_slot) = usize::try_from(head) else {
            self.readers.store(0, Ordering::Release);
            return;
        };
        for (index, state) in self.slot_states.iter().enumerate() {
            if index != head_slot {
                let _ = state.compare_exchange(
                    SLOT_RETIRED,
                    SLOT_FREE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        self.readers.store(0, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
enum MutationError {
    Busy,
    Denied(Denied),
}

fn apply(
    snapshot: &Snapshot,
    mutation: Mutation,
    rpm: LogConfig,
    tpm: LogConfig,
) -> Result<(), MutationError> {
    let mut rpm_log = snapshot.rpm.load().map_err(|_| MutationError::Busy)?;
    let mut tpm_log = snapshot.tpm.load().map_err(|_| MutationError::Busy)?;
    snapshot.clear_result();
    match mutation {
        Mutation::Reserve { id, now_ns, cost } => {
            let next_rpm = rpm_log
                .replay(rpm.period_ns)
                .max(now_ns)
                .saturating_add(rpm.period_ns);
            let next_tpm = tpm_log
                .replay(tpm.period_ns)
                .max(now_ns)
                .checked_add(tpm.period_ns.checked_mul(cost).ok_or_else(|| {
                    MutationError::Denied(Denied::accounting())
                })?)
                .ok_or_else(|| MutationError::Denied(Denied::accounting()))?;
            check_capacity(next_rpm, now_ns, rpm)?;
            check_capacity(next_tpm, now_ns, tpm)?;
            rpm_log
                .append_active(id, now_ns, 1)
                .map_err(|_| MutationError::Busy)?;
            tpm_log
                .append_active(id, now_ns, cost)
                .map_err(|_| MutationError::Busy)?;
        }
        Mutation::Dispatch { id } => {
            let index = rpm_log.find(id).ok_or(MutationError::Busy)?;
            rpm_log
                .replace_with_transform(index, rpm.period_ns, 1)
                .map_err(|_| MutationError::Denied(Denied::accounting()))?;
        }
        Mutation::Intent(intent) if intent.kind == 2 => {
            let rpm_index = rpm_log.find(intent.id).ok_or(MutationError::Busy)?;
            let tpm_index = tpm_log.find(intent.id).ok_or(MutationError::Busy)?;
            rpm_log.erase(rpm_index).map_err(|_| MutationError::Busy)?;
            tpm_log.erase(tpm_index).map_err(|_| MutationError::Busy)?;
            rpm_log.compact();
            tpm_log.compact();
            let result_slot = u64::try_from(intent.slot).map_err(|_| MutationError::Busy)?;
            snapshot.result_slot.store(result_slot, Ordering::Relaxed);
            snapshot
                .result_generation
                .store(intent.generation, Ordering::Relaxed);
        }
        Mutation::Intent(intent) => {
            let index = tpm_log.find(intent.id).ok_or(MutationError::Busy)?;
            let old_tail = tpm_log.replay(tpm.period_ns);
            let reserved = tpm_log
                .records
                .get(index)
                .ok_or(MutationError::Busy)?
                .cost;
            let overrun = intent.actual_tokens > reserved;
            let actual = intent.actual_tokens;
            let charged = if overrun { reserved } else { actual };
            tpm_log
                .replace_with_transform(index, tpm.period_ns, charged)
                .map_err(|_| MutationError::Denied(Denied::accounting()))?;
            if overrun {
                let cost_delta = actual.saturating_sub(reserved);
                tpm_log
                    .append_forced_charge(intent.now_ns, tpm.period_ns, cost_delta)
                    .map_err(|_| MutationError::Busy)?;
            }
            let new_tail = tpm_log.replay(tpm.period_ns);
            let unused = reserved.saturating_sub(actual);
            let old_at_settlement = old_tail.max(intent.now_ns);
            let new_at_settlement = new_tail.max(intent.now_ns);
            let refundable_ns = old_at_settlement.saturating_sub(new_at_settlement);
            let refundable = refundable_ns
                .checked_div(tpm.period_ns)
                .ok_or(MutationError::Denied(Denied::accounting()))?;
            let refund = if overrun { 0 } else { unused.min(refundable) };
            let result_slot = u64::try_from(intent.slot).map_err(|_| MutationError::Busy)?;
            snapshot.result_slot.store(result_slot, Ordering::Relaxed);
            snapshot
                .result_generation
                .store(intent.generation, Ordering::Relaxed);
            snapshot.result_refund.store(refund, Ordering::Relaxed);
            snapshot
                .result_retained
                .store(unused.saturating_sub(refund), Ordering::Relaxed);
            snapshot
                .result_overrun
                .store(u64::from(overrun), Ordering::Relaxed);
        }
    }
    snapshot
        .rpm
        .store(&rpm_log)
        .map_err(|_| MutationError::Busy)?;
    snapshot
        .tpm
        .store(&tpm_log)
        .map_err(|_| MutationError::Busy)?;
    Ok(())
}

fn check_capacity(next: u64, now_ns: u64, config: LogConfig) -> Result<(), MutationError> {
    let horizon = config
        .tolerance_ns
        .checked_add(config.period_ns)
        .ok_or_else(|| MutationError::Denied(Denied::accounting()))?;
    let capacity = now_ns
        .checked_add(horizon)
        .ok_or_else(|| MutationError::Denied(Denied::accounting()))?;
    if next > capacity {
        Err(MutationError::Denied(Denied::new(
            config.reason,
            Duration::from_nanos(next.saturating_sub(capacity)),
        )))
    } else {
        Ok(())
    }
}

fn config(rate: u32, burst: u32, reason: DenyReason) -> Result<LogConfig, RateLimitError> {
    if rate == 0 || burst == 0 {
        return Err(RateLimitError::InvalidQuota);
    }
    let rate = u64::from(rate);
    let period_ns = NANOS_PER_MINUTE.div_ceil(rate);
    let burst_minus_one = burst
        .checked_sub(1)
        .ok_or(RateLimitError::InvalidQuota)?;
    let tolerance_ns = period_ns
        .checked_mul(u64::from(burst_minus_one))
        .ok_or(RateLimitError::InvalidQuota)?;
    Ok(LogConfig {
        period_ns,
        tolerance_ns,
        burst,
        reason,
    })
}

fn utilization(tat: u64, now_ns: u64, config: LogConfig) -> u8 {
    let debt = tat.saturating_sub(now_ns);
    let horizon = config.tolerance_ns.saturating_add(config.period_ns).max(1);
    let pct = debt.saturating_mul(100).saturating_div(horizon).min(100);
    match u8::try_from(pct) {
        Ok(value) => value,
        Err(_) => 100,
    }
}

fn retry_pct(tat: u64, now_ns: u64, threshold_pct: u8, config: LogConfig) -> Duration {
    let debt = tat.saturating_sub(now_ns);
    let horizon = config.tolerance_ns.saturating_add(config.period_ns).max(1);
    let allowed = horizon
        .saturating_mul(u64::from(threshold_pct))
        .saturating_div(100);
    Duration::from_nanos(debt.saturating_sub(allowed))
}

fn reservation_key(slot: usize, generation: u64) -> Option<u64> {
    let slot = u64::try_from(slot).ok()?;
    let reservation_slots = u64::try_from(RESERVATIONS).ok()?;
    generation.checked_mul(reservation_slots)?.checked_add(slot)
}

fn pack_intent(generation: u64, state: u64) -> Option<u64> {
    Some(generation.checked_mul(8)?.checked_add(state)?)
}

fn generation_part(state: u64) -> u64 {
    state >> 3
}

fn state_part(state: u64) -> u64 {
    state & INTENT_STATE_MASK
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-ratelimit/src/ledger.rs"]
mod tests;
