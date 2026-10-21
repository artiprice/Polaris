//! Circuit Breaker (descriptor: specs/descriptors/supervisor_circuit_breaker.json).

use std::error::Error;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_kernel_bus::{Bus, BusConfig};
use polaris_kernel_supervisor::{
    BreakerConfig, BreakerState, ChildName, CircuitBreaker, Supervisor, SupervisorConfig,
};
use polaris_sdk::contracts::{Event, EventRecord};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use polaris_sdk::registry::{EventStore, EventStoreError};
use proptest::prelude::*;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

/// In-memory store with gap-free sequence numbers.
#[derive(Default)]
struct MemStore {
    records: Mutex<Vec<EventRecord>>,
}

impl MemStore {
    fn events(&self) -> Vec<Event> {
        self.records
            .lock()
            .map(|r| r.iter().map(|rec| *rec.event()).collect())
            .unwrap_or_default()
    }
}

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        let seq = Seq::new(
            u64::try_from(records.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        );
        records.push(record.with_seq(seq));
        Ok(seq)
    }
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        let records = self
            .records
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        Ok(records
            .iter()
            .filter(|r| r.seq() >= from && r.seq() <= to)
            .copied()
            .collect())
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap_or(NonZeroU32::MIN)
}

fn breaker(max: u32, window_ms: u64, cooldown_ms: u64) -> CircuitBreaker {
    CircuitBreaker::new(nz(max), ms(window_ms), ms(cooldown_ms))
}

#[test]
fn opens_after_n_failures_in_window() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(3, 1_000, 5_000);
    assert!(b.allow(t0));
    assert_eq!(b.on_failure(t0), BreakerState::Closed);
    assert_eq!(b.on_failure(t0 + ms(100)), BreakerState::Closed);
    assert!(b.allow(t0 + ms(150)));
    assert_eq!(
        b.on_failure(t0 + ms(200)),
        BreakerState::Open {
            until: t0 + ms(200) + ms(5_000)
        }
    );
    assert_eq!(
        b.state(),
        BreakerState::Open {
            until: t0 + ms(5_200)
        }
    );
    Ok(())
}

#[test]
fn old_failures_age_out_of_window() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(2, 1_000, 5_000);
    // One failure per 1.5 s never accumulates two inside a 1 s window.
    for i in 0..20_u64 {
        assert_eq!(b.on_failure(t0 + ms(i * 1_500)), BreakerState::Closed);
        assert!(b.allow(t0 + ms(i * 1_500 + 1)));
    }
    // A failure exactly one window after the previous one has aged it out as well.
    let mut edge = breaker(2, 1_000, 5_000);
    assert_eq!(edge.on_failure(t0), BreakerState::Closed);
    assert_eq!(edge.on_failure(t0 + ms(1_000)), BreakerState::Closed);
    // 999 ms after the previous one is inside the window: opens.
    assert!(matches!(
        edge.on_failure(t0 + ms(1_999)),
        BreakerState::Open { .. }
    ));
    Ok(())
}

#[test]
fn open_rejects_until_cooldown() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(1, 1_000, 5_000);
    b.on_failure(t0);
    assert!(!b.allow(t0));
    assert!(!b.allow(t0 + ms(4_999)));
    assert_eq!(
        b.state(),
        BreakerState::Open {
            until: t0 + ms(5_000)
        }
    );
    Ok(())
}

#[test]
fn half_open_after_cooldown() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(1, 1_000, 5_000);
    b.on_failure(t0);
    // The first caller at the cooldown instant is the probe; the next one is refused.
    assert!(b.allow(t0 + ms(5_000)));
    assert_eq!(b.state(), BreakerState::HalfOpen);
    assert!(!b.allow(t0 + ms(5_001)));
    assert!(!b.allow(t0 + ms(9_999)));
    // A probe whose result never arrives does not wedge the breaker: after another
    // cooldown a new probe is admitted.
    assert!(b.allow(t0 + ms(10_000)));
    assert_eq!(b.state(), BreakerState::HalfOpen);
    Ok(())
}

