//! Explicit degradation and worker-epoch admission for schema-selected S1 consumers.
//!
//! Inputs must already be masked. Answers remain advice: this owner creates no capability or
//! approval witness. Host readiness reports must come from the authenticated worker supervisor.
use crate::schemas::{
    CalibrationContext, Consumer, FailMode, SchemaCatalog, Verdict, apply_threshold_with_context,
};
use polaris_sdk::{
    registry::{DecisionEngine, DecisionError},
    system_one::{Decision, DecisionInput, QuestionId},
};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::Semaphore;

const OPEN: u64 = 1 << 31;
const FAILURES: u64 = OPEN - 1;
const EPOCH_SHIFT: u32 = 32;
const MAX_INPUT_BYTES: usize = 262_144;

/// Operational bounds supplied by the host; these are not model calibration thresholds.
#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig {
    /// Consecutive transport failures before admission closes until a host readiness report.
    pub failure_threshold: u32,
    /// Maximum simultaneous policy calls; overload returns immediately without queueing.
    pub max_inflight: usize,
    /// Maximum per-call wall time, also capped by the selected schema's deadline.
    pub max_deadline: Duration,
}

/// Construction or exhausted host epoch error, containing no request or credential data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Zero or unsupported processing bounds.
    #[error("invalid System One policy bounds")]
    InvalidConfig,
    /// The bounded epoch counter cannot advance; admission remains closed.
    #[error("System One worker epoch exhausted")]
    EpochExhausted,
}

/// Sanitized reason for missing trustworthy analysis. Backend error strings are never retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// Deadline expired, including the consumer's own schema deadline.
    Timeout,
    /// Engine transport is absent or restarting.
    Unavailable,
    /// The model rejected or failed the request.
    Model,
    /// Schema/version is absent from the host catalog.
    SchemaUnavailable,
    /// Caller consumer or schema failure policy does not match the host boundary.
    PolicyMismatch,
    /// Input exceeds the transport or schema bound.
    InputLimit,
    /// Engine output does not answer the exact selected schema.
    InvalidDecision,
    /// Operational failure threshold or host retirement has closed admission.
    CircuitOpen,
    /// The configured number of policy operations are already active.
    Saturated,
    /// The host retired/recovered the worker while this call was active.
    WorkerChanged,
}

/// Consumer-specific result. A failed-open outcome supplies no fabricated answer or authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyOutcome {
    /// Exact schema-validated decision and calibration-aware threshold verdicts.
    Answered {
        /// Actual engine result, never a fallback prediction.
        decision: Decision,
        /// Verdicts in host schema order. Strict uncalibrated acceptance needs approval.
        verdicts: Vec<(QuestionId, Verdict)>,
    },
    /// Caller may apply its documented deterministic advisory fallback.
    FailedOpen(FailureReason),
    /// Caller must retain deterministic protections and require review as appropriate.
    FailedClosed(FailureReason),
}

/// Minimum error policy. A schema can tighten an open row, but cannot open a closed row.
/// Strict arbitration follows the shipped schema's closed policy; it never authorizes a merge.
#[must_use]
pub const fn minimum_failure_mode(consumer: Consumer) -> FailMode {
    match consumer {
        Consumer::SafetyGate | Consumer::PiiMask | Consumer::Postflight | Consumer::DiffArbiter => {
            FailMode::Closed
        }
        Consumer::Triage
        | Consumer::ToolPrune
        | Consumer::Watchdog
        | Consumer::Sifter
        | Consumer::EffortRouter
        | Consumer::SandboxRoute
        | Consumer::EnvMatch
        | Consumer::NvidiaProfile
        | Consumer::SkillSelect => FailMode::Open,
    }
}

