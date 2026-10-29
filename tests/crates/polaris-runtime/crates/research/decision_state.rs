use polaris_runtime_research::{
    Budget, Decision, Direction, MetricValue, Observation, ResearchState, Usage,
};
use polaris_sdk::ids::Cid;

fn cid(value: u8) -> Cid {
    Cid::from_bytes([value; 32])
}
fn state(direction: Direction) -> Result<ResearchState, Box<dyn std::error::Error>> {
    Ok(ResearchState::new(
        cid(1),
        direction,
        Budget::new(100, 1_000, 10_000, 30)?,
    )?)
}
fn observe(value: f64, evidence: u8) -> Result<Observation, Box<dyn std::error::Error>> {
    Ok(Observation::new(
        MetricValue::new(value)?,
        cid(evidence),
        Usage::new(1, 10, 100),
    ))
}

#[test]
fn strict_improvements_only() -> Result<(), Box<dyn std::error::Error>> {
    for (direction, values) in [
        (Direction::Min, [5.0, 4.0, 4.0, 6.0]),
        (Direction::Max, [5.0, 6.0, 6.0, 4.0]),
    ] {
        let mut state = state(direction)?;
        for (value, expected) in values.into_iter().zip([
            Decision::Keep,
            Decision::Keep,
            Decision::Revert,
            Decision::Revert,
        ]) {
            let id = state.begin(Usage::new(2, 20, 200))?;
            let decision = state.measure(id, observe(value, 2)?)?;
            assert_eq!(decision, expected);
            state.settle(id, decision, cid(3))?;
        }
        assert_eq!(state.completed(), 4);
    }
    assert!(MetricValue::new(f64::NAN).is_err());
    assert!(MetricValue::new(f64::INFINITY).is_err());
    Ok(())
}

#[test]
fn budget_reservation_is_conservative() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state(Direction::Min)?;
    assert!(state.begin(Usage::new(101, 1, 1)).is_err());
    let id = state.begin(Usage::new(5, 20, 200))?;
    assert!(state.begin(Usage::new(1, 1, 1)).is_err());
    assert!(
        state
            .measure(
                id,
                Observation::new(MetricValue::new(1.0)?, cid(2), Usage::new(6, 20, 200))
            )
            .is_err()
    );
    assert_eq!(state.spent(), Usage::new(6, 20, 200));
    assert!(state.halted());
    assert!(state.measure(id, observe(1.0, 2)?).is_err());
    state.recovered(id, cid(4))?;
    assert_eq!(state.completed(), 1);
    assert!(state.begin(Usage::new(1, 1, 1)).is_err());
    Ok(())
}

#[test]
fn confirmation_requires_independent_evidence() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state(Direction::Min)?;
    let id = state.begin(Usage::new(2, 20, 200))?;
    state.measure(id, observe(1.0, 2)?)?;
    assert!(state.confirm(id, cid(3), true).is_err());
    state.settle(id, Decision::Keep, cid(3))?;
    assert!(state.confirm(id, cid(2), true).is_err());
    assert!(state.confirm(id, cid(3), true).is_err());
    state.confirm(id, cid(4), false)?;
    assert_eq!(state.confirmed(), 0);
    assert!(state.confirm(id, cid(5), true).is_err());
    Ok(())
}

#[test]
fn interrupted_iteration_requires_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state(Direction::Min)?;
    let id = state.begin(Usage::new(2, 20, 200))?;
    let checkpoint = state.checkpoint();
    assert!(ResearchState::resume(cid(9), checkpoint.clone()).is_err());
    let mut resumed = ResearchState::resume(cid(1), checkpoint)?;
    assert!(resumed.begin(Usage::new(1, 1, 1)).is_err());
    assert_eq!(resumed.recovery_required(), Some(id));
    resumed.recovered(id, cid(4))?;
    assert!(resumed.begin(Usage::new(1, 1, 1)).is_ok());
    assert!(resumed.recovered(id, cid(4)).is_err());
    Ok(())
}
