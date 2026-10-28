mod support;

use polaris_runtime_dag::{
    DagBuilder, DagError, DagRunStatus, NodeId, NodeResult, OnFailure, Pool, RunOptions, run,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use support::{Behavior, add_retry_safe_task, add_task, default_options, pools};
use tokio::sync::{Barrier, Notify};
use tokio_util::sync::CancellationToken;

fn start_position<O>(
    report: &polaris_runtime_dag::DagReport<O>,
    id: NodeId,
) -> Result<usize, DagError> {
    report
        .order_started()
        .iter()
        .position(|started_id| *started_id == id)
        .ok_or(DagError::GraphInvariant)
}

#[tokio::test]
async fn continue_independent_skips_failed_descendants() -> Result<(), DagError> {
    let root_id = NodeId::new(1);
    let child_id = NodeId::new(2);
    let independent_id = NodeId::new(3);
    let mut builder = DagBuilder::new();
    add_task(&mut builder, root_id, Pool::Web, Behavior::Fail)?;
    let child_calls = add_task(&mut builder, child_id, Pool::Web, Behavior::Succeed)?;
    let independent_calls = add_task(&mut builder, independent_id, Pool::Web, Behavior::Succeed)?;
    builder.edge(root_id, child_id)?;
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        run(
            builder.build()?,
            &pools()?,
            CancellationToken::new(),
            default_options(),
        ),
    )
    .await
    .map_err(|_| DagError::JoinFailed)??;
    assert_eq!(
        report.status(),
        DagRunStatus::Failed,
        "a terminal node failure makes the completed report Failed"
    );
    assert!(
        matches!(report.result(root_id), Some(NodeResult::Failed { .. })),
        "the root failure remains visible"
    );
    assert!(
        matches!(report.result(child_id), Some(NodeResult::DependencyFailed { caused_by }) if *caused_by == root_id),
        "the child must not start after its prerequisite fails"
    );
    assert!(
        matches!(
            report.result(independent_id),
            Some(NodeResult::Succeeded { .. })
        ),
        "the independent sibling must finish"
    );
    assert_eq!(
        child_calls.load(Ordering::SeqCst),
        0,
        "a skipped child must not execute"
    );
    assert_eq!(
        independent_calls.load(Ordering::SeqCst),
        1,
        "the independent sibling must execute exactly once"
    );
    Ok(())
}

#[tokio::test]
async fn fail_fast_drains_owned_tasks() -> Result<(), DagError> {
    let barrier = Arc::new(Barrier::new(2));
    let root_id = NodeId::new(1);
    let sibling_id = NodeId::new(2);
    let mut builder = DagBuilder::new();
    add_task(
        &mut builder,
        root_id,
        Pool::Web,
        Behavior::FailAfterBarrier(Arc::clone(&barrier)),
    )?;
    add_task(
        &mut builder,
        sibling_id,
        Pool::Web,
        Behavior::WaitAfterBarrier(barrier),
    )?;
    let pool_set = pools()?;
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        run(
            builder.build()?,
            &pool_set,
            CancellationToken::new(),
            RunOptions {
                on_failure: OnFailure::FailFast,
                max_parallel: None,
            },
        ),
    )
    .await
    .map_err(|_| DagError::JoinFailed)??;
    assert_eq!(
        report.status(),
        DagRunStatus::Failed,
        "FailFast must preserve failure status"
    );
    assert!(
        matches!(report.result(sibling_id), Some(NodeResult::Cancelled)),
        "the running sibling is stopped"
    );
    let stats = pool_set.stats();
    assert_eq!(
        stats.web.active, 0,
        "all sibling permits are released before return"
    );
    assert_eq!(
        stats.web.aborted_early, 1,
        "the running sibling abort is counted once"
    );
    Ok(())
}

#[tokio::test]
async fn retry_policy_bounds_safe_and_non_idempotent_attempts() -> Result<(), DagError> {
    let safe_id = NodeId::new(1);
    let unsafe_id = NodeId::new(2);
    let mut builder = DagBuilder::new();
    let safe_calls = add_retry_safe_task(&mut builder, safe_id, Pool::Web, Behavior::Fail)?;
    let unsafe_calls = add_task(&mut builder, unsafe_id, Pool::Web, Behavior::Fail)?;
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        run(
            builder.build()?,
            &pools()?,
            CancellationToken::new(),
            RunOptions {
                on_failure: OnFailure::Retry { max: 2 },
                max_parallel: None,
            },
        ),
    )
    .await
    .map_err(|_| DagError::JoinFailed)??;
    assert_eq!(report.status(), DagRunStatus::Failed, "all attempts fail");
    assert!(
        matches!(
            report.result(safe_id),
            Some(NodeResult::Failed { attempts: 3, .. })
        ),
        "retry-safe operations receive at most max plus one attempts"
    );
    assert!(
        matches!(
            report.result(unsafe_id),
            Some(NodeResult::Failed { attempts: 1, .. })
        ),
        "non-idempotent operations must not be replayed"
    );
    assert_eq!(
        safe_calls.load(Ordering::SeqCst),
        3,
        "safe task matches the retry bound"
    );
    assert_eq!(
        unsafe_calls.load(Ordering::SeqCst),
        1,
        "unsafe task runs once"
    );
    Ok(())
}