/// One reusable consumer policy owner over the actual decision engine and host schema catalog.
/// Atomically fenced worker epochs prevent stale completions from reopening a retired circuit.
pub struct PolicyClient<D, C> {
    engine: D,
    catalog: C,
    config: PolicyConfig,
    admission: Semaphore,
    // High 32 bits: host epoch; low 31: failures; bit 31: closed admission.
    state: AtomicU64,
}
impl<D, C> fmt::Debug for PolicyClient<D, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolicyClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
impl<D: DecisionEngine, C: SchemaCatalog> PolicyClient<D, C> {
    /// Compose without loading a model or creating fallback predictions.
    /// # Errors
    /// Rejects zero/unsupported bounds. Initial admission is open; startup health remains the
    /// host supervisor's responsibility, and can be closed immediately through its readiness report.
    pub fn new(engine: D, catalog: C, config: PolicyConfig) -> Result<Self, PolicyError> {
        if config.failure_threshold == 0
            || u64::from(config.failure_threshold) > FAILURES
            || config.max_inflight == 0
            || config.max_inflight > 4096
            || config.max_deadline.is_zero()
            || config.max_deadline > Duration::from_secs(120)
        {
            return Err(PolicyError::InvalidConfig);
        }
        Ok(Self {
            engine,
            catalog,
            config,
            admission: Semaphore::new(config.max_inflight),
            state: AtomicU64::new(1 << EPOCH_SHIFT),
        })
    }

    /// Fence the old epoch and apply an actual host-observed authenticated readiness result.
    /// This is a trusted host API, not a model tool. `true` resets failures only for a new epoch;
    /// elapsed time alone never reopens a failed worker. Old in-flight responses are discarded.
    /// # Errors
    /// Epoch exhaustion permanently closes admission instead of reusing an old epoch.
    pub fn report_worker_readiness(&self, ready: bool) -> Result<(), PolicyError> {
        let mut old = self.state.load(Ordering::Acquire);
        loop {
            let Some(next_epoch) = (old >> EPOCH_SHIFT)
                .checked_add(1)
                .filter(|value| *value <= u64::from(u32::MAX))
            else {
                self.state.fetch_or(OPEN, Ordering::AcqRel);
                return Err(PolicyError::EpochExhausted);
            };
            let Some(epoch_bits) = next_epoch.checked_shl(EPOCH_SHIFT) else {
                return Err(PolicyError::EpochExhausted);
            };
            let next = epoch_bits | if ready { 0 } else { OPEN };
            match self
                .state
                .compare_exchange_weak(old, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(()),
                Err(observed) => old = observed,
            }
        }
    }

    /// Reopen a threshold-closed circuit only after an authenticated probe proves a loaded model.
    ///
    /// A service-ready response without a loaded model is insufficient. Repeated healthy probes
    /// while admission is already open do not advance the epoch or fence in-flight calls.
    /// # Errors
    /// Epoch exhaustion permanently closes admission instead of reusing an old epoch.
    pub fn recover_if_model_ready(&self, model_ready: bool) -> Result<bool, PolicyError> {
        recover_admission(&self.state, model_ready)
    }

