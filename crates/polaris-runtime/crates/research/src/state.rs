//! Pure experiment transitions; no model response can directly mark an experiment kept or confirmed.
use polaris_sdk::ids::Cid;
use std::collections::BTreeMap;

#[path = "checkpoint.rs"]
mod checkpoint;

/// Absolute ledger bound, independent of the operator's requested budget.
const MAX_EXPERIMENTS: u32 = 100_000;

/// Optimisation direction fixed by the immutable program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Lower values are better.
    Min,
    /// Higher values are better.
    Max,
}

/// Finite metric value; NaN and infinities cannot enter the comparison state.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct MetricValue(f64);
impl MetricValue {
    /// Validate a measured metric.
    ///
    /// # Errors
    /// Returns [`ResearchError::NonFinite`] for NaN or either infinity.
    pub fn new(value: f64) -> Result<Self, ResearchError> {
        if !value.is_finite() {
            return Err(ResearchError::NonFinite);
        }
        Ok(Self(value))
    }
    /// Return the finite underlying value.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// Measured or reserved resources. Wall time is expressed in milliseconds, cost in micro-USD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    wall_ms: u64,
    tokens: u64,
    micro_usd: u64,
}
impl Usage {
    /// Build a resource quantity. Zero token/cost usage is legitimate for local experiments.
    #[must_use]
    pub const fn new(wall_ms: u64, tokens: u64, micro_usd: u64) -> Self {
        Self {
            wall_ms,
            tokens,
            micro_usd,
        }
    }
    /// Accounted wall time in milliseconds.
    #[must_use]
    pub const fn wall_ms(self) -> u64 {
        self.wall_ms
    }
    /// Accounted model tokens.
    #[must_use]
    pub const fn tokens(self) -> u64 {
        self.tokens
    }
    /// Accounted cost in micro-USD.
    #[must_use]
    pub const fn micro_usd(self) -> u64 {
        self.micro_usd
    }
    fn fits(self, limit: Self) -> bool {
        self.wall_ms <= limit.wall_ms
            && self.tokens <= limit.tokens
            && self.micro_usd <= limit.micro_usd
    }
    fn component_max(self, other: Self) -> Self {
        Self {
            wall_ms: self.wall_ms.max(other.wall_ms),
            tokens: self.tokens.max(other.tokens),
            micro_usd: self.micro_usd.max(other.micro_usd),
        }
    }
    fn add(self, other: Self) -> Result<Self, ResearchError> {
        Ok(Self {
            wall_ms: self
                .wall_ms
                .checked_add(other.wall_ms)
                .ok_or(ResearchError::Overflow)?,
            tokens: self
                .tokens
                .checked_add(other.tokens)
                .ok_or(ResearchError::Overflow)?,
            micro_usd: self
                .micro_usd
                .checked_add(other.micro_usd)
                .ok_or(ResearchError::Overflow)?,
        })
    }
    fn sub(self, other: Self) -> Result<Self, ResearchError> {
        Ok(Self {
            wall_ms: self
                .wall_ms
                .checked_sub(other.wall_ms)
                .ok_or(ResearchError::Invariant)?,
            tokens: self
                .tokens
                .checked_sub(other.tokens)
                .ok_or(ResearchError::Invariant)?,
            micro_usd: self
                .micro_usd
                .checked_sub(other.micro_usd)
                .ok_or(ResearchError::Invariant)?,
        })
    }
}