#[tokio::test]
async fn successful_diamond_runs_each_node_once() -> Result<(), DagError> {
    let root_id = NodeId::new(1);
    let left_id = NodeId::new(2);
    let right_id = NodeId::new(3);
    let join_id = NodeId::new(4);
    let mut builder = DagBuilder::new();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let root_calls = add_task(
        &mut builder,
        root_id,
        Pool::Disk,
        Behavior::CompleteAfter(Arc::clone(&completed), 0),
    )?;
    let left_calls = add_task(
        &mut builder,
        left_id,
        Pool::Disk,
        Behavior::CompleteAfter(Arc::clone(&completed), 1),
    )?;
    let right_calls = add_task(
        &mut builder,
        right_id,
        Pool::Disk,
        Behavior::CompleteAfter(Arc::clone(&completed), 1),
    )?;
    let join_calls = add_task(
        &mut builder,
        join_id,
        Pool::Disk,
        Behavior::CompleteAfter(completed, 3),
    )?;
    builder.edge(root_id, left_id)?;
    builder.edge(root_id, right_id)?;
    builder.edge(left_id, join_id)?;
    builder.edge(right_id, join_id)?;
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        run(
            builder.build()?,
            &pools()?,
            CancellationToken::new(),
            default_options(),
        ),
    )
    .await
    .map_err(|_| DagError::JoinFailed)??;
    assert_eq!(
        report.status(),
        DagRunStatus::Completed,
        "the diamond should complete"
    );
    let root_position = start_position(&report, root_id)?;
    let left_position = start_position(&report, left_id)?;
    let right_position = start_position(&report, right_id)?;
    let join_position = start_position(&report, join_id)?;
    assert!(
        root_position < left_position && root_position < right_position,
        "both branches must start after the root"
    );
    assert!(
        left_position < join_position && right_position < join_position,
        "the join must start after both prerequisites"
    );
    for (id, calls) in [
        (root_id, root_calls),
        (left_id, left_calls),
        (right_id, right_calls),
        (join_id, join_calls),
    ] {
        assert!(
            matches!(
                report.result(id),
                Some(NodeResult::Succeeded { attempts: 1, .. })
            ),
            "every diamond node should succeed once"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "each task closure should run once"
        );
    }
    Ok(())
}

#[tokio::test]
async fn pre_cancelled_run_reports_global_cancellation() -> Result<(), DagError> {
    let id = NodeId::new(1);
    let mut builder = DagBuilder::new();
    let calls = add_task(&mut builder, id, Pool::Web, Behavior::Succeed)?;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let report = tokio::time::timeout(
        Duration::from_secs(2),
        run(builder.build()?, &pools()?, cancellation, default_options()),
    )
    .await
    .map_err(|_| DagError::JoinFailed)??;
    assert_eq!(
        report.status(),
        DagRunStatus::Cancelled,
        "caller cancellation remains global"
    );
    assert!(
        matches!(report.result(id), Some(NodeResult::Cancelled)),
        "the unstarted node is canceled"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a pre-canceled run must not start operators"
    );
    Ok(())
}

#[tokio::test]
async fn caller_cancellation_drains_started_tasks() -> Result<(), DagError> {
    let id = NodeId::new(1);
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let mut builder = DagBuilder::new();
    let calls = add_task(
        &mut builder,
        id,
        Pool::Web,
        Behavior::NotifyThenPending(Arc::clone(&started), Arc::clone(&dropped)),
    )?;
    let pool_set = pools()?;
    let cancellation = CancellationToken::new();
    let dag = builder.build()?;
    let task_pools = pool_set.clone();
    let task_cancellation = cancellation.clone();
    let run_task =
        tokio::spawn(
            async move { run(dag, &task_pools, task_cancellation, default_options()).await },
        );
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .map_err(|_| DagError::JoinFailed)?;
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), run_task)
        .await
        .map_err(|_| DagError::JoinFailed)?;
    let report = result.map_err(|_| DagError::JoinFailed)??;
    assert_eq!(
        report.status(),
        DagRunStatus::Cancelled,
        "caller cancellation remains the run's terminal cause"
    );
    assert!(
        matches!(report.result(id), Some(NodeResult::Cancelled)),
        "the started node is reported as canceled"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the operator must have started"
    );
    assert_eq!(
        pool_set.stats().web.active,
        0,
        "the node permit must be released before return"
    );
    Ok(())
}

#[tokio::test]
async fn dropping_run_after_start_drops_owned_task_future() -> Result<(), DagError> {
    let id = NodeId::new(1);
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let mut builder = DagBuilder::new();
    add_task(
        &mut builder,
        id,
        Pool::Web,
        Behavior::NotifyThenPending(Arc::clone(&started), Arc::clone(&dropped)),
    )?;
    let pool_set = pools()?;
    let dag = builder.build()?;
    let task_pools = pool_set.clone();
    let run_task = tokio::spawn(async move {
        run(
            dag,
            &task_pools,
            CancellationToken::new(),
            default_options(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .map_err(|_| DagError::JoinFailed)?;
    run_task.abort();
    let join_result = tokio::time::timeout(Duration::from_secs(2), run_task)
        .await
        .map_err(|_| DagError::JoinFailed)?;
    assert!(
        join_result.is_err(),
        "aborting the owner task must cancel the run future"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| DagError::JoinFailed)?;
    assert!(
        dropped.load(Ordering::SeqCst),
        "dropping the run must drop its owned task future"
    );
    assert_eq!(
        pool_set.stats().web.active,
        0,
        "dropping the owned task releases its permit"
    );
    Ok(())
}

#[test]
fn invalid_graph_is_rejected_before_run() {
    let known_id = NodeId::new(1);
    let unknown_id = NodeId::new(2);
    let mut builder = DagBuilder::new();
    assert!(
        add_task(&mut builder, known_id, Pool::Web, Behavior::Succeed).is_ok(),
        "the node should be accepted"
    );
    assert!(
        builder.edge(unknown_id, known_id).is_ok(),
        "edge ids resolve during build"
    );
    assert!(
        matches!(builder.build(), Err(DagError::UnknownNode(id)) if id == unknown_id),
        "build must reject unknown dependency ids"
    );
}
