//! Bounded, versioned binary checkpoint codec; storage authenticity belongs to the CAS owner.
use super::{
    Budget, Checkpoint, Decision, Direction, ExperimentId, InFlight, MetricValue, Observation,
    Record, ResearchError, ResearchState, Skeptic, Usage,
};
use polaris_sdk::ids::Cid;
use std::collections::BTreeMap;

const HEADER: &[u8] = b"PRSC\x01";
const MAX_BYTES: usize = 32 * 1024 * 1024;

impl Checkpoint {
    /// Encode a validated state with a fixed version and explicit bounded counts.
    ///
    /// # Errors
    /// Rejects inconsistent accounting, impossible transitions, or the 32 MiB encoding bound.
    pub fn encode(&self) -> Result<Vec<u8>, ResearchError> {
        validate(&self.state)?;
        let state = &self.state;
        let mut out = Vec::new();
        out.extend_from_slice(HEADER);
        out.extend_from_slice(state.program.as_bytes());
        out.push(match state.direction {
            Direction::Min => 0,
            Direction::Max => 1,
        });
        out.push(0); // Reserved; unknown extensions must fail closed.
        put_usage(&mut out, state.budget.resources);
        put_u32(&mut out, state.budget.experiments);
        put_usage(&mut out, state.spent);
        out.push(u8::from(state.halted));
        put_u32(&mut out, state.next);
        put_u32(&mut out, state.completed);
        match state.best {
            Some(best) => {
                out.push(1);
                put_metric(&mut out, best);
            }
            None => out.push(0),
        }
        put_u32(&mut out, state.plateau);
        put_u32(
            &mut out,
            u32::try_from(state.records.len()).map_err(|_| ResearchError::Checkpoint)?,
        );
        for (id, record) in &state.records {
            put_u32(&mut out, id.0);
            put_usage(&mut out, record.charge);
            match record.observation {
                Some(observation) => {
                    out.push(1);
                    put_observation(&mut out, observation);
                }
                None => out.push(0),
            }
            put_decision(&mut out, record.decision);
            out.extend_from_slice(record.receipt.as_bytes());
            match record.skeptic {
                Skeptic::Pending => out.push(0),
                Skeptic::Confirmed(evidence) => {
                    out.push(1);
                    out.extend_from_slice(evidence.as_bytes());
                }
                Skeptic::Refuted(evidence) => {
                    out.push(2);
                    out.extend_from_slice(evidence.as_bytes());
                }
            }
        }
        match state.in_flight {
            None => out.push(0),
            Some(InFlight::Running { id, reservation }) => {
                out.push(1);
                put_u32(&mut out, id.0);
                put_usage(&mut out, reservation);
            }
            Some(InFlight::Measured {
                id,
                reservation,
                observation,
                decision,
            }) => {
                out.push(2);
                put_u32(&mut out, id.0);
                put_usage(&mut out, reservation);
                put_observation(&mut out, observation);
                put_decision(&mut out, decision);
            }
            Some(InFlight::Recovery { id, charge }) => {
                out.push(3);
                put_u32(&mut out, id.0);
                put_usage(&mut out, charge);
            }
        }
        if out.len() > MAX_BYTES {
            return Err(ResearchError::Checkpoint);
        }
        Ok(out)
    }

