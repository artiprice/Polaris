use polaris_runtime_dag::{
    DagBuilder, DagError, NodeCtx, NodeError, NodeId, NodeTask, Pool, Pools, RunOptions,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::{Barrier, Notify};

#[derive(Clone)]
pub enum Behavior {
    Succeed,
    CompleteAfter(Arc<AtomicUsize>, usize),
    Fail,
    FailAfterBarrier(Arc<Barrier>),
    WaitAfterBarrier(Arc<Barrier>),
    NotifyThenPending(Arc<Notify>, Arc<AtomicBool>),
}

pub struct TestTask {
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
    retry_safe: bool,
}

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl TestTask {
    pub fn new(behavior: Behavior) -> (Self, Arc<AtomicUsize>) {
        Self::with_retry_safety(behavior, false)
    }

    pub fn retry_safe(behavior: Behavior) -> (Self, Arc<AtomicUsize>) {
        Self::with_retry_safety(behavior, true)
    }

    fn with_retry_safety(behavior: Behavior, retry_safe: bool) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                behavior,
                calls: Arc::clone(&calls),
                retry_safe,
            },
            calls,
        )
    }
}

impl NodeTask for TestTask {
    type Output = usize;

    fn retry_safe(&self) -> bool {
        self.retry_safe
    }

    fn run<'a>(
        &'a self,
        _context: NodeCtx,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, NodeError>> + Send + 'a>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let behavior = self.behavior.clone();
        Box::pin(async move {
            match behavior {
                Behavior::Succeed => Ok(1),
                Behavior::CompleteAfter(completed, required) => {
                    if completed.load(std::sync::atomic::Ordering::SeqCst) < required {
                        Err(NodeError::Failed)
                    } else {
                        completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(1)
                    }
                }
                Behavior::Fail => Err(NodeError::Failed),
                Behavior::FailAfterBarrier(barrier) => {
                    barrier.wait().await;
                    Err(NodeError::Failed)
                }
                Behavior::WaitAfterBarrier(barrier) => {
                    barrier.wait().await;
                    std::future::pending::<Result<usize, NodeError>>().await
                }
                Behavior::NotifyThenPending(started, dropped) => {
                    started.notify_one();
                    let _drop_signal = DropSignal(dropped);
                    std::future::pending::<Result<usize, NodeError>>().await
                }
            }
        })
    }
}

pub fn add_task(
    builder: &mut DagBuilder<TestTask>,
    id: NodeId,
    pool: Pool,
    behavior: Behavior,
) -> Result<Arc<AtomicUsize>, DagError> {
    let (task, calls) = TestTask::new(behavior);
    builder.node(id, pool, task)?;
    Ok(calls)
}

pub fn add_retry_safe_task(
    builder: &mut DagBuilder<TestTask>,
    id: NodeId,
    pool: Pool,
    behavior: Behavior,
) -> Result<Arc<AtomicUsize>, DagError> {
    let (task, calls) = TestTask::retry_safe(behavior);
    builder.node(id, pool, task)?;
    Ok(calls)
}

pub fn pools() -> Result<Pools, DagError> {
    Pools::new(polaris_runtime_dag::PoolLimits::default())
}

pub fn default_options() -> RunOptions {
    RunOptions::default()
}