#[test]
fn half_open_success_closes() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(2, 1_000, 5_000);
    b.on_failure(t0);
    b.on_failure(t0 + ms(1));
    assert!(b.allow(t0 + ms(6_000)));
    b.on_success(t0 + ms(6_010));
    assert_eq!(b.state(), BreakerState::Closed);
    assert!(b.allow(t0 + ms(6_011)));
    // History was cleared: one failure does not reopen a 2-failure breaker.
    assert_eq!(b.on_failure(t0 + ms(6_020)), BreakerState::Closed);
    Ok(())
}

#[test]
fn half_open_failure_reopens() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(1, 1_000, 5_000);
    b.on_failure(t0);
    assert!(b.allow(t0 + ms(5_000)));
    assert_eq!(
        b.on_failure(t0 + ms(5_100)),
        BreakerState::Open {
            until: t0 + ms(10_100)
        }
    );
    assert!(!b.allow(t0 + ms(10_099)));
    assert!(b.allow(t0 + ms(10_100)));
    Ok(())
}

#[test]
fn late_results_while_open_are_ignored() -> TestResult {
    let t0 = Instant::now();
    let mut b = breaker(1, 1_000, 5_000);
    let opened = b.on_failure(t0);
    // Results of calls admitted before the trip arrive while Open.
    assert_eq!(b.on_failure(t0 + ms(3_000)), opened);
    b.on_success(t0 + ms(3_500));
    assert_eq!(b.state(), opened);
    assert!(b.allow(t0 + ms(5_000)));
    Ok(())
}

fn handle_fixture() -> Result<(Supervisor<Arc<MemStore>>, Arc<MemStore>), Box<dyn Error>> {
    let store = Arc::new(MemStore::default());
    let (bus, _commands) = Bus::new(BusConfig::default(), Arc::clone(&store))?;
    let sup = Supervisor::new(
        Arc::new(bus),
        SessionId::from_u128(3),
        CancellationToken::new(),
        SupervisorConfig::default(),
    );
    Ok((sup, store))
}

fn config(max: u32, window_ms: u64, cooldown_ms: u64) -> BreakerConfig {
    BreakerConfig {
        max_failures: nz(max),
        window: ms(window_ms),
        cooldown: ms(cooldown_ms),
    }
}

#[tokio::test(start_paused = true)]
async fn handle_publishes_circuit_tripped_once() -> TestResult {
    let (sup, store) = handle_fixture()?;
    let name = ChildName::new("s1-client", Cid::from_bytes([5; 32]));
    let handle = sup.breaker(name, config(3, 10_000, 30_000));
    for _ in 0..2 {
        assert_eq!(handle.on_failure().await, BreakerState::Closed);
    }
    assert!(store.events().is_empty());
    assert!(matches!(
        handle.on_failure().await,
        BreakerState::Open { .. }
    ));
    // Late failures while Open do not announce the trip again.
    handle.on_failure().await;
    handle.on_failure().await;
    assert_eq!(
        store.events(),
        vec![Event::CircuitTripped {
            breaker: name.cid()
        }]
    );
    let snap = sup.snapshot();
    let entry = snap.breakers.first().ok_or("breaker missing in snapshot")?;
    assert_eq!(entry.name, name);
    assert!(matches!(entry.state, BreakerState::Open { .. }));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn handle_half_open_probe_under_paused_time() -> TestResult {
    let (sup, store) = handle_fixture()?;
    let handle = sup.breaker(
        ChildName::new("s1-client", Cid::from_bytes([5; 32])),
        config(2, 10_000, 30_000),
    );
    handle.on_failure().await;
    handle.on_failure().await;
    assert!(!handle.allow());
    tokio::time::advance(ms(29_999)).await;
    assert!(!handle.allow());
    tokio::time::advance(ms(1)).await;
    assert!(
        handle.allow(),
        "first caller after the cooldown is the probe"
    );
    assert!(!handle.allow(), "only one probe at a time");
    assert_eq!(handle.state(), BreakerState::HalfOpen);
    handle.on_success();
    assert_eq!(handle.state(), BreakerState::Closed);
    assert!(handle.allow());
    assert_eq!(store.events().len(), 1);
    Ok(())
}

/// One operation of the property tests; every operation first advances the clock.
#[derive(Debug, Clone, Copy)]
enum Op {
    Allow(u64),
    Success(u64),
    Failure(u64),
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let dt = 0_u64..3_000;
    let op = prop_oneof![
        dt.clone().prop_map(Op::Allow),
        dt.clone().prop_map(Op::Success),
        dt.prop_map(Op::Failure),
    ];
    proptest::collection::vec(op, 0..80)
}

/// Reference model written independently of the implementation: it keeps every failure in a
/// list and recomputes the window count from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Model {
    Closed,
    Open(u64),
    Half(u64),
}

