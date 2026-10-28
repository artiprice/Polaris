//! Validated graph storage and exactly-once asynchronous scheduling.

use crate::{DagError, Pool, PoolPermit, Pools};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::task::{AbortHandle, Id as TokioTaskId, JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

/// Absolute upper bound for nodes in one graph.
pub const MAX_DAG_NODES: usize = 100_000;
/// Absolute upper bound for dependency edges in one graph.
pub const MAX_DAG_EDGES: usize = 1_000_000;

/// Stable caller-assigned node identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// Creates an id. Every `u32` value is valid.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Numeric id value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Explicit graph-size bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagLimits {
    /// Maximum nodes accepted by one builder.
    pub max_nodes: usize,
    /// Maximum dependency edges accepted by one builder.
    pub max_edges: usize,
}

impl Default for DagLimits {
    fn default() -> Self {
        Self {
            max_nodes: 10_000,
            max_edges: 100_000,
        }
    }
}

impl DagLimits {
    /// Creates positive graph-size limits.
    pub fn new(max_nodes: usize, max_edges: usize) -> Result<Self, DagError> {
        if !valid_limits(max_nodes, max_edges) {
            return Err(DagError::InvalidLimits);
        }
        Ok(Self {
            max_nodes,
            max_edges,
        })
    }
}

/// Structured node failure. Payloads are stable categories, not arbitrary backend text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeError {
    /// The operation failed for an unspecified task-specific reason.
    Failed,
    /// The underlying service or resource was unavailable.
    Unavailable,
    /// The operation rejected the request or candidate.
    Rejected,
    /// The operator observed cooperative cancellation.
    Cancelled,
}

/// Why a scheduler task ended before returning a structured node result.
///
/// An interruption has an unknown attempt count. The scheduler does not retry an interrupted
/// task, even when `OnFailure::Retry` is selected, because its side-effect state is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeInterruption {
    /// The operator task panicked. Panic payloads are intentionally not exposed.
    Panicked,
    /// The operator task was aborted before it returned a structured result.
    Aborted,
}

/// Context supplied to every node attempt.
pub struct NodeCtx {
    cancellation: CancellationToken,
    permit: PoolPermit,
}

impl NodeCtx {
    pub(crate) fn new(cancellation: CancellationToken, permit: PoolPermit) -> Self {
        Self {
            cancellation,
            permit,
        }
    }

    /// Child cancellation token for cooperative operator shutdown.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Whether this node or its parent run has been canceled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Named pool held for this attempt.
    #[must_use]
    pub fn pool(&self) -> Pool {
        self.permit.pool()
    }
}

/// Injected operation for a graph node. The returned future may borrow the operator.
pub trait NodeTask: Send + Sync + 'static {
    /// Successful node output type.
    type Output: Send + 'static;

    /// Whether repeated calls are safe after an unsuccessful attempt.
    ///
    /// The default is false because node operations may have side effects. A run retry policy is
    /// applied only to operators that explicitly opt into retry safety.
    fn retry_safe(&self) -> bool {
        false
    }

    /// Runs one attempt with an owned pool permit and child cancellation context.
    fn run<'a>(
        &'a self,
        context: NodeCtx,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, NodeError>> + Send + 'a>>;
}

/// Closure-backed operator wrapper for injected async implementations.
pub struct FunctionalOperator<F> {
    operation: F,
    retry_safe: bool,
}

impl<F> FunctionalOperator<F> {
    /// Wraps an operation without supplying a fake backend implementation.
    #[must_use]
    pub const fn new(operation: F) -> Self {
        Self {
            operation,
            retry_safe: false,
        }
    }

    /// Wraps an operation whose caller has established retry idempotence.
    #[must_use]
    pub const fn retry_safe(operation: F) -> Self {
        Self {
            operation,
            retry_safe: true,
        }
    }
}

impl<F, Fut, O> NodeTask for FunctionalOperator<F>
where
    F: Fn(NodeCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, NodeError>> + Send + 'static,
    O: Send + 'static,
{
    type Output = O;

    fn retry_safe(&self) -> bool {
        self.retry_safe
    }

    fn run<'a>(
        &'a self,
        context: NodeCtx,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, NodeError>> + Send + 'a>> {
        Box::pin((self.operation)(context))
    }
}

