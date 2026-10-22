//! Cancellation-aware typed pipeline and ordered request-stage ledger.
use polaris_sdk::{error::HasKind, registry::FunctionalOperator};
use std::{error::Error, fmt};
use tokio_util::sync::CancellationToken;

/// Maximum number of stage records retained by one request pipeline.
pub const MAX_STAGE_RECORDS: usize = 1_024;
/// Maximum items accepted by one fold call.
pub const MAX_FOLD_ITEMS: usize = 65_536;

/// Canonical request stage order. Stages implemented by other crates remain injectable contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum RequestStage {
    /// PII masking before any model-bound data is produced.
    SanitizePii = 0,
    /// Parse and apply session directives.
    Directives = 1,
    /// S1 route and effort selection.
    Triage = 2,
    /// Assemble bounded context.
    ContextAssemble = 3,
    /// Acquire request/token budget permit.
    RateLimit = 4,
    /// Call S2.
    S2 = 5,
    /// Acquire and enforce tool leases.
    ToolLeases = 6,
    /// Execute tools inside an injected sandbox.
    Sandbox = 7,
    /// Fold tool output into content-addressed context.
    FoldCas = 8,
    /// Verify proposed mutations.
    Verification = 9,
    /// Run proof-of-done checks.
    ProofOfDone = 10,
    /// Publish durable and user-facing events.
    Events = 11,
}
const REQUEST_ORDER: [RequestStage; 12] = [
    RequestStage::SanitizePii,
    RequestStage::Directives,
    RequestStage::Triage,
    RequestStage::ContextAssemble,
    RequestStage::RateLimit,
    RequestStage::S2,
    RequestStage::ToolLeases,
    RequestStage::Sandbox,
    RequestStage::FoldCas,
    RequestStage::Verification,
    RequestStage::ProofOfDone,
    RequestStage::Events,
];

/// Outcome of one operator invocation in the pipeline's local audit trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    /// Operator returned a value.
    Succeeded,
    /// Operator returned an error.
    Failed,
    /// Cancellation won while the operator future was pending.
    Cancelled,
}

/// One completed or interrupted stage in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageRecord {
    /// Monotonic one-based invocation number in this pipeline.
    pub ordinal: u64,
    /// Stage name from an SDK operator type or the explicit request-stage label.
    pub stage: &'static str,
    /// Request stage, when the composition root used `pipe_at`.
    pub request_stage: Option<RequestStage>,
    /// Recorded outcome.
    pub status: StageStatus,
}