/// Positive limits for the complete research run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    resources: Usage,
    experiments: u32,
}
impl Budget {
    /// Validate total run limits.
    ///
    /// # Errors
    /// Returns [`ResearchError::Budget`] for zero or unbounded experiment limits.
    pub fn new(
        wall_ms: u64,
        tokens: u64,
        micro_usd: u64,
        experiments: u32,
    ) -> Result<Self, ResearchError> {
        if wall_ms == 0
            || tokens == 0
            || micro_usd == 0
            || experiments == 0
            || experiments > MAX_EXPERIMENTS
        {
            return Err(ResearchError::Budget);
        }
        Ok(Self {
            resources: Usage::new(wall_ms, tokens, micro_usd),
            experiments,
        })
    }
    /// Total resource limits.
    #[must_use]
    pub const fn resources(self) -> Usage {
        self.resources
    }
    /// Maximum completed or interrupted experiments.
    #[must_use]
    pub const fn experiments(self) -> u32 {
        self.experiments
    }
}

/// Run-local monotone experiment identity; only the state machine issues identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExperimentId(u32);
impl ExperimentId {
    /// Return the run-local ordinal, starting at one.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Requested workspace disposition. Ties and unsuccessful measurements always revert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Strict metric improvement relative to the last acknowledged baseline.
    Keep,
    /// Restore the acknowledged baseline.
    Revert,
}