/// Policy applied after a node attempt fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// Stop scheduling independent nodes after the first terminal failure.
    FailFast,
    /// Mark descendants of failed nodes as dependency-failed; continue independent branches.
    ContinueIndependent,
    /// Retry a failed node up to `max` times after its first attempt.
    Retry {
        /// Number of additional attempts.
        max: u8,
    },
}

/// Per-run scheduling options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// Failure handling policy.
    pub on_failure: OnFailure,
    /// Optional global simultaneous-node cap, in addition to pool limits.
    pub max_parallel: Option<u32>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            on_failure: OnFailure::ContinueIndependent,
            max_parallel: None,
        }
    }
}

/// A node result retained for caller inspection after the run.
#[derive(Debug, PartialEq, Eq)]
pub enum NodeResult<O> {
    /// Node completed successfully.
    Succeeded {
        /// Operator output.
        value: O,
        /// Number of attempts used.
        attempts: u32,
    },
    /// Node exhausted its configured attempts.
    Failed {
        /// Stable failure category.
        error: NodeError,
        /// Number of attempts used.
        attempts: u32,
    },
    /// The operator task ended without a structured result; its attempt count is unknown.
    Interrupted {
        /// Scheduler-observed interruption category.
        reason: NodeInterruption,
    },
    /// A prerequisite failed, so this node was never started.
    DependencyFailed {
        /// Failed prerequisite that first blocked this dependency chain.
        caused_by: NodeId,
    },
    /// Run cancellation or fail-fast stopped this node before a result was recorded.
    Cancelled,
}

/// Terminal state of one graph execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagRunStatus {
    /// Scheduler finished without node failure or cancellation.
    Completed,
    /// At least one node failed after its configured attempts.
    Failed,
    /// The caller canceled the run.
    Cancelled,
}

/// Per-run results and scheduler dispatch order.
#[derive(Debug, PartialEq, Eq)]
pub struct DagReport<O> {
    status: DagRunStatus,
    results: Vec<(NodeId, NodeResult<O>)>,
    order_started: Vec<NodeId>,
}

impl<O> DagReport<O> {
    /// Run terminal state.
    #[must_use]
    pub const fn status(&self) -> DagRunStatus {
        self.status
    }

    /// Results in deterministic node-id order.
    #[must_use]
    pub fn results(&self) -> &[(NodeId, NodeResult<O>)] {
        &self.results
    }

    /// Nodes in scheduler dispatch order. Operators may enter their futures later.
    #[must_use]
    pub fn order_started(&self) -> &[NodeId] {
        &self.order_started
    }

    /// Returns a node result by id.
    #[must_use]
    pub fn result(&self, id: NodeId) -> Option<&NodeResult<O>> {
        self.results
            .binary_search_by_key(&id, |(node_id, _)| *node_id)
            .ok()
            .and_then(|index| self.results.get(index).map(|(_, result)| result))
    }
}

struct PendingNode<T> {
    id: NodeId,
    pool: Pool,
    task: T,
}

struct Node<T> {
    id: NodeId,
    pool: Pool,
    task: Arc<T>,
    dependencies: Vec<usize>,
    dependents: Vec<usize>,
}

/// Builder for a validated directed acyclic task graph.
pub struct DagBuilder<T> {
    limits: DagLimits,
    nodes: Vec<PendingNode<T>>,
    indices: BTreeMap<NodeId, usize>,
    edges: Vec<(NodeId, NodeId)>,
    edge_set: BTreeSet<(NodeId, NodeId)>,
}

