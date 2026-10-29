// Unit tests of crates/polaris-ratelimit/src/ledger.rs (`tests`); compiled into that module via #[path].
use super::*;
use crate::{Clock, DualLimiter, Estimate, Priority, Quota, ShedPolicy};
use polaris_sdk::ids::TokenCount;
use std::num::NonZeroU32;

#[derive(Clone, Copy)]
struct ZeroClock;

impl Clock for ZeroClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

#[test]
fn max_plus_composition_preserves_later_floor() {
    let first = Record {
        kind: 2,
        id: 0,
        time_ns: 0,
        cost: 0,
        delta_ns: 6,
        floor_ns: 6,
    };
    let second = Record {
        kind: 2,
        id: 0,
        time_ns: 0,
        cost: 0,
        delta_ns: 4,
        floor_ns: 7,
    };
    let mut log = Log::empty();
    assert!(log.push(first).is_ok());
    assert!(log.push(second).is_ok());
    log.compact();
    assert_eq!(log.count, 1);
    assert_eq!(log.replay(1), 10);
}

#[test]
fn snapshot_pool_saturation_defers_without_losing_settlement_intent()
-> Result<(), Box<dyn std::error::Error>> {
    let ledger = QuotaLedger::new(60_000, 100, 60_000, 100)?;
    let reservation = ledger
        .reserve(0, 1)
        .map_err(|_| "initial reservation denied")?;
    ledger.dispatch(reservation)?;
    let Some(held_reader) = ledger.pin() else {
        return Err("could not pin current snapshot".into());
    };
    let mut saturated = false;
    for id in 100_u64..120_u64 {
        match ledger.mutate(Mutation::Reserve {
            id,
            now_ns: 0,
            cost: 1,
        }) {
            Ok(()) => {}
            Err(MutationError::Busy) => {
                saturated = true;
                break;
            }
            Err(MutationError::Denied(_)) => {
                return Err("capacity denied before snapshot pool saturated".into());
            }
        }
    }
    if !saturated {
        return Err("saturated snapshot pool did not return Busy".into());
    }
    let (refunded, deferred) = ledger.settle(reservation, 0, 0)?;
    if refunded != 0 || !deferred {
        return Err("saturated settlement was reported before publication".into());
    }
    drop(held_reader);
    ledger
        .help_ready()
        .map_err(|_| "ready settlement intent was not helpable")?;
    let Some(intent) = ledger.intents.get(reservation.slot) else {
        return Err("settlement intent slot disappeared".into());
    };
    let state = intent.state.load(Ordering::Acquire);
    if generation_part(state) != reservation.generation || state_part(state) != INTENT_AVAILABLE {
        return Err("settlement intent slot was not released after publication".into());
    }
    Ok(())
}