/// Measurement linked to the immutable raw run evidence by content ID.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation {
    metric: MetricValue,
    evidence: Cid,
    usage: Usage,
}
impl Observation {
    /// Attach a validated metric to its evidence and actual usage.
    #[must_use]
    pub const fn new(metric: MetricValue, evidence: Cid, usage: Usage) -> Self {
        Self {
            metric,
            evidence,
            usage,
        }
    }
    /// Metric measured by the fixed harness.
    #[must_use]
    pub const fn metric(self) -> MetricValue {
        self.metric
    }
    /// Content ID of the raw measurement evidence.
    #[must_use]
    pub const fn evidence(self) -> Cid {
        self.evidence
    }
    /// Actual usage reported by the trusted runner.
    #[must_use]
    pub const fn usage(self) -> Usage {
        self.usage
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InFlight {
    Running {
        id: ExperimentId,
        reservation: Usage,
    },
    Measured {
        id: ExperimentId,
        reservation: Usage,
        observation: Observation,
        decision: Decision,
    },
    Recovery {
        id: ExperimentId,
        charge: Usage,
    },
}
impl InFlight {
    const fn id(self) -> ExperimentId {
        match self {
            Self::Running { id, .. } | Self::Measured { id, .. } | Self::Recovery { id, .. } => id,
        }
    }
    const fn charge(self) -> Usage {
        match self {
            Self::Running { reservation, .. } | Self::Measured { reservation, .. } => reservation,
            Self::Recovery { charge, .. } => charge,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Skeptic {
    Pending,
    Confirmed(Cid),
    Refuted(Cid),
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Record {
    charge: Usage,
    observation: Option<Observation>,
    decision: Decision,
    receipt: Cid,
    skeptic: Skeptic,
}

/// Opaque, owned recovery snapshot. Its fields cannot be forged through public constructors.
///
/// Persisting this value is an adapter responsibility; cloning a value alone is not crash durability.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    state: ResearchState,
}

/// Deterministic decision authority for one immutable research program.
///
/// Resource reservations are charged before execution. A crash forfeits the reservation. A
/// reported overrun is charged at least its actual usage and permanently stops new experiments.
/// Only an acknowledged completed experiment can refund unused reserved resources.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchState {
    program: Cid,
    direction: Direction,
    budget: Budget,
    spent: Usage,
    halted: bool,
    next: u32,
    completed: u32,
    best: Option<MetricValue>,
    plateau: u32,
    in_flight: Option<InFlight>,
    records: BTreeMap<ExperimentId, Record>,
}

impl ResearchState {
    /// Start a run with positive bounded limits and an immutable program digest.
    ///
    /// # Errors
    /// Returns [`ResearchError::Budget`] if the defensive ledger bound is exceeded.
    pub fn new(program: Cid, direction: Direction, budget: Budget) -> Result<Self, ResearchError> {
        if budget.experiments > MAX_EXPERIMENTS {
            return Err(ResearchError::Budget);
        }
        Ok(Self {
            program,
            direction,
            budget,
            spent: Usage::default(),
            halted: false,
            next: 0,
            completed: 0,
            best: None,
            plateau: 0,
            in_flight: None,
            records: BTreeMap::new(),
        })
    }
    /// Reserve the entire worst-case experiment cost before invoking any external runner.
    ///
    /// # Errors
    /// Rejects concurrent/incomplete work, exhausted budgets, zero wall time and arithmetic overflow.
    pub fn begin(&mut self, reservation: Usage) -> Result<ExperimentId, ResearchError> {
        if self.halted {
            return Err(ResearchError::Overrun);
        }
        if self.in_flight.is_some() {
            return Err(ResearchError::Unsettled);
        }
        if reservation.wall_ms == 0 || self.completed >= self.budget.experiments {
            return Err(ResearchError::Budget);
        }
        let spent = self.spent.add(reservation)?;
        if !spent.fits(self.budget.resources) {
            return Err(ResearchError::Budget);
        }
        let next = self.next.checked_add(1).ok_or(ResearchError::Overflow)?;
        if next > self.budget.experiments {
            return Err(ResearchError::Budget);
        }
        let id = ExperimentId(next);
        self.spent = spent;
        self.next = next;
        self.in_flight = Some(InFlight::Running { id, reservation });
        Ok(id)
    }
    /// Compare actual measurement with the last acknowledged baseline.
    ///
    /// # Errors
    /// A wrong/stale ID leaves state unchanged. An overrun moves the correct experiment into
    /// recovery, retains at least its full reservation, and permanently halts new experiments.
    pub fn measure(
        &mut self,
        id: ExperimentId,
        observation: Observation,
    ) -> Result<Decision, ResearchError> {
        let Some(InFlight::Running {
            id: active,
            reservation,
        }) = self.in_flight
        else {
            return Err(ResearchError::Unsettled);
        };
        if id != active {
            return Err(ResearchError::Stale);
        }
        if !observation.usage.fits(reservation) {
            let charge = reservation.component_max(observation.usage);
            self.in_flight = Some(InFlight::Recovery { id, charge });
            self.halted = true;
            self.spent = self.spent.sub(reservation)?.add(charge)?;
            return Err(ResearchError::Overrun);
        }
        let improvement = match self.best {
            None => true,
            Some(best) => match self.direction {
                Direction::Min => observation.metric.0 < best.0,
                Direction::Max => observation.metric.0 > best.0,
            },
        };
        let decision = if improvement {
            Decision::Keep
        } else {
            Decision::Revert
        };
        self.in_flight = Some(InFlight::Measured {
            id,
            reservation,
            observation,
            decision,
        });
        Ok(decision)
    }
    /// Acknowledge the trusted workspace owner's completed keep/revert operation.
    ///
    /// # Errors
    /// Rejects mismatched IDs/decisions, reuse of measurement evidence as a mutation receipt, or
    /// missing measurements. A failed acknowledgement changes no baseline or accounting.
    pub fn settle(
        &mut self,
        id: ExperimentId,
        applied: Decision,
        receipt: Cid,
    ) -> Result<(), ResearchError> {
        let Some(InFlight::Measured {
            id: active,
            reservation,
            observation,
            decision,
        }) = self.in_flight
        else {
            return Err(ResearchError::Unsettled);
        };
        if id != active {
            return Err(ResearchError::Stale);
        }
        if applied != decision || receipt == observation.evidence {
            return Err(ResearchError::Receipt);
        }
        let spent = self.spent.sub(reservation)?.add(observation.usage)?;
        let completed = self
            .completed
            .checked_add(1)
            .ok_or(ResearchError::Overflow)?;
        let plateau = if decision == Decision::Keep {
            0
        } else {
            self.plateau.checked_add(1).ok_or(ResearchError::Overflow)?
        };
        let record = Record {
            charge: observation.usage,
            observation: Some(observation),
            decision,
            receipt,
            skeptic: Skeptic::Pending,
        };
        if self.records.contains_key(&id) {
            return Err(ResearchError::Invariant);
        }
        self.records.insert(id, record);
        if decision == Decision::Keep {
            self.best = Some(observation.metric);
        }
        self.spent = spent;
        self.completed = completed;
        self.plateau = plateau;
        self.in_flight = None;
        Ok(())
    }
    /// Mark a failed/interrupted experiment for baseline recovery before any further work.
    ///
    /// # Errors
    /// Rejects absent or stale experiment IDs. The original reservation remains charged.
    pub fn interrupt(&mut self, id: ExperimentId) -> Result<(), ResearchError> {
        let active = self.in_flight.ok_or(ResearchError::Unsettled)?;
        if active.id() != id {
            return Err(ResearchError::Stale);
        }
        self.in_flight = Some(InFlight::Recovery {
            id,
            charge: active.charge(),
        });
        Ok(())
    }
    /// Record proof that the workspace owner restored the last acknowledged baseline.
    ///
    /// # Errors
    /// Rejects stale IDs or recovery of a normally running/measured experiment.
    pub fn recovered(&mut self, id: ExperimentId, receipt: Cid) -> Result<(), ResearchError> {
        let Some(InFlight::Recovery { id: active, charge }) = self.in_flight else {
            return Err(ResearchError::Unsettled);
        };
        if id != active {
            return Err(ResearchError::Stale);
        }
        let completed = self
            .completed
            .checked_add(1)
            .ok_or(ResearchError::Overflow)?;
        let plateau = self.plateau.checked_add(1).ok_or(ResearchError::Overflow)?;
        if self.records.contains_key(&id) {
            return Err(ResearchError::Invariant);
        }
        self.records.insert(
            id,
            Record {
                charge,
                observation: None,
                decision: Decision::Revert,
                receipt,
                skeptic: Skeptic::Pending,
            },
        );
        self.completed = completed;
        self.plateau = plateau;
        self.in_flight = None;
        Ok(())
    }
    /// Record the independent skeptic's terminal verdict for an acknowledged improvement.
    ///
    /// The caller must enforce that the skeptic frame is read-only and actually ran independently;
    /// distinct CIDs prevent trivial reuse but do not prove semantic independence by themselves.
    ///
    /// # Errors
    /// Rejects unacknowledged/reverted records, repeated verdicts, or evidence equal to the
    /// experiment's measurement or mutation receipt.
    pub fn confirm(
        &mut self,
        id: ExperimentId,
        evidence: Cid,
        unrefuted: bool,
    ) -> Result<(), ResearchError> {
        let record = self.records.get_mut(&id).ok_or(ResearchError::Stale)?;
        let observation = record.observation.ok_or(ResearchError::Evidence)?;
        if record.decision != Decision::Keep
            || record.skeptic != Skeptic::Pending
            || evidence == record.receipt
            || evidence == observation.evidence
        {
            return Err(ResearchError::Evidence);
        }
        record.skeptic = if unrefuted {
            Skeptic::Confirmed(evidence)
        } else {
            Skeptic::Refuted(evidence)
        };
        Ok(())
    }
    /// Snapshot state. Persist before external work and after each acknowledgement.
    #[must_use]
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            state: self.clone(),
        }
    }
    /// Content ID of the immutable program bound to this run.
    #[must_use]
    pub const fn program(&self) -> Cid {
        self.program
    }
    /// Restore the same immutable program; unresolved external work always requires recovery.
    ///
    /// # Errors
    /// Returns [`ResearchError::ProgramChanged`] if the program's content ID differs.
    pub fn resume(program: Cid, checkpoint: Checkpoint) -> Result<Self, ResearchError> {
        let mut state = checkpoint.state;
        if program != state.program {
            return Err(ResearchError::ProgramChanged);
        }
        if let Some(active) = state.in_flight {
            state.in_flight = Some(InFlight::Recovery {
                id: active.id(),
                charge: active.charge(),
            });
        }
        Ok(state)
    }
    /// In-flight experiment requiring a trusted workspace recovery receipt.
    #[must_use]
    pub const fn recovery_required(&self) -> Option<ExperimentId> {
        match self.in_flight {
            Some(InFlight::Recovery { id, .. }) => Some(id),
            _ => None,
        }
    }
    /// Conservative total usage including the full current reservation.
    #[must_use]
    pub const fn spent(&self) -> Usage {
        self.spent
    }
    /// Whether a reported resource overrun permanently halted further experiments.
    #[must_use]
    pub const fn halted(&self) -> bool {
        self.halted
    }
    /// Number of acknowledged complete or recovered experiments.
    #[must_use]
    pub const fn completed(&self) -> u32 {
        self.completed
    }
    /// Last acknowledged best metric. Confirmation is reported separately.
    #[must_use]
    pub const fn best(&self) -> Option<MetricValue> {
        self.best
    }
    /// Completed iterations since the last strictly better result.
    #[must_use]
    pub const fn plateau(&self) -> u32 {
        self.plateau
    }
    /// Count independently unrefuted improvements.
    #[must_use]
    pub fn confirmed(&self) -> usize {
        self.records
            .values()
            .filter(|record| matches!(record.skeptic, Skeptic::Confirmed(_)))
            .count()
    }
    /// Render only acknowledged records with exact evidence links; no model-generated citations.
    #[must_use]
    pub fn report(&self) -> String {
        use std::fmt::Write;
        let mut output = format!(
            "program={} completed={} confirmed={} halted={} wall_ms={} tokens={} micro_usd={}\n",
            self.program,
            self.completed,
            self.confirmed(),
            self.halted,
            self.spent.wall_ms,
            self.spent.tokens,
            self.spent.micro_usd
        );
        for (id, record) in &self.records {
            let measurement = record
                .observation
                .map(|value| format!("metric={} evidence={}", value.metric.0, value.evidence));
            // Writing to a String is infallible; nevertheless preserve fmt's Result explicitly.
            let result = writeln!(
                &mut output,
                "experiment={} decision={:?} {} receipt={} skeptic={:?}",
                id.0,
                record.decision,
                match measurement {
                    Some(value) => value,
                    None => "interrupted".to_owned(),
                },
                record.receipt,
                record.skeptic
            );
            if result.is_err() {
                return output;
            }
        }
        output
    }
}

/// Typed rejection of invalid transitions or untrusted experiment observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResearchError {
    /// Unsupported, oversized, truncated or inconsistent checkpoint bytes.
    #[error("invalid research checkpoint")]
    Checkpoint,
    /// A metric contained NaN or infinity.
    #[error("metric must be finite")]
    NonFinite,
    /// Invalid or exhausted run budget.
    #[error("research budget is invalid or exhausted")]
    Budget,
    /// Arithmetic exceeded its representation.
    #[error("research accounting overflow")]
    Overflow,
    /// Internal state did not satisfy accounting/identity invariants.
    #[error("research accounting invariant violated")]
    Invariant,
    /// Prior experiment lacks a settled mutation or recovery receipt.
    #[error("experiment is unresolved or in the wrong state")]
    Unsettled,
    /// Experiment ID is stale or absent.
    #[error("stale experiment identity")]
    Stale,
    /// Actual usage exceeded its preauthorized reservation.
    #[error("experiment exceeded its reservation; recovery required")]
    Overrun,
    /// Workspace acknowledgement did not match the requested transition.
    #[error("workspace disposition receipt is invalid")]
    Receipt,
    /// Independent evidence is missing, reused, or already settled.
    #[error("independent evidence is invalid")]
    Evidence,
    /// Resume attempted against a modified immutable program.
    #[error("research program changed since checkpoint")]
    ProgramChanged,
}