impl<T: NodeTask> DagBuilder<T> {
    /// Creates a builder with the default 10,000 node and 100,000 edge bounds.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(DagLimits::default())
    }

    /// Creates a builder with explicit graph-size bounds.
    #[must_use]
    pub fn with_limits(limits: DagLimits) -> Self {
        Self {
            limits,
            nodes: Vec::new(),
            indices: BTreeMap::new(),
            edges: Vec::new(),
            edge_set: BTreeSet::new(),
        }
    }

    /// Adds a unique node and its named pool operation.
    pub fn node(&mut self, id: NodeId, pool: Pool, task: T) -> Result<&mut Self, DagError> {
        if self.nodes.len() >= self.limits.max_nodes.min(MAX_DAG_NODES) {
            return Err(DagError::InvalidLimits);
        }
        if self.indices.contains_key(&id) {
            return Err(DagError::DuplicateNode(id));
        }
        let index = self.nodes.len();
        self.nodes.push(PendingNode { id, pool, task });
        self.indices.insert(id, index);
        Ok(self)
    }

    /// Adds prerequisite edge `from -> to`.
    pub fn edge(&mut self, from: NodeId, to: NodeId) -> Result<&mut Self, DagError> {
        if self.edges.len() >= self.limits.max_edges.min(MAX_DAG_EDGES) {
            return Err(DagError::InvalidLimits);
        }
        if !self.edge_set.insert((from, to)) {
            return Err(DagError::DuplicateEdge);
        }
        self.edges.push((from, to));
        Ok(self)
    }

    /// Resolves all ids and rejects cyclic or malformed graphs.
    pub fn build(self) -> Result<Dag<T>, DagError> {
        if !valid_limits(self.limits.max_nodes, self.limits.max_edges) {
            return Err(DagError::InvalidLimits);
        }
        let count = self.nodes.len();
        let mut indegree = vec![0_usize; count];
        let mut dependencies = vec![Vec::new(); count];
        let mut dependents = vec![Vec::new(); count];
        for (from, to) in &self.edges {
            let from_index = *self.indices.get(from).ok_or(DagError::UnknownNode(*from))?;
            let to_index = *self.indices.get(to).ok_or(DagError::UnknownNode(*to))?;
            let degree = indegree.get_mut(to_index).ok_or(DagError::GraphInvariant)?;
            *degree = degree.checked_add(1).ok_or(DagError::InvalidLimits)?;
            dependencies
                .get_mut(to_index)
                .ok_or(DagError::GraphInvariant)?
                .push(from_index);
            dependents
                .get_mut(from_index)
                .ok_or(DagError::GraphInvariant)?
                .push(to_index);
        }
        let mut ready: BTreeSet<(NodeId, usize)> = self
            .nodes
            .iter()
            .zip(indegree.iter())
            .enumerate()
            .filter_map(|(index, (node, degree))| (*degree == 0).then_some((node.id, index)))
            .collect();
        let mut visited = 0_usize;
        while let Some((_, index)) = ready.pop_first() {
            visited = visited.checked_add(1).ok_or(DagError::InvalidLimits)?;
            let children = dependents.get(index).ok_or(DagError::GraphInvariant)?;
            for dependent in children {
                let degree = indegree
                    .get_mut(*dependent)
                    .ok_or(DagError::GraphInvariant)?;
                *degree = degree.checked_sub(1).ok_or(DagError::GraphInvariant)?;
                if *degree == 0 {
                    let node = self.nodes.get(*dependent).ok_or(DagError::GraphInvariant)?;
                    ready.insert((node.id, *dependent));
                }
            }
        }
        if visited != count {
            return Err(DagError::Cycle);
        }
        let mut nodes = Vec::with_capacity(count);
        for (index, pending) in self.nodes.into_iter().enumerate() {
            nodes.push(Node {
                id: pending.id,
                pool: pending.pool,
                task: Arc::new(pending.task),
                dependencies: dependencies
                    .get(index)
                    .cloned()
                    .ok_or(DagError::GraphInvariant)?,
                dependents: dependents
                    .get(index)
                    .cloned()
                    .ok_or(DagError::GraphInvariant)?,
            });
        }
        Ok(Dag { nodes })
    }
}

fn valid_limits(max_nodes: usize, max_edges: usize) -> bool {
    max_nodes > 0 && max_nodes <= MAX_DAG_NODES && max_edges > 0 && max_edges <= MAX_DAG_EDGES
}

impl<T: NodeTask> Default for DagBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A validated acyclic graph. Nodes cannot be added after construction.
pub struct Dag<T> {
    nodes: Vec<Node<T>>,
}

struct RunCancellationGuard(CancellationToken);

impl Drop for RunCancellationGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct TrackedTask {
    index: usize,
    id: NodeId,
    pool: Pool,
    abort_handle: AbortHandle,
}

type NodeCompletion<O> = (usize, NodeId, Pool, Result<NodeResult<O>, DagError>);
type JoinedTask<O> = Result<(TokioTaskId, NodeCompletion<O>), JoinError>;

struct SchedulerState<O> {
    indegree: Vec<usize>,
    ready: BTreeSet<(NodeId, usize)>,
    started: BTreeSet<usize>,
    results: Vec<Option<NodeResult<O>>>,
}