/// A pipeline error carries the audit trace accumulated before failure.
#[derive(Debug)]
pub enum PipelineError<E> {
    /// An operator failed; later stages did not run.
    Stage {
        stage: &'static str,
        source: E,
        trace: Vec<StageRecord>,
    },
    /// Cancellation interrupted the currently named operator.
    Cancelled {
        stage: &'static str,
        trace: Vec<StageRecord>,
    },
    /// A stage was submitted out of the canonical request order.
    StageOrder {
        previous: RequestStage,
        requested: RequestStage,
        trace: Vec<StageRecord>,
    },
    /// The per-request audit bound was reached before invoking another operator.
    TraceLimit { trace: Vec<StageRecord> },
    /// A configured fold maximum exceeded the crate hard bound.
    InvalidFoldLimit {
        limit: usize,
        maximum: usize,
        trace: Vec<StageRecord>,
    },
    /// A fold attempted to process more than its caller-supplied item limit.
    FoldLimit {
        stage: &'static str,
        limit: usize,
        trace: Vec<StageRecord>,
    },
}
impl<E: fmt::Display> fmt::Display for PipelineError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stage { stage, source, .. } => {
                write!(f, "pipeline stage {stage} failed: {source}")
            }
            Self::Cancelled { stage, .. } => write!(f, "pipeline cancelled at stage {stage}"),
            Self::StageOrder {
                previous,
                requested,
                ..
            } => write!(
                f,
                "request stage {requested:?} follows {previous:?} out of order"
            ),
            Self::TraceLimit { .. } => f.write_str("pipeline stage trace limit reached"),
            Self::InvalidFoldLimit { limit, maximum, .. } => {
                write!(f, "fold item limit {limit} exceeds hard maximum {maximum}")
            }
            Self::FoldLimit { stage, limit, .. } => {
                write!(f, "pipeline fold {stage} exceeded item limit {limit}")
            }
        }
    }
}
impl<E: Error + 'static> Error for PipelineError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stage { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Pipeline wrapper whose value is transformed by each consuming operator.
#[derive(Debug)]
pub struct Pipeline<T> {
    value: T,
    trace: Vec<StageRecord>,
    cancellation: Option<CancellationToken>,
    last_request_stage: Option<RequestStage>,
}
impl<T> Pipeline<T> {
    /// Start a pipeline with a value and no cancellation source.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            value,
            trace: Vec::new(),
            cancellation: None,
            last_request_stage: None,
        }
    }

    /// Attach a cancellation token. Dropping an in-flight stage future is the cancellation
    /// boundary; operators must release permits/resources through RAII.
    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = Some(token);
        self
    }

    /// Audit trace accumulated by the pipeline.
    #[must_use]
    pub fn trace(&self) -> &[StageRecord] {
        &self.trace
    }

    /// Consume the pipeline and return its current value.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Require a complete 12-stage request before returning a value as end-to-end complete.
    /// Pipelines built with generic `pipe` or with skipped `pipe_at` stages cannot pass this gate.
    pub fn finish_request(self) -> Result<T, IncompleteRequest> {
        let recorded = self
            .trace
            .iter()
            .filter_map(|record| record.request_stage)
            .collect::<Vec<_>>();
        let mut stages = recorded.iter().copied();
        for required in REQUEST_ORDER {
            if stages.next() != Some(required) {
                return Err(IncompleteRequest {
                    required,
                    trace: self.trace,
                });
            }
        }
        if let Some(extra) = stages.next() {
            return Err(IncompleteRequest {
                required: extra,
                trace: self.trace,
            });
        }
        Ok(self.value)
    }

    /// Run an operator without an explicit request-stage label.
    pub async fn pipe<Op>(self, operator: &Op) -> Result<Pipeline<Op::Out>, PipelineError<Op::Err>>
    where
        Op: FunctionalOperator<In = T>,
    {
        self.run(operator, std::any::type_name::<Op>(), None).await
    }

    /// Run a real owner-provided operator at its canonical request stage. This labels and orders an
    /// actual operator; it does not manufacture any stage implementation.
    pub async fn pipe_at<Op>(
        self,
        stage: RequestStage,
        operator: &Op,
    ) -> Result<Pipeline<Op::Out>, PipelineError<Op::Err>>
    where
        Op: FunctionalOperator<In = T>,
    {
        if let Some(previous) = self.last_request_stage {
            if stage <= previous {
                return Err(PipelineError::StageOrder {
                    previous,
                    requested: stage,
                    trace: self.trace,
                });
            }
        }
        self.run(operator, stage_name(stage), Some(stage)).await
    }

    async fn run<Op>(
        mut self,
        operator: &Op,
        stage: &'static str,
        request_stage: Option<RequestStage>,
    ) -> Result<Pipeline<Op::Out>, PipelineError<Op::Err>>
    where
        Op: FunctionalOperator<In = T>,
    {
        if self.trace.len() >= MAX_STAGE_RECORDS {
            return Err(PipelineError::TraceLimit { trace: self.trace });
        }
        let ordinal = next_ordinal(self.trace.len());
        let cancellation = self.cancellation.clone();
        let result = match cancellation.as_ref() {
            Some(token) => tokio::select! {
                biased;
                () = token.cancelled() => {
                    self.trace.push(StageRecord { ordinal, stage, request_stage, status: StageStatus::Cancelled });
                    return Err(PipelineError::Cancelled { stage, trace: self.trace });
                }
                result = operator.apply(self.value) => result,
            },
            None => operator.apply(self.value).await,
        };
        match result {
            Ok(value) => {
                self.trace.push(StageRecord {
                    ordinal,
                    stage,
                    request_stage,
                    status: StageStatus::Succeeded,
                });
                Ok(Pipeline {
                    value,
                    trace: self.trace,
                    cancellation: self.cancellation,
                    last_request_stage: request_stage.or(self.last_request_stage),
                })
            }
            Err(source) => {
                self.trace.push(StageRecord {
                    ordinal,
                    stage,
                    request_stage,
                    status: StageStatus::Failed,
                });
                Err(PipelineError::Stage {
                    stage,
                    source,
                    trace: self.trace,
                })
            }
        }
    }

    /// Execute exactly one selected branch. The unselected operator is never polled.
    pub async fn fork_branch<A, B, P>(
        self,
        predicate: P,
        fast: &A,
        deep: &B,
    ) -> Result<Pipeline<A::Out>, PipelineError<A::Err>>
    where
        P: FnOnce(&T) -> bool,
        A: FunctionalOperator<In = T>,
        B: FunctionalOperator<In = T, Out = A::Out, Err = A::Err>,
    {
        if predicate(&self.value) {
            self.pipe(fast).await
        } else {
            self.pipe(deep).await
        }
    }

    /// Fold a bounded iterator through a real owner-supplied accumulator operator.
    pub async fn fold<Op, I, Item>(
        mut self,
        operator: &Op,
        items: I,
        max_items: usize,
    ) -> Result<Pipeline<T>, PipelineError<Op::Err>>
    where
        Op: FoldOperator<T, Item>,
        I: IntoIterator<Item = Item>,
    {
        let stage = std::any::type_name::<Op>();
        if max_items > MAX_FOLD_ITEMS {
            return Err(PipelineError::InvalidFoldLimit {
                limit: max_items,
                maximum: MAX_FOLD_ITEMS,
                trace: self.trace,
            });
        }
        if self.trace.len() >= MAX_STAGE_RECORDS {
            return Err(PipelineError::TraceLimit { trace: self.trace });
        }
        let ordinal = next_ordinal(self.trace.len());
        let mut processed = 0_usize;
        let mut iterator = items.into_iter();
        loop {
            let Some(item) = iterator.next() else {
                break;
            };
            if processed >= max_items {
                self.trace.push(StageRecord {
                    ordinal,
                    stage,
                    request_stage: None,
                    status: StageStatus::Failed,
                });
                return Err(PipelineError::FoldLimit {
                    stage,
                    limit: max_items,
                    trace: self.trace,
                });
            }
            let cancellation = self.cancellation.clone();
            let result = match cancellation.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    () = token.cancelled() => {
                        self.trace.push(StageRecord { ordinal, stage, request_stage: None, status: StageStatus::Cancelled });
                        return Err(PipelineError::Cancelled { stage, trace: self.trace });
                    }
                    result = operator.fold(self.value, item) => result,
                },
                None => operator.fold(self.value, item).await,
            };
            match result {
                Ok(next) => self.value = next,
                Err(source) => {
                    self.trace.push(StageRecord {
                        ordinal,
                        stage,
                        request_stage: None,
                        status: StageStatus::Failed,
                    });
                    return Err(PipelineError::Stage {
                        stage,
                        source,
                        trace: self.trace,
                    });
                }
            }
            processed = processed.saturating_add(1);
        }
        self.trace.push(StageRecord {
            ordinal,
            stage,
            request_stage: None,
            status: StageStatus::Succeeded,
        });
        Ok(self)
    }

    /// Run an asynchronous guard against the current value without changing it.
    pub async fn assert_guard<G>(mut self, guard: &G) -> Result<Self, PipelineError<G::Err>>
    where
        G: Guard<T>,
    {
        let stage = std::any::type_name::<G>();
        if self.trace.len() >= MAX_STAGE_RECORDS {
            return Err(PipelineError::TraceLimit { trace: self.trace });
        }
        let ordinal = next_ordinal(self.trace.len());
        let cancellation = self.cancellation.clone();
        let result = match cancellation.as_ref() {
            Some(token) => tokio::select! {
                biased;
                () = token.cancelled() => {
                    self.trace.push(StageRecord { ordinal, stage, request_stage: None, status: StageStatus::Cancelled });
                    return Err(PipelineError::Cancelled { stage, trace: self.trace });
                }
                result = guard.check(&self.value) => result,
            },
            None => guard.check(&self.value).await,
        };
        match result {
            Ok(()) => {
                self.trace.push(StageRecord {
                    ordinal,
                    stage,
                    request_stage: None,
                    status: StageStatus::Succeeded,
                });
                Ok(self)
            }
            Err(source) => {
                self.trace.push(StageRecord {
                    ordinal,
                    stage,
                    request_stage: None,
                    status: StageStatus::Failed,
                });
                Err(PipelineError::Stage {
                    stage,
                    source,
                    trace: self.trace,
                })
            }
        }
    }
}

