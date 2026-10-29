use polaris_runtime_research::{
    Budget, CheckpointStore, Decision, Direction, LedgerEntry, LedgerKind, MetricValue,
    Observation, ResearchState, Usage,
};
use polaris_sdk::ids::{Cid, SessionId};
use std::fs;
use std::path::PathBuf;
use std::thread;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("polaris-research-{}", SessionId::new_v7()))
}

fn state(program: Cid, inflight: bool) -> TestResult<ResearchState> {
    let mut state = ResearchState::new(program, Direction::Min, Budget::new(100, 100, 100, 20)?)?;
    if inflight {
        let id = state.begin(Usage::new(5, 5, 5))?;
        state.measure(
            id,
            Observation::new(
                MetricValue::new(1.0)?,
                Cid::from_bytes([2; 32]),
                Usage::new(2, 2, 2),
            ),
        )?;
        state.settle(id, Decision::Keep, Cid::from_bytes([3; 32]))?;
    }
    Ok(state)
}

#[test]
fn atomic_checkpoint_reopens_after_store_drop() -> TestResult<()> {
    let root = temp_root();
    let program = Cid::from_bytes([1; 32]);
    let saved = state(program, true)?;
    let generation = CheckpointStore::open(&root, program)?.save(&saved)?;
    drop(saved);
    let reopened = CheckpointStore::open(&root, program)?
        .load()?
        .ok_or("missing checkpoint")?;
    assert_eq!(generation, 1);
    assert_eq!(reopened.generation(), 1);
    assert_eq!(reopened.state().completed(), 1);
    assert_eq!(reopened.state().best().map(MetricValue::get), Some(1.0));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn in_flight_work_requires_recovery_after_reopen() -> TestResult<()> {
    let root = temp_root();
    let program = Cid::from_bytes([4; 32]);
    let mut state = state(program, false)?;
    let id = state.begin(Usage::new(5, 5, 5))?;
    CheckpointStore::open(&root, program)?.save(&state)?;
    let reopened = CheckpointStore::open(&root, program)?
        .load()?
        .ok_or("missing checkpoint")?;
    let mut state = reopened.into_state();
    assert_eq!(state.recovery_required(), Some(id));
    assert!(state.begin(Usage::new(1, 1, 1)).is_err());
    state.recovered(id, Cid::from_bytes([5; 32]))?;
    assert_eq!(state.completed(), 1);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn program_identity_is_checked_on_reopen() -> TestResult<()> {
    let root = temp_root();
    let program = Cid::from_bytes([6; 32]);
    CheckpointStore::open(&root, program)?.save(&state(program, false)?)?;
    assert!(CheckpointStore::open(&root, Cid::from_bytes([7; 32])).is_err());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn concurrent_handles_serialize_checkpoint_and_ledger_updates() -> TestResult<()> {
    let root = temp_root();
    let program = Cid::from_bytes([12; 32]);
    let store_a = CheckpointStore::open(&root, program)?;
    let store_b = CheckpointStore::open(&root, program)?;
    let saved = state(program, false)?;
    thread::scope(|scope| -> TestResult<()> {
        let left = scope.spawn(|| -> Result<(), String> {
            for index in 0_u8..20 {
                store_a.save(&saved).map_err(|error| error.to_string())?;
                store_a
                    .append_ledger(
                        u32::from(index) + 1,
                        LedgerKind::Tool,
                        store_a
                            .put_evidence(&[index])
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        });
        let right = scope.spawn(|| -> Result<(), String> {
            for index in 20_u8..40 {
                store_b.save(&saved).map_err(|error| error.to_string())?;
                store_b
                    .append_ledger(
                        u32::from(index) + 1,
                        LedgerKind::MetricRun,
                        store_b
                            .put_evidence(&[index])
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        });
        left.join()
            .map_err(|_| std::io::Error::other("left writer panicked"))?
            .map_err(std::io::Error::other)?;
        right
            .join()
            .map_err(|_| std::io::Error::other("right writer panicked"))?
            .map_err(std::io::Error::other)?;
        Ok(())
    })?;
    let reopened = CheckpointStore::open(&root, program)?;
    let checkpoint = reopened.load()?.ok_or("missing concurrent checkpoint")?;
    assert_eq!(checkpoint.generation(), 40);
    assert_eq!(reopened.ledger()?.len(), 40);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn evidence_ledger_is_content_addressed_and_reopens_in_order() -> TestResult<()> {
    let root = temp_root();
    let program = Cid::from_bytes([8; 32]);
    let store = CheckpointStore::open(&root, program)?;
    let tool = store.put_evidence(b"tool bytes")?;
    let metric = store.put_evidence(b"metric bytes")?;
    let tool_node = store.append_ledger(1, LedgerKind::Tool, tool)?;
    let metric_node = store.append_ledger(1, LedgerKind::MetricRun, metric)?;
    assert_ne!(tool_node, metric_node);
    drop(store);

    let reopened = CheckpointStore::open(&root, program)?;
    assert_eq!(
        reopened.ledger()?,
        vec![
            LedgerEntry {
                experiment: 1,
                kind: LedgerKind::MetricRun,
                evidence: metric,
            },
            LedgerEntry {
                experiment: 1,
                kind: LedgerKind::Tool,
                evidence: tool,
            },
        ]
    );
    assert_eq!(reopened.get_evidence(tool)?, b"tool bytes");
    assert_eq!(reopened.get_evidence(metric)?, b"metric bytes");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_evidence_ledger_pointer_fails_closed() -> TestResult<()> {
    let root = temp_root();
    let store = CheckpointStore::open(&root, Cid::from_bytes([11; 32]))?;
    fs::write(root.join("evidence-current"), b"invalid-ledger-pointer")?;
    assert!(store.ledger().is_err());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn ledger_rejects_missing_evidence_payload() -> TestResult<()> {
    let root = temp_root();
    let store = CheckpointStore::open(&root, Cid::from_bytes([13; 32]))?;
    assert!(
        store
            .append_ledger(1, LedgerKind::Tool, Cid::from_bytes([14; 32]))
            .is_err()
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