    /// Decode and validate all resource totals, experiment ordering and metric dispositions.
    ///
    /// A valid encoding does not prove who wrote it. The storage owner must verify its expected
    /// content ID before accepting it; never resume arbitrary unauthenticated bytes as authority.
    ///
    /// # Errors
    /// Rejects oversized input, unknown versions/tags, non-finite metrics, truncation, trailing
    /// bytes, excessive records, duplicate IDs, or inconsistent baseline/accounting state.
    pub fn decode(bytes: &[u8]) -> Result<Self, ResearchError> {
        if bytes.len() > MAX_BYTES {
            return Err(ResearchError::Checkpoint);
        }
        let rest = bytes
            .strip_prefix(HEADER)
            .ok_or(ResearchError::Checkpoint)?;
        let mut reader = Reader(rest);
        let program = reader.cid()?;
        let direction = match reader.byte()? {
            0 => Direction::Min,
            1 => Direction::Max,
            _ => return Err(ResearchError::Checkpoint),
        };
        if reader.byte()? != 0 {
            return Err(ResearchError::Checkpoint);
        }
        let resources = reader.usage()?;
        let budget = Budget::new(
            resources.wall_ms,
            resources.tokens,
            resources.micro_usd,
            reader.u32()?,
        )?;
        let spent = reader.usage()?;
        let halted = reader.flag()?;
        let next = reader.u32()?;
        let completed = reader.u32()?;
        let best = if reader.flag()? {
            Some(reader.metric()?)
        } else {
            None
        };
        let plateau = reader.u32()?;
        let count = reader.u32()?;
        if count > budget.experiments || count != completed {
            return Err(ResearchError::Checkpoint);
        }
        let mut records = BTreeMap::new();
        for expected in 1..=count {
            let id = ExperimentId(reader.u32()?);
            if id.0 != expected {
                return Err(ResearchError::Checkpoint);
            }
            let charge = reader.usage()?;
            let observation = if reader.flag()? {
                Some(reader.observation()?)
            } else {
                None
            };
            let decision = reader.decision()?;
            let receipt = reader.cid()?;
            let skeptic = match reader.byte()? {
                0 => Skeptic::Pending,
                1 => Skeptic::Confirmed(reader.cid()?),
                2 => Skeptic::Refuted(reader.cid()?),
                _ => return Err(ResearchError::Checkpoint),
            };
            if records
                .insert(
                    id,
                    Record {
                        charge,
                        observation,
                        decision,
                        receipt,
                        skeptic,
                    },
                )
                .is_some()
            {
                return Err(ResearchError::Checkpoint);
            }
        }
        let in_flight = match reader.byte()? {
            0 => None,
            1 => Some(InFlight::Running {
                id: ExperimentId(reader.u32()?),
                reservation: reader.usage()?,
            }),
            2 => Some(InFlight::Measured {
                id: ExperimentId(reader.u32()?),
                reservation: reader.usage()?,
                observation: reader.observation()?,
                decision: reader.decision()?,
            }),
            3 => Some(InFlight::Recovery {
                id: ExperimentId(reader.u32()?),
                charge: reader.usage()?,
            }),
            _ => return Err(ResearchError::Checkpoint),
        };
        if !reader.0.is_empty() {
            return Err(ResearchError::Checkpoint);
        }
        let state = ResearchState {
            program,
            direction,
            budget,
            spent,
            halted,
            next,
            completed,
            best,
            plateau,
            in_flight,
            records,
        };
        validate(&state)?;
        Ok(Self { state })
    }
}

fn decision(direction: Direction, best: Option<MetricValue>, candidate: MetricValue) -> Decision {
    let improves = match best {
        None => true,
        Some(previous) => match direction {
            Direction::Min => candidate.0 < previous.0,
            Direction::Max => candidate.0 > previous.0,
        },
    };
    if improves {
        Decision::Keep
    } else {
        Decision::Revert
    }
}