struct Reference {
    max: usize,
    window: u64,
    cooldown: u64,
    failures: Vec<u64>,
    state: Model,
}

impl Reference {
    fn allow(&mut self, now: u64) -> bool {
        match self.state {
            Model::Closed => true,
            Model::Open(until) if now >= until => {
                self.state = Model::Half(now);
                true
            }
            Model::Open(_) => false,
            Model::Half(since) if now >= since + self.cooldown => {
                self.state = Model::Half(now);
                true
            }
            Model::Half(_) => false,
        }
    }

    fn success(&mut self) {
        if let Model::Half(_) = self.state {
            self.state = Model::Closed;
            self.failures.clear();
        }
    }

    fn failure(&mut self, now: u64) {
        match self.state {
            Model::Closed => {
                self.failures.push(now);
                let recent = self
                    .failures
                    .iter()
                    .filter(|t| now - **t < self.window)
                    .count();
                if recent >= self.max {
                    self.state = Model::Open(now + self.cooldown);
                    self.failures.clear();
                }
            }
            Model::Half(_) => self.state = Model::Open(now + self.cooldown),
            Model::Open(_) => {}
        }
    }

    fn public(&self, base: Instant) -> BreakerState {
        match self.state {
            Model::Closed => BreakerState::Closed,
            Model::Open(until) => BreakerState::Open {
                until: base + ms(until),
            },
            Model::Half(_) => BreakerState::HalfOpen,
        }
    }
}

proptest! {
    #[test]
    fn prop_never_closed_to_half_open(
        max in 1_u32..5,
        window_ms in 1_u64..2_000,
        cooldown_ms in 1_u64..4_000,
        ops in ops(),
    ) {
        let base = Instant::now();
        let mut b = breaker(max, window_ms, cooldown_ms);
        let mut now = 0_u64;
        let mut previous = b.state();
        for op in ops {
            match op {
                Op::Allow(dt) => { now += dt; b.allow(base + ms(now)); }
                Op::Success(dt) => { now += dt; b.on_success(base + ms(now)); }
                Op::Failure(dt) => { now += dt; b.on_failure(base + ms(now)); }
            }
            let current = b.state();
            prop_assert!(
                !(previous == BreakerState::Closed && current == BreakerState::HalfOpen),
                "Closed -> HalfOpen after {op:?}"
            );
            previous = current;
        }
    }

    #[test]
    fn prop_matches_reference_model(
        max in 1_u32..5,
        window_ms in 1_u64..2_000,
        cooldown_ms in 1_u64..4_000,
        ops in ops(),
    ) {
        let base = Instant::now();
        let mut b = breaker(max, window_ms, cooldown_ms);
        let mut model = Reference {
            max: max as usize,
            window: window_ms,
            cooldown: cooldown_ms,
            failures: Vec::new(),
            state: Model::Closed,
        };
        let mut now = 0_u64;
        for op in ops {
            match op {
                Op::Allow(dt) => {
                    now += dt;
                    prop_assert_eq!(b.allow(base + ms(now)), model.allow(now), "allow at {}", now);
                }
                Op::Success(dt) => {
                    now += dt;
                    b.on_success(base + ms(now));
                    model.success();
                }
                Op::Failure(dt) => {
                    now += dt;
                    let returned = b.on_failure(base + ms(now));
                    model.failure(now);
                    prop_assert_eq!(returned, model.public(base));
                }
            }
            prop_assert_eq!(b.state(), model.public(base), "state after {:?} at {}", op, now);
        }
    }
}