enum JoinedNode<O> {
    Completed {
        index: usize,
        id: NodeId,
        result: Result<NodeResult<O>, DagError>,
    },
    Interrupted {
        index: usize,
        id: NodeId,
        reason: NodeInterruption,
    },
}

fn normalize_join<O>(
    joined: JoinedTask<O>,
    in_flight: &mut BTreeMap<TokioTaskId, TrackedTask>,
) -> Result<JoinedNode<O>, DagError> {
    match joined {
        Ok((task_id, (index, id, pool, result))) => {
            let tracked = in_flight.remove(&task_id).ok_or(DagError::JoinFailed)?;
            if tracked.index != index || tracked.id != id || tracked.pool != pool {
                return Err(DagError::JoinFailed);
            }
            Ok(JoinedNode::Completed { index, id, result })
        }
        Err(error) => {
            let reason = if error.is_panic() {
                NodeInterruption::Panicked
            } else {
                NodeInterruption::Aborted
            };
            let tracked = in_flight.remove(&error.id()).ok_or(DagError::JoinFailed)?;
            Ok(JoinedNode::Interrupted {
                index: tracked.index,
                id: tracked.id,
                reason,
            })
        }
    }
}

fn record_node_result<T: NodeTask>(
    dag: &Dag<T>,
    index: usize,
    id: NodeId,
    node_result: NodeResult<T::Output>,
    state: &mut SchedulerState<T::Output>,
) -> Result<(bool, bool), DagError> {
    let success = matches!(node_result, NodeResult::Succeeded { .. });
    let failed = matches!(
        node_result,
        NodeResult::Failed { .. } | NodeResult::Interrupted { .. }
    );
    let cancelled = matches!(node_result, NodeResult::Cancelled);
    *state
        .results
        .get_mut(index)
        .ok_or(DagError::GraphInvariant)? = Some(node_result);
    if success {
        let node = dag.nodes.get(index).ok_or(DagError::GraphInvariant)?;
        for dependent in &node.dependents {
            let degree = state
                .indegree
                .get_mut(*dependent)
                .ok_or(DagError::GraphInvariant)?;
            *degree = degree.checked_sub(1).ok_or(DagError::GraphInvariant)?;
            if *degree == 0 && !state.started.contains(dependent) {
                let dependent_node = dag.nodes.get(*dependent).ok_or(DagError::GraphInvariant)?;
                state.ready.insert((dependent_node.id, *dependent));
            }
        }
    } else if failed || cancelled {
        skip_descendants(
            dag,
            index,
            id,
            &mut state.results,
            &mut state.started,
            &mut state.ready,
        )?;
    }
    Ok((failed, cancelled))
}

fn record_joined_node<T: NodeTask>(
    dag: &Dag<T>,
    event: JoinedNode<T::Output>,
    state: &mut SchedulerState<T::Output>,
) -> Result<(NodeId, bool, bool), DagError> {
    let (index, id, node_result) = match event {
        JoinedNode::Completed { index, id, result } => (index, id, result?),
        JoinedNode::Interrupted { index, id, reason } => {
            (index, id, NodeResult::Interrupted { reason })
        }
    };
    let (failed, cancelled) = record_node_result(dag, index, id, node_result, state)?;
    Ok((id, failed, cancelled))
}

fn store_drained_result<O>(
    index: usize,
    result: NodeResult<O>,
    state: &mut SchedulerState<O>,
) -> Result<(), DagError> {
    let slot = state
        .results
        .get_mut(index)
        .ok_or(DagError::GraphInvariant)?;
    if slot.is_some() {
        return Err(DagError::JoinFailed);
    }
    *slot = Some(result);
    Ok(())
}

