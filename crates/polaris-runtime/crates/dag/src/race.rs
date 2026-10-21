//! First-accepted speculative races with structured loser cleanup.

use crate::graph::{NodeCtx, NodeError, NodeTask};
use crate::{DagError, Pool, Pools};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Maximum speculative branches accepted by one race.
pub const MAX_RACE_BRANCHES: usize = 1_024;

/// Asynchronous acceptance policy for race candidates.
pub trait Accept<T>: Send + Sync + 'static {
    /// Evaluates a completed candidate. Returning `false` lets other branches continue.
    fn accept<'a>(
        &'a self,
        candidate: &'a T,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
}

/// One injected operator branch in a speculative race.
pub struct Branch<T> {
    pool: Pool,
    task: Box<dyn NodeTask<Output = T>>,
}

impl<T: Send + Sync + 'static> Branch<T> {
    /// Adds a node operator and the pool it must hold while producing its candidate.
    #[must_use]
    pub fn new<N: NodeTask<Output = T>>(pool: Pool, task: N) -> Self {
        Self {
            pool,
            task: Box::new(task),
        }
    }
}

/// Winner or exhaustion outcome of a speculative race.
#[derive(Debug, PartialEq, Eq)]
pub enum RaceOutcome<T> {
    /// The first candidate accepted by the policy; all remaining branches were canceled and
    /// joined before this result was returned.
    Won {
        /// Index in the submitted branch list.
        index: usize,
        /// Accepted candidate.
        value: T,
        /// Number of still-pending branches for which abort was initiated.
        aborted: usize,
    },
    /// All branches completed or failed without an accepted candidate.
    NoneAccepted,
}

/// Runs injected operators in parallel and returns the first candidate accepted by `accept`.
///
/// The cancellation token scopes all branches. Winner, cancellation, and error paths abort and
/// drain outstanding tasks so no loser continues holding a pool permit after return.
pub async fn race<T, A>(
    branches: Vec<Branch<T>>,
    pools: &Pools,
    accept: Arc<A>,
    cancellation: CancellationToken,
) -> Result<RaceOutcome<T>, DagError>
where
    T: Send + Sync + 'static,
    A: Accept<T>,
{
    if branches.is_empty() {
        return Ok(RaceOutcome::NoneAccepted);
    }
    if branches.len() > MAX_RACE_BRANCHES {
        return Err(DagError::InvalidLimits);
    }
    let race_token = cancellation.child_token();
    let mut tasks = JoinSet::new();
    let _cancel_on_drop = RaceCancellationGuard(race_token.clone());
    let mut pending_pools: Vec<Option<Pool>> = Vec::with_capacity(branches.len());
    for (index, branch) in branches.into_iter().enumerate() {
        let task = branch.task;
        let pool = branch.pool;
        let token = race_token.child_token();
        let pools = pools.clone();
        let accept = Arc::clone(&accept);
        tasks.spawn(async move {
            let result = run_branch(task, pool, pools, token, accept).await;
            (index, pool, result)
        });
        pending_pools.push(Some(pool));
    }

    loop {
        tokio::select! {
            _ = race_token.cancelled() => {
                race_token.cancel();
                tasks.abort_all();
                note_pending_aborts(pools, &pending_pools);
                while tasks.join_next().await.is_some() {}
                return Err(DagError::Cancelled);
            }
            joined = tasks.join_next() => {
                let Some(joined) = joined else {
                    if race_token.is_cancelled() {
                        return Err(DagError::Cancelled);
                    }
                    return Ok(RaceOutcome::NoneAccepted);
                };
                let (index, _pool, result) = match joined {
                    Ok(value) => value,
                    Err(_) => {
                        race_token.cancel();
                        tasks.abort_all();
                        note_pending_aborts(pools, &pending_pools);
                        while tasks.join_next().await.is_some() {}
                        return Err(DagError::JoinFailed);
                    }
                };
                let finished_pool = pending_pools.get_mut(index).and_then(Option::take);
                if finished_pool.is_none() {
                    race_token.cancel();
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Err(DagError::JoinFailed);
                }
                match result {
                    Ok(Some(value)) => {
                        race_token.cancel();
                        let aborted = pending_pools.iter().filter(|pool| pool.is_some()).count();
                        tasks.abort_all();
                        note_pending_aborts(pools, &pending_pools);
                        while tasks.join_next().await.is_some() {}
                        return Ok(RaceOutcome::Won {
                            index,
                            value,
                            aborted,
                        });
                    }
                    Ok(None) | Err(BranchError::Task(NodeError::Failed | NodeError::Unavailable | NodeError::Rejected | NodeError::Cancelled)) => {}
                    Err(BranchError::Infrastructure(error)) => {
                        race_token.cancel();
                        tasks.abort_all();
                        note_pending_aborts(pools, &pending_pools);
                        while tasks.join_next().await.is_some() {}
                        return Err(error);
                    }
                }
            }
        }
    }
}

async fn run_branch<T, A>(
    task: Box<dyn NodeTask<Output = T>>,
    pool: Pool,
    pools: Pools,
    cancellation: CancellationToken,
    accept: Arc<A>,
) -> Result<Option<T>, BranchError>
where
    T: Send + Sync + 'static,
    A: Accept<T>,
{
    if cancellation.is_cancelled() {
        return Err(BranchError::Task(NodeError::Cancelled));
    }
    let permit = tokio::select! {
        _ = cancellation.cancelled() => return Err(BranchError::Task(NodeError::Cancelled)),
        result = pools.acquire(pool) => result.map_err(BranchError::Infrastructure)?,
    };
    let context = NodeCtx::new(cancellation.clone(), permit.clone());
    let candidate = tokio::select! {
        _ = cancellation.cancelled() => return Err(BranchError::Task(NodeError::Cancelled)),
        result = task.run(context) => result.map_err(BranchError::Task)?,
    };
    drop(permit);
    let accepted = tokio::select! {
        _ = cancellation.cancelled() => return Err(BranchError::Task(NodeError::Cancelled)),
        accepted = accept.accept(&candidate) => accepted,
    };
    if accepted {
        Ok(Some(candidate))
    } else {
        Ok(None)
    }
}

fn note_pending_aborts(pools: &Pools, pending: &[Option<Pool>]) {
    for pool in pending.iter().flatten() {
        pools.note_aborted(*pool);
    }
}

enum BranchError {
    Task(NodeError),
    Infrastructure(DagError),
}

struct RaceCancellationGuard(CancellationToken);

impl Drop for RaceCancellationGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
