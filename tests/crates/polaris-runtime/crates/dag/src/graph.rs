// Unit tests of crates/polaris-runtime/crates/dag/src/graph.rs (`tests`); compiled into that module via #[path].
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::task::JoinSet;

struct SuccessTask;

impl NodeTask for SuccessTask {
    type Output = ();

    fn run<'a>(
        &'a self,
        _context: NodeCtx,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, NodeError>> + Send + 'a>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

#[tokio::test]
async fn aborted_join_retains_node_identity() -> Result<(), DagError> {
    let mut join_set = JoinSet::new();
    let abort_handle = join_set.spawn(std::future::pending::<(
        usize,
        NodeId,
        Pool,
        Result<NodeResult<usize>, DagError>,
    )>());
    let task_id = abort_handle.id();
    abort_handle.abort();
    let mut in_flight = BTreeMap::new();
    in_flight.insert(
        task_id,
        TrackedTask {
            index: 4,
            id: NodeId::new(41),
            pool: Pool::Web,
            abort_handle,
        },
    );
    let Some(joined) = join_set.join_next_with_id().await else {
        return Err(DagError::JoinFailed);
    };
    let event = normalize_join(joined, &mut in_flight)?;
    match event {
        JoinedNode::Interrupted { index, id, reason } => {
            assert_eq!(index, 4, "the interrupted task must retain its graph index");
            assert_eq!(
                id,
                NodeId::new(41),
                "the interrupted task must retain its node id"
            );
            assert_eq!(
                reason,
                NodeInterruption::Aborted,
                "Tokio cancellation is Aborted"
            );
        }
        JoinedNode::Completed { .. } => return Err(DagError::JoinFailed),
    }
    assert!(
        in_flight.is_empty(),
        "the exact completion must remove its tracking entry"
    );
    Ok(())
}

#[tokio::test]
async fn abort_drain_retains_a_task_that_finished_before_cleanup() -> Result<(), DagError> {
    let node_id = NodeId::new(19);
    let mut join_set = JoinSet::new();
    let abort_handle = join_set.spawn(async move {
        (
            0,
            node_id,
            Pool::Web,
            Ok(NodeResult::Succeeded {
                value: 19_usize,
                attempts: 1,
            }),
        )
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !abort_handle.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| DagError::JoinFailed)?;
    let mut state = SchedulerState {
        indegree: vec![0],
        ready: BTreeSet::new(),
        started: BTreeSet::new(),
        results: vec![None],
    };
    let mut in_flight = BTreeMap::new();
    in_flight.insert(
        abort_handle.id(),
        TrackedTask {
            index: 0,
            id: NodeId::new(19),
            pool: Pool::Web,
            abort_handle,
        },
    );
    abort_and_drain(&mut join_set, &mut in_flight, &mut state).await?;
    assert!(
        matches!(
            state.results.first(),
            Some(Some(NodeResult::Succeeded {
                value: 19,
                attempts: 1
            }))
        ),
        "cleanup must retain a result completed before abort"
    );
    Ok(())
}

#[tokio::test]
async fn interrupted_completion_records_failure_and_preserves_sibling_readiness()
-> Result<(), DagError> {
    let root_id = NodeId::new(1);
    let child_id = NodeId::new(2);
    let sibling_id = NodeId::new(3);
    let mut builder = DagBuilder::new();
    builder.node(root_id, Pool::Web, SuccessTask)?;
    builder.node(child_id, Pool::Web, SuccessTask)?;
    builder.node(sibling_id, Pool::Disk, SuccessTask)?;
    builder.edge(root_id, child_id)?;
    let dag = builder.build()?;
    let indegree = dag
        .nodes
        .iter()
        .map(|node| node.dependencies.len())
        .collect::<Vec<_>>();
    let ready = dag
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| node.dependencies.is_empty().then_some((node.id, index)))
        .collect::<BTreeSet<_>>();
    let mut state = SchedulerState {
        indegree,
        ready,
        started: BTreeSet::new(),
        results: std::iter::repeat_with(|| None)
            .take(dag.nodes.len())
            .collect(),
    };
    let root_index = dag
        .nodes
        .iter()
        .position(|node| node.id == root_id)
        .ok_or(DagError::GraphInvariant)?;
    let mut join_set = JoinSet::new();
    let abort_handle = join_set.spawn(std::future::pending::<(
        usize,
        NodeId,
        Pool,
        Result<NodeResult<()>, DagError>,
    )>());
    let task_id = abort_handle.id();
    abort_handle.abort();
    let mut in_flight = BTreeMap::new();
    in_flight.insert(
        task_id,
        TrackedTask {
            index: root_index,
            id: root_id,
            pool: Pool::Web,
            abort_handle,
        },
    );
    let Some(joined) = join_set.join_next_with_id().await else {
        return Err(DagError::JoinFailed);
    };
    let event = normalize_join(joined, &mut in_flight)?;
    let outcome = record_joined_node(&dag, event, &mut state)?;
    assert_eq!(
        outcome,
        (root_id, true, false),
        "interruption records failure without global cancellation"
    );
    let child_index = dag
        .nodes
        .iter()
        .position(|node| node.id == child_id)
        .ok_or(DagError::GraphInvariant)?;
    let sibling_index = dag
        .nodes
        .iter()
        .position(|node| node.id == sibling_id)
        .ok_or(DagError::GraphInvariant)?;
    assert!(
        matches!(
            state.results.get(child_index),
            Some(Some(NodeResult::DependencyFailed { caused_by })) if *caused_by == root_id
        ),
        "the interrupted prerequisite must skip its descendant"
    );
    assert!(
        state.results.get(root_index).is_some_and(|result| matches!(
            result,
            Some(NodeResult::Interrupted {
                reason: NodeInterruption::Aborted
            })
        )),
        "the interrupted node result must retain its category"
    );
    assert!(
        state
            .results
            .get(sibling_index)
            .is_some_and(Option::is_none),
        "an independent sibling remains unstarted"
    );
    assert!(
        state.ready.contains(&(sibling_id, sibling_index)),
        "the independent sibling remains ready for ContinueIndependent"
    );
    Ok(())
}

#[tokio::test]
async fn unknown_completion_fails_closed() -> Result<(), DagError> {
    let mut join_set = JoinSet::new();
    let tracked_handle = join_set.spawn(std::future::pending::<(
        usize,
        NodeId,
        Pool,
        Result<NodeResult<usize>, DagError>,
    )>());
    let unknown_handle = join_set.spawn(std::future::pending::<(
        usize,
        NodeId,
        Pool,
        Result<NodeResult<usize>, DagError>,
    )>());
    let mut in_flight = BTreeMap::new();
    let tracked_id = tracked_handle.id();
    in_flight.insert(
        tracked_id,
        TrackedTask {
            index: 0,
            id: NodeId::new(10),
            pool: Pool::Disk,
            abort_handle: tracked_handle,
        },
    );
    let unknown = normalize_join(
        Ok((
            unknown_handle.id(),
            (
                0,
                NodeId::new(10),
                Pool::Disk,
                Ok(NodeResult::Succeeded {
                    value: 1,
                    attempts: 1,
                }),
            ),
        )),
        &mut in_flight,
    );
    assert!(
        matches!(unknown, Err(DagError::JoinFailed)),
        "unknown task IDs must fail closed"
    );

    let mismatch = normalize_join(
        Ok((
            tracked_id,
            (
                1,
                NodeId::new(11),
                Pool::Web,
                Ok(NodeResult::Succeeded {
                    value: 2,
                    attempts: 1,
                }),
            ),
        )),
        &mut in_flight,
    );
    assert!(
        matches!(mismatch, Err(DagError::JoinFailed)),
        "mismatched identity must fail closed"
    );
    join_set.abort_all();
    while join_set.join_next().await.is_some() {}
    Ok(())
}