async fn abort_and_drain<O: Send + 'static>(
    join_set: &mut JoinSet<NodeCompletion<O>>,
    in_flight: &mut BTreeMap<TokioTaskId, TrackedTask>,
    state: &mut SchedulerState<O>,
) -> Result<(), DagError> {
    join_set.abort_all();
    let mut drain_error = None;
    while let Some(joined) = join_set.join_next_with_id().await {
        let event = match normalize_join(joined, in_flight) {
            Ok(event) => event,
            Err(error) => {
                if drain_error.is_none() {
                    drain_error = Some(error);
                }
                continue;
            }
        };
        let stored = match event {
            JoinedNode::Completed { index, result, .. } => match result {
                Ok(node_result) => store_drained_result(index, node_result, state),
                Err(error) => {
                    if drain_error.is_none() {
                        drain_error = Some(error);
                    }
                    continue;
                }
            },
            JoinedNode::Interrupted { index, reason, .. } => {
                let node_result = match reason {
                    NodeInterruption::Panicked => NodeResult::Interrupted { reason },
                    NodeInterruption::Aborted => NodeResult::Cancelled,
                };
                store_drained_result(index, node_result, state)
            }
        };
        if drain_error.is_none() {
            drain_error = stored.err();
        }
    }
    if !in_flight.is_empty() && drain_error.is_none() {
        drain_error = Some(DagError::JoinFailed);
    }
    match drain_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Runs a validated graph with readiness scheduling, bounded parallelism and structured abort.
pub async fn run<T: NodeTask>(
    dag: Dag<T>,
    pools: &Pools,
    cancellation: CancellationToken,
    options: RunOptions,
) -> Result<DagReport<T::Output>, DagError> {
    if options.max_parallel == Some(0) {
        return Err(DagError::InvalidLimits);
    }
    let count = dag.nodes.len();
    let parallelism = match options.max_parallel {
        Some(value) => usize::try_from(value).map_err(|_| DagError::InvalidLimits)?,
        None => count.max(1),
    };
    let run_token = cancellation.child_token();
    let indegree: Vec<usize> = dag
        .nodes
        .iter()
        .map(|node| node.dependencies.len())
        .collect();
    let ready: BTreeSet<(NodeId, usize)> = dag
        .nodes
        .iter()
        .zip(indegree.iter())
        .enumerate()
        .filter_map(|(index, (node, degree))| (*degree == 0).then_some((node.id, index)))
        .collect();
    let mut state = SchedulerState {
        indegree,
        ready,
        started: BTreeSet::new(),
        results: std::iter::repeat_with(|| None).take(count).collect(),
    };
    let mut order_started = Vec::with_capacity(count);
    let mut join_set = JoinSet::new();
    let _cancel_on_drop = RunCancellationGuard(run_token.clone());
    let mut in_flight: BTreeMap<TokioTaskId, TrackedTask> = BTreeMap::new();
    let mut status = DagRunStatus::Completed;
    let mut had_failure = false;

    loop {
        if run_token.is_cancelled() {
            status = DagRunStatus::Cancelled;
            break;
        }
        while join_set.len() < parallelism {
            let Some((_, index)) = state.ready.pop_first() else {
                break;
            };
            if !state.started.insert(index) {
                continue;
            }
            let node = dag.nodes.get(index).ok_or(DagError::GraphInvariant)?;
            order_started.push(node.id);
            let task = Arc::clone(&node.task);
            let pool = node.pool;
            let token = run_token.child_token();
            let pools = pools.clone();
            let id = node.id;
            let abort_handle = join_set.spawn(async move {
                let result = execute_node(task, pool, id, pools, token, options.on_failure).await;
                (index, id, pool, result)
            });
            let task_id = abort_handle.id();
            in_flight.insert(
                task_id,
                TrackedTask {
                    index,
                    id,
                    pool,
                    abort_handle,
                },
            );
        }

        if join_set.is_empty() {
            if state.ready.is_empty() {
                break;
            }
            // An acyclic validated graph cannot reach this state unless its dependency state was
            // corrupted internally; report a fatal scheduler inconsistency.
            return Err(DagError::JoinFailed);
        }

        tokio::select! {
            _ = run_token.cancelled() => {
                status = DagRunStatus::Cancelled;
                break;
            }
            joined = join_set.join_next_with_id() => {
                let Some(joined) = joined else { break; };
                let event = match normalize_join(joined, &mut in_flight) {
                    Ok(event) => event,
                    Err(error) => {
                        run_token.cancel();
                        for tracked in in_flight.values() {
                            if !tracked.abort_handle.is_finished() {
                                pools.note_aborted(tracked.pool);
                            }
                        }
                        abort_and_drain(&mut join_set, &mut in_flight, &mut state).await?;
                        return Err(error);
                    }
                };
                let recorded = record_joined_node(
                    &dag,
                    event,
                    &mut state,
                );
                let (_, failed, cancelled) = match recorded {
                    Ok(recorded) => recorded,
                    Err(error) => {
                        run_token.cancel();
                        for tracked in in_flight.values() {
                            if !tracked.abort_handle.is_finished() {
                                pools.note_aborted(tracked.pool);
                            }
                        }
                        abort_and_drain(&mut join_set, &mut in_flight, &mut state).await?;
                        return Err(error);
                    }
                };
                had_failure |= failed || cancelled;
                if (failed || cancelled) && options.on_failure == OnFailure::FailFast {
                    status = if cancelled {
                        DagRunStatus::Cancelled
                    } else {
                        DagRunStatus::Failed
                    };
                    run_token.cancel();
                    break;
                }
            }
        }
    }

    if had_failure && status == DagRunStatus::Completed {
        status = DagRunStatus::Failed;
    }
    if status != DagRunStatus::Completed || !join_set.is_empty() {
        run_token.cancel();
        for tracked in in_flight.values() {
            if !tracked.abort_handle.is_finished() {
                pools.note_aborted(tracked.pool);
            }
        }
        abort_and_drain(&mut join_set, &mut in_flight, &mut state).await?;
    }
    for (index, slot) in state.results.iter_mut().enumerate() {
        if slot.is_none() {
            let _node = dag.nodes.get(index).ok_or(DagError::GraphInvariant)?;
            *slot = Some(NodeResult::Cancelled);
            if state.started.contains(&index) && status == DagRunStatus::Completed {
                status = DagRunStatus::Cancelled;
            }
        }
    }
    if run_token.is_cancelled() && status == DagRunStatus::Completed {
        status = DagRunStatus::Cancelled;
    }
    let mut ordered: Vec<(NodeId, NodeResult<T::Output>)> = dag
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            state
                .results
                .get_mut(index)
                .and_then(Option::take)
                .map(|result| (node.id, result))
        })
        .collect();
    ordered.sort_unstable_by_key(|(id, _)| *id);
    Ok(DagReport {
        status,
        results: ordered,
        order_started,
    })
}

