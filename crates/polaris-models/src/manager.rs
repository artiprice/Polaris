//! Explicit eviction triggers over real loader-backed slots selected by the composition root.
use core::future::Future;
use std::time::{Duration, Instant};
use polaris_sdk::registry::MemoryPressure;
use crate::{EvictOutcome, EvictableSlot, ModelLoader, ModelsError};

/// Why the caller selected a model for eviction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictReason {
    /// Only idle slots beyond their own configured TTL are selected.
    Idle,
    /// Critical memory pressure closes admission until an explicit resume.
    MemoryPressure,
    /// A compiler or other heavy workload needs memory; caller resumes after it finishes.
    HeavyBuild,
    /// Permanent shutdown; slots may never resume admission.
    Shutdown,
}

/// Root-selected model family; enum dispatch can combine heterogeneous real slot loaders.
pub trait Evictable: Send + Sync {
    /// Time since loading or the most recent lease release, for LRU ordering.
    fn idle_for(&self) -> Result<Duration, ModelsError>;
    /// Request a real eviction. A deadline cannot justify unloading an active lease.
    fn evict(&self, reason: EvictReason, deadline: Duration)
        -> impl Future<Output = Result<EvictOutcome, ModelsError>> + Send;
    /// Resume a previously paused slot only after the caller's heavy/pressure work is over.
    fn resume_admission(&self) -> Result<(), ModelsError>;
}

impl<M: Send + Sync + 'static, L: ModelLoader<M>> Evictable for EvictableSlot<M, L> {
    fn idle_for(&self) -> Result<Duration, ModelsError> { EvictableSlot::idle_for(self) }

    async fn evict(&self, reason: EvictReason, deadline: Duration) -> Result<EvictOutcome, ModelsError> {
        match reason {
            EvictReason::Idle => match tokio::time::timeout(deadline, self.evict_if_idle()).await {
                Ok(result) => result,
                Err(_) => Ok(EvictOutcome::Pending { state: self.state()? }),
            },
            EvictReason::MemoryPressure | EvictReason::HeavyBuild => self.force_evict(deadline).await,
            EvictReason::Shutdown => self.close(deadline).await,
        }
    }

    fn resume_admission(&self) -> Result<(), ModelsError> { self.resume() }
}

/// One observed slot result, including errors rather than hiding unsuccessful unloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionReport {
    /// Stable slot label supplied by the composition root.
    pub name: String,
    /// Trigger applied.
    pub reason: EvictReason,
    /// Actual outcome, including deadline/in-use/error results.
    pub outcome: Result<EvictOutcome, ModelsError>,
}

/// Bounded collection of evictable real model slots. It owns no invented backend implementations.
#[derive(Debug)]
pub struct EvictionManager<E> { slots: Vec<(String, E)> }

impl<E: Evictable> EvictionManager<E> {
    /// Register uniquely named slots.
    ///
    /// # Errors
    /// Duplicate/invalid names or more than 256 slots.
    pub fn new(slots: Vec<(String, E)>) -> Result<Self, ModelsError> {
        if slots.len() > 256 { return Err(ModelsError::Config("too many eviction slots")); }
        let mut names = std::collections::BTreeSet::new();
        for (name, _) in &slots {
            if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) || !names.insert(name) {
                return Err(ModelsError::Config("eviction slot name"));
            }
        }
        Ok(Self { slots })
    }

    /// Evict longest-idle slots first within one overall wait budget. Once the budget is used,
    /// later slots still receive a zero-wait drain request so shutdown/admission closure is not
    /// silently skipped. Per-slot work remains owned until its leases actually release.
    pub async fn evict_all(&self, reason: EvictReason, budget: Duration) -> Vec<EvictionReport> {
        let mut order: Vec<_> = self.slots.iter().map(|(name, slot)| (name, slot, slot.idle_for())).collect();
        order.sort_by(|(an, _, age_a), (bn, _, age_b)| {
            let age = |value: &Result<Duration, ModelsError>| match value { Ok(age) => *age, Err(_) => Duration::MAX };
            age(age_b).cmp(&age(age_a)).then_with(|| an.cmp(bn))
        });
        let started = Instant::now();
        let mut reports = Vec::with_capacity(order.len());
        for (name, slot, age) in order {
            let remaining = budget.saturating_sub(started.elapsed());
            let outcome = match age {
                Ok(_) => slot.evict(reason, remaining).await,
                // Even an age observation failure must not suppress a shutdown request.
                Err(error) if reason == EvictReason::Idle => Err(error),
                Err(_) => slot.evict(reason, remaining).await,
            };
            reports.push(EvictionReport { name: name.clone(), reason, outcome });
        }
        reports
    }

    /// Apply an observed platform memory state. Elevated pressure evicts idle models; critical
    /// pressure requests draining everywhere. Normal pressure does not implicitly resume slots
    /// which may have been paused for a heavy build or explicit user action.
    pub async fn on_memory_pressure(&self, pressure: MemoryPressure, budget: Duration) -> Vec<EvictionReport> {
        match pressure {
            MemoryPressure::Normal => Vec::new(),
            MemoryPressure::Elevated => self.evict_all(EvictReason::Idle, budget).await,
            MemoryPressure::Critical => self.evict_all(EvictReason::MemoryPressure, budget).await,
        }
    }

    /// Explicitly reopen admissions after the caller confirms its pressure/heavy-build interval
    /// has ended. Failed resumes are returned individually; shutdown stays permanently closed.
    #[must_use]
    pub fn resume_all(&self) -> Vec<(String, Result<(), ModelsError>)> {
        self.slots.iter().map(|(name, slot)| (name.clone(), slot.resume_admission())).collect()
    }
}