#[test]
fn snapshot_result_survives_next_publication_and_stale_ack_cannot_release_reuse()
-> Result<(), Box<dyn std::error::Error>> {
    let ledger = QuotaLedger::new(60_000, 100, 60_000, 100)?;
    let reservation = ledger
        .reserve(0, 1)
        .map_err(|_| "initial reservation denied")?;
    ledger.dispatch(reservation)?;
    ledger.publish_intent(reservation, 0, 0, 1, false)?;
    let Some(intent_data) = ledger.first_ready() else {
        return Err("settlement intent was not published".into());
    };
    if !matches!(ledger.mutate(Mutation::Intent(intent_data)), Ok(())) {
        return Err("settlement snapshot was not published".into());
    }
    let Some(result_read) = ledger.pin() else {
        return Err("could not pin published settlement result".into());
    };
    let Some(result_snapshot) = ledger.snapshots.get(result_read.slot) else {
        return Err("published snapshot slot was invalid".into());
    };
    if state_part(
        ledger
            .intents
            .get(reservation.slot)
            .ok_or("intent slot missing")?
            .state
            .load(Ordering::Acquire),
    ) != INTENT_READY_OWNER
    {
        return Err("intent was acknowledged before the forced interleaving".into());
    }

    if !matches!(
        ledger.mutate(Mutation::Reserve {
            id: 10_000,
            now_ns: 0,
            cost: 1,
        }),
        Ok(())
    ) {
        return Err("next publication did not acknowledge its exact source".into());
    }
    if state_part(
        ledger
            .intents
            .get(reservation.slot)
            .ok_or("intent slot missing")?
            .state
            .load(Ordering::Acquire),
    ) != INTENT_APPLIED_OWNER
    {
        return Err("next publication lost the pending result".into());
    }
    if ledger.read_outcome(reservation).is_none() {
        return Err("published result was unavailable to its owner".into());
    }

    let intent = ledger
        .intents
        .get(reservation.slot)
        .ok_or("intent slot missing")?;
    let applied =
        pack_intent(reservation.generation, INTENT_APPLIED_OWNER).ok_or("invalid applied state")?;
    let available =
        pack_intent(reservation.generation, INTENT_AVAILABLE).ok_or("invalid available state")?;
    if intent
        .state
        .compare_exchange(applied, available, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("owner did not release the completed intent".into());
    }
    let (reused_slot, reused_generation) = ledger
        .claim_intent()
        .map_err(|()| "intent slot was not reusable")?;
    if reused_slot != reservation.slot || reused_generation <= reservation.generation {
        return Err("intent generation was not advanced on reuse".into());
    }
    ledger
        .ack_snapshot_result(result_snapshot)
        .map_err(|()| "stale result acknowledgement was rejected")?;
    let state = intent.state.load(Ordering::Acquire);
    if generation_part(state) != reused_generation || state_part(state) != INTENT_RESERVED {
        return Err("stale result acknowledgement changed the reused intent".into());
    }
    drop(result_read);
    ledger.release_unpublished_intent(reused_slot, reused_generation);
    Ok(())
}

#[test]
fn background_admission_helps_deferred_refund_before_shedding()
-> Result<(), Box<dyn std::error::Error>> {
    let limiter = DualLimiter::new(
        Quota {
            rpm: NonZeroU32::new(60_000).ok_or("invalid RPM")?,
            tpm: NonZeroU32::new(60).ok_or("invalid TPM")?,
            burst_requests: 100,
            burst_tokens: 1_000,
        },
        ZeroClock,
        ShedPolicy {
            background_above_pct: 50,
            subagent_above_pct: 75,
            verifier_above_pct: 90,
        },
    )?;
    let estimate = |tokens| Estimate {
        prompt: TokenCount::new(tokens),
        max_output: TokenCount::new(0),
    };
    let mut first = match limiter.try_acquire(estimate(600), Priority::InteractiveUser) {
        Ok(permit) => permit,
        Err(denied) => return Err(format!("first request denied: {denied:?}").into()),
    };
    first.mark_dispatched()?;
    let Some(held_reader) = limiter.shared.ledger.pin() else {
        return Err("could not hold a snapshot reader".into());
    };
    // Keep one old snapshot pinned while seven publications consume the other slots. The next
    // settlement must therefore remain owned as an intent until the reader is released.
    for id in 100_u64..107_u64 {
        if !matches!(
            limiter.shared.ledger.mutate(Mutation::Reserve {
                id,
                now_ns: 0,
                cost: 10,
            }),
            Ok(())
        ) {
            return Err("could not saturate the snapshot pool".into());
        }
    }
    let reconciliation = first.reconcile(0)?;
    if !reconciliation.settlement_deferred || reconciliation.refunded_tokens != 0 {
        return Err("saturated settlement was reported as already refunded".into());
    }
    drop(held_reader);
    let follow_up = match limiter.try_acquire(estimate(1), Priority::Background) {
        Ok(permit) => permit,
        Err(denied) => {
            return Err(format!("background request stayed shed: {denied:?}").into());
        }
    };
    drop(follow_up);
    Ok(())
}