async fn execute_node<T: NodeTask>(
    task: Arc<T>,
    pool: Pool,
    _id: NodeId,
    pools: Pools,
    cancellation: CancellationToken,
    policy: OnFailure,
) -> Result<NodeResult<T::Output>, DagError> {
    let retries = if task.retry_safe() {
        match policy {
            OnFailure::Retry { max } => u32::from(max),
            OnFailure::FailFast | OnFailure::ContinueIndependent => 0,
        }
    } else {
        0
    };
    let attempts = retries.saturating_add(1);
    let mut last_error = NodeError::Failed;
    for attempt in 1..=attempts {
        if cancellation.is_cancelled() {
            return Ok(NodeResult::Cancelled);
        }
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Ok(NodeResult::Cancelled),
            result = pools.acquire(pool) => result?,
        };
        let context = NodeCtx::new(cancellation.clone(), permit.clone());
        let result = tokio::select! {
            _ = cancellation.cancelled() => return Ok(NodeResult::Cancelled),
            result = task.run(context) => result,
        };
        match result {
            Ok(value) => {
                return Ok(NodeResult::Succeeded {
                    value,
                    attempts: attempt,
                });
            }
            Err(NodeError::Cancelled) => {
                return Ok(NodeResult::Cancelled);
            }
            Err(error) => last_error = error,
        }
    }
    Ok(NodeResult::Failed {
        error: last_error,
        attempts,
    })
}

fn skip_descendants<T: NodeTask>(
    dag: &Dag<T>,
    failed_index: usize,
    failed_id: NodeId,
    results: &mut [Option<NodeResult<T::Output>>],
    started: &mut BTreeSet<usize>,
    ready: &mut BTreeSet<(NodeId, usize)>,
) -> Result<(), DagError> {
    let mut pending = VecDeque::new();
    let failed = dag
        .nodes
        .get(failed_index)
        .ok_or(DagError::GraphInvariant)?;
    pending.extend(failed.dependents.iter().copied());
    while let Some(index) = pending.pop_front() {
        if started.contains(&index) {
            continue;
        }
        let node = dag.nodes.get(index).ok_or(DagError::GraphInvariant)?;
        ready.remove(&(node.id, index));
        let slot = results.get_mut(index).ok_or(DagError::GraphInvariant)?;
        if slot.is_none() {
            *slot = Some(NodeResult::DependencyFailed {
                caused_by: failed_id,
            });
            pending.extend(node.dependents.iter().copied());
        }
        started.insert(index);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