fn validate(state: &ResearchState) -> Result<(), ResearchError> {
    let count = u32::try_from(state.records.len()).map_err(|_| ResearchError::Checkpoint)?;
    if count != state.completed
        || count > state.budget.experiments
        || state.next > state.budget.experiments
    {
        return Err(ResearchError::Checkpoint);
    }
    let expected_next = count
        .checked_add(u32::from(state.in_flight.is_some()))
        .ok_or(ResearchError::Checkpoint)?;
    if state.next != expected_next {
        return Err(ResearchError::Checkpoint);
    }
    let mut total = Usage::default();
    let mut best = None;
    let mut plateau = 0_u32;
    for ((id, record), expected) in state.records.iter().zip(1..=count) {
        if id.0 != expected {
            return Err(ResearchError::Checkpoint);
        }
        total = total.add(record.charge)?;
        if let Some(observation) = record.observation {
            MetricValue::new(observation.metric.0)?;
            if record.charge != observation.usage
                || record.receipt == observation.evidence
                || record.decision != decision(state.direction, best, observation.metric)
            {
                return Err(ResearchError::Checkpoint);
            }
            match record.skeptic {
                Skeptic::Pending => {}
                Skeptic::Confirmed(evidence) | Skeptic::Refuted(evidence) => {
                    if record.decision != Decision::Keep
                        || evidence == observation.evidence
                        || evidence == record.receipt
                    {
                        return Err(ResearchError::Checkpoint);
                    }
                }
            }
            if record.decision == Decision::Keep {
                best = Some(observation.metric);
            }
        } else if record.charge.wall_ms == 0
            || record.decision != Decision::Revert
            || record.skeptic != Skeptic::Pending
        {
            return Err(ResearchError::Checkpoint);
        }
        plateau = if record.decision == Decision::Keep {
            0
        } else {
            plateau.checked_add(1).ok_or(ResearchError::Checkpoint)?
        };
    }
    if best != state.best || plateau != state.plateau {
        return Err(ResearchError::Checkpoint);
    }
    if let Some(active) = state.in_flight {
        if active.id().0 != state.next || active.charge().wall_ms == 0 {
            return Err(ResearchError::Checkpoint);
        }
        total = total.add(active.charge())?;
        match active {
            InFlight::Running { .. } if state.halted => return Err(ResearchError::Checkpoint),
            InFlight::Measured {
                observation,
                reservation,
                decision: applied,
                ..
            } => {
                if state.halted
                    || !observation.usage.fits(reservation)
                    || applied != decision(state.direction, best, observation.metric)
                {
                    return Err(ResearchError::Checkpoint);
                }
                MetricValue::new(observation.metric.0)?;
            }
            InFlight::Running { .. } | InFlight::Recovery { .. } => {}
        }
    }
    if total != state.spent || (!state.halted && !total.fits(state.budget.resources)) {
        return Err(ResearchError::Checkpoint);
    }
    Ok(())
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_usage(out: &mut Vec<u8>, usage: Usage) {
    for value in [usage.wall_ms, usage.tokens, usage.micro_usd] {
        out.extend_from_slice(&value.to_le_bytes());
    }
}
fn put_metric(out: &mut Vec<u8>, metric: MetricValue) {
    out.extend_from_slice(&metric.0.to_bits().to_le_bytes());
}
fn put_observation(out: &mut Vec<u8>, observation: Observation) {
    put_metric(out, observation.metric);
    out.extend_from_slice(observation.evidence.as_bytes());
    put_usage(out, observation.usage);
}
fn put_decision(out: &mut Vec<u8>, decision: Decision) {
    out.push(match decision {
        Decision::Keep => 0,
        Decision::Revert => 1,
    });
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ResearchError> {
        let (value, remaining) = self
            .0
            .split_at_checked(N)
            .ok_or(ResearchError::Checkpoint)?;
        let value = value.try_into().map_err(|_| ResearchError::Checkpoint)?;
        self.0 = remaining;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, ResearchError> {
        let (value, remaining) = self.0.split_first().ok_or(ResearchError::Checkpoint)?;
        self.0 = remaining;
        Ok(*value)
    }
    fn flag(&mut self) -> Result<bool, ResearchError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ResearchError::Checkpoint),
        }
    }
    fn u32(&mut self) -> Result<u32, ResearchError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, ResearchError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn cid(&mut self) -> Result<Cid, ResearchError> {
        Ok(Cid::from_bytes(self.array()?))
    }
    fn usage(&mut self) -> Result<Usage, ResearchError> {
        Ok(Usage::new(self.u64()?, self.u64()?, self.u64()?))
    }
    fn metric(&mut self) -> Result<MetricValue, ResearchError> {
        MetricValue::new(f64::from_bits(self.u64()?))
    }
    fn observation(&mut self) -> Result<Observation, ResearchError> {
        Ok(Observation::new(self.metric()?, self.cid()?, self.usage()?))
    }
    fn decision(&mut self) -> Result<Decision, ResearchError> {
        match self.byte()? {
            0 => Ok(Decision::Keep),
            1 => Ok(Decision::Revert),
            _ => Err(ResearchError::Checkpoint),
        }
    }
}
