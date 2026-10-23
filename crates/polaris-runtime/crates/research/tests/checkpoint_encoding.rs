use polaris_runtime_research::{
    Budget, Checkpoint, Decision, Direction, MetricValue, Observation, ResearchState, Usage,
};
use polaris_sdk::ids::Cid;
type ResultTest = Result<(), Box<dyn std::error::Error>>;
fn cid(value: u8) -> Cid {
    Cid::from_bytes([value; 32])
}
fn sample() -> Result<ResearchState, Box<dyn std::error::Error>> {
    let mut state = ResearchState::new(cid(1), Direction::Min, Budget::new(100, 100, 100, 30)?)?;
    let id = state.begin(Usage::new(5, 5, 5))?;
    state.measure(
        id,
        Observation::new(MetricValue::new(3.0)?, cid(2), Usage::new(2, 2, 2)),
    )?;
    state.settle(id, Decision::Keep, cid(3))?;
    state.confirm(id, cid(4), true)?;
    state.begin(Usage::new(5, 5, 5))?;
    Ok(state)
}
#[test]
fn checkpoint_roundtrip_preserves_recovery_and_accounting() -> ResultTest {
    let state = sample()?;
    let bytes = state.checkpoint().encode()?;
    let decoded = Checkpoint::decode(&bytes)?;
    assert_eq!(decoded, state.checkpoint());
    let mut resumed = ResearchState::resume(cid(1), decoded)?;
    assert_eq!(resumed.spent(), Usage::new(7, 7, 7));
    let id = resumed.recovery_required().ok_or("missing recovery")?;
    resumed.recovered(id, cid(5))?;
    let restored =
        ResearchState::resume(cid(1), Checkpoint::decode(&resumed.checkpoint().encode()?)?)?;
    assert_eq!(restored, resumed);
    assert_eq!(restored.confirmed(), 1);
    assert_eq!(restored.completed(), 2);
    Ok(())
}
#[test]
fn checkpoint_rejects_truncation_and_trailing_bytes() -> ResultTest {
    let bytes = sample()?.checkpoint().encode()?;
    for length in 0..bytes.len() {
        assert!(Checkpoint::decode(bytes.get(..length).ok_or("prefix")?).is_err());
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(Checkpoint::decode(&extra).is_err());
    Ok(())
}
#[test]
fn checkpoint_rejects_corrupt_accounting() -> ResultTest {
    let bytes = sample()?.checkpoint().encode()?;
    // The fixed header is followed by the program CID and direction. Changing either version
    // or a budget/accounting byte must never create an inconsistent accepted checkpoint.
    for offset in [0_usize, 4, 38, 66, 90] {
        let mut mutated = bytes.clone();
        let byte = mutated.get_mut(offset).ok_or("fixture too short")?;
        *byte = 255;
        assert!(Checkpoint::decode(&mutated).is_err(), "offset={offset}");
    }
    Ok(())
}