    /// Resolve one exact host schema and apply bounded inference, failure policy and calibration.
    /// No failure is cached and no result contains a backend error string. Canceling this future
    /// releases its admission permit; native computation is still owned by the isolated worker.
    pub async fn decide(
        &self,
        consumer: Consumer,
        id: &str,
        version: u32,
        input: &DecisionInput,
        calibration: Option<&CalibrationContext>,
    ) -> PolicyOutcome {
        let fallback = minimum_failure_mode(consumer);
        let Some(schema) = self.catalog.get(id, version) else {
            return failure(fallback, FailureReason::SchemaUnavailable);
        };
        if schema.id != id
            || schema.version != version
            || schema.fail.consumer != consumer
            || (fallback == FailMode::Closed && schema.fail.on_timeout != FailMode::Closed)
            || schema.fail.deadline_ms == 0
            || schema.max_chars == 0
            || schema.questions.is_empty()
            || schema.questions.len() > 64
        {
            return failure(FailMode::Closed, FailureReason::PolicyMismatch);
        }
        let mode = schema.fail.on_timeout;
        let context_bytes = input.context.as_ref().map_or(0, String::len);
        if input
            .text
            .len()
            .checked_add(context_bytes)
            .is_none_or(|n| n > MAX_INPUT_BYTES)
        {
            return failure(mode, FailureReason::InputLimit);
        }
        let context_chars = input
            .context
            .as_ref()
            .map_or(0, |value| value.chars().count());
        if input
            .text
            .chars()
            .count()
            .checked_add(context_chars)
            .is_none_or(|n| match u64::try_from(n) {
                Ok(value) => value > u64::from(schema.max_chars),
                Err(_) => true,
            })
        {
            return failure(mode, FailureReason::InputLimit);
        }
        let ticket = self.state.load(Ordering::Acquire);
        if ticket & OPEN != 0 {
            return failure(mode, FailureReason::CircuitOpen);
        }
        let Ok(_permit) = self.admission.try_acquire() else {
            return failure(mode, FailureReason::Saturated);
        };
        let deadline =
            Duration::from_millis(u64::from(schema.fail.deadline_ms)).min(self.config.max_deadline);
        let response =
            tokio::time::timeout(deadline, self.engine.decide(&schema.questions, input)).await;
        if !self.same_epoch(ticket) {
            return failure(mode, FailureReason::WorkerChanged);
        }
        let decision = match response {
            Ok(Ok(decision)) => decision,
            Ok(Err(DecisionError::Unavailable(_))) => {
                self.transport_failed(ticket);
                return failure(mode, FailureReason::Unavailable);
            }
            Err(_) | Ok(Err(DecisionError::Timeout)) => {
                self.transport_failed(ticket);
                return failure(mode, FailureReason::Timeout);
            }
            Ok(Err(DecisionError::InvalidSchema(_))) => {
                return failure(mode, FailureReason::InvalidDecision);
            }
            Ok(Err(DecisionError::Model(_))) => return failure(mode, FailureReason::Model),
        };
        let verdicts = match apply_threshold_with_context(schema, &decision, calibration) {
            Ok(value) => value,
            Err(_) => return failure(mode, FailureReason::InvalidDecision),
        };
        self.transport_succeeded(ticket);
        if !self.same_epoch(ticket) {
            return failure(mode, FailureReason::WorkerChanged);
        }
        PolicyOutcome::Answered { decision, verdicts }
    }

    fn same_epoch(&self, ticket: u64) -> bool {
        self.state.load(Ordering::Acquire) >> EPOCH_SHIFT == ticket >> EPOCH_SHIFT
    }
    fn transport_failed(&self, ticket: u64) {
        let mut old = self.state.load(Ordering::Acquire);
        loop {
            if old >> EPOCH_SHIFT != ticket >> EPOCH_SHIFT || old & OPEN != 0 {
                return;
            }
            let Some(failures) = (old & FAILURES).checked_add(1) else {
                return;
            };
            let threshold = u64::from(self.config.failure_threshold);
            let next = (old & !FAILURES)
                | failures.min(threshold)
                | if failures >= threshold { OPEN } else { 0 };
            match self
                .state
                .compare_exchange_weak(old, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(observed) => old = observed,
            }
        }
    }
    fn transport_succeeded(&self, ticket: u64) {
        let mut old = self.state.load(Ordering::Acquire);
        loop {
            if old >> EPOCH_SHIFT != ticket >> EPOCH_SHIFT || old & OPEN != 0 {
                return;
            }
            match self.state.compare_exchange_weak(
                old,
                old & !FAILURES,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => old = observed,
            }
        }
    }
}

fn recover_admission(state: &AtomicU64, model_ready: bool) -> Result<bool, PolicyError> {
    if !model_ready {
        return Ok(false);
    }
    let mut old = state.load(Ordering::Acquire);
    loop {
        if old & OPEN == 0 {
            return Ok(false);
        }
        let Some(next_epoch) = (old >> EPOCH_SHIFT)
            .checked_add(1)
            .filter(|value| *value <= u64::from(u32::MAX))
        else {
            state.fetch_or(OPEN, Ordering::AcqRel);
            return Err(PolicyError::EpochExhausted);
        };
        let Some(epoch_bits) = next_epoch.checked_shl(EPOCH_SHIFT) else {
            state.fetch_or(OPEN, Ordering::AcqRel);
            return Err(PolicyError::EpochExhausted);
        };
        match state.compare_exchange_weak(old, epoch_bits, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(true),
            Err(observed) => old = observed,
        }
    }
}

fn failure(mode: FailMode, reason: FailureReason) -> PolicyOutcome {
    match mode {
        FailMode::Open => PolicyOutcome::FailedOpen(reason),
        FailMode::Closed => PolicyOutcome::FailedClosed(reason),
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-system-one/src/policy.rs"]
mod recovery_tests;