/// Error returned when a pipeline has not recorded every required request stage in order.
#[derive(Debug)]
pub struct IncompleteRequest {
    /// First missing or unexpected stage.
    pub required: RequestStage,
    /// Recorded pipeline history.
    pub trace: Vec<StageRecord>,
}
impl fmt::Display for IncompleteRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "request is incomplete at stage {:?}", self.required)
    }
}
impl Error for IncompleteRequest {}

/// Asynchronous accumulator contract used by [`Pipeline::fold`].
pub trait FoldOperator<Acc, Item>: Send + Sync {
    /// Failure from the fold operation.
    type Err: HasKind + Send;
    /// Fold one item into the accumulator.
    fn fold(
        &self,
        accumulator: Acc,
        item: Item,
    ) -> impl std::future::Future<Output = Result<Acc, Self::Err>> + Send;
}

/// Asynchronous invariant check used by [`Pipeline::assert_guard`].
pub trait Guard<T>: Send + Sync {
    /// Failure to establish the invariant.
    type Err: HasKind + Send;
    /// Check without taking ownership of the value.
    fn check(&self, value: &T) -> impl std::future::Future<Output = Result<(), Self::Err>> + Send;
}

fn next_ordinal(length: usize) -> u64 {
    match u64::try_from(length) {
        Ok(value) => value.saturating_add(1),
        Err(_) => u64::MAX,
    }
}
const fn stage_name(stage: RequestStage) -> &'static str {
    match stage {
        RequestStage::SanitizePii => "sanitize_pii",
        RequestStage::Directives => "directives",
        RequestStage::Triage => "triage",
        RequestStage::ContextAssemble => "context_assemble",
        RequestStage::RateLimit => "rate_limit",
        RequestStage::S2 => "s2",
        RequestStage::ToolLeases => "tool_leases",
        RequestStage::Sandbox => "sandbox",
        RequestStage::FoldCas => "fold_cas",
        RequestStage::Verification => "verification",
        RequestStage::ProofOfDone => "proof_of_done",
        RequestStage::Events => "events",
    }
}
