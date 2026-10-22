//! Checked resource quotas with reservation and RAII return to the parent's available pool.
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
};

/// Network access domain. Allow-list narrowing uses exact host identity; no glob semantics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NetworkPolicy {
    /// No outbound network access.
    #[default]
    Deny,
    /// Only these exact host names are permitted.
    Allowlist(BTreeSet<String>),
    /// Any network destination. A child may select this only when its parent does too.
    Any,
}

impl NetworkPolicy {
    /// Return whether `child` is provably contained in this policy.
    #[must_use]
    pub fn contains_policy(&self, child: &Self) -> bool {
        match (self, child) {
            (Self::Any, _) => true,
            (Self::Deny, Self::Deny) => true,
            (Self::Allowlist(_), Self::Deny) => true,
            (Self::Allowlist(parent), Self::Allowlist(child)) => child.is_subset(parent),
            _ => false,
        }
    }
}

/// Per-frame resource ceilings. Numeric values are reserved from the parent while the child lives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceQuota {
    /// CPU allowance in milliseconds.
    pub cpu_millis: u32,
    /// Memory allowance in MiB.
    pub memory_mb: u32,
    /// Network destinations permitted to this frame.
    pub network: NetworkPolicy,
    /// Maximum model/tool steps for this frame.
    pub max_steps: u32,
    /// Maximum nested child depth, including this frame.
    pub max_depth: u8,
    /// Model token allowance.
    pub token_budget: u64,
}

/// Quota carving failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    /// A requested resource exceeds remaining parent budget.
    #[error("child quota exceeds parent remaining budget: {0}")]
    Exceeded(&'static str),
    /// One or more quota arithmetic operations overflowed.
    #[error("quota accounting invariant failed")]
    Accounting,
    /// A child requested a broader network policy.
    #[error("child network policy exceeds parent policy")]
    Network,
}

impl ResourceQuota {
    /// Prove child non-numeric policies are monotone.
    pub fn check_subset(&self, child: &Self) -> Result<(), QuotaError> {
        if !self.network.contains_policy(&child.network) {
            return Err(QuotaError::Network);
        }
        if child.max_depth > self.max_depth {
            return Err(QuotaError::Exceeded("max_depth"));
        }
        Ok(())
    }

    pub(crate) fn reserve(&mut self, child: &Self) -> Result<QuotaLease, QuotaError> {
        self.check_subset(child)?;
        if child.cpu_millis > self.cpu_millis {
            return Err(QuotaError::Exceeded("cpu_millis"));
        }
        if child.memory_mb > self.memory_mb {
            return Err(QuotaError::Exceeded("memory_mb"));
        }
        if child.max_steps > self.max_steps {
            return Err(QuotaError::Exceeded("max_steps"));
        }
        if child.token_budget > self.token_budget {
            return Err(QuotaError::Exceeded("token_budget"));
        }
        self.cpu_millis = self
            .cpu_millis
            .checked_sub(child.cpu_millis)
            .ok_or(QuotaError::Accounting)?;
        self.memory_mb = self
            .memory_mb
            .checked_sub(child.memory_mb)
            .ok_or(QuotaError::Accounting)?;
        self.max_steps = self
            .max_steps
            .checked_sub(child.max_steps)
            .ok_or(QuotaError::Accounting)?;
        self.token_budget = self
            .token_budget
            .checked_sub(child.token_budget)
            .ok_or(QuotaError::Accounting)?;
        Ok(QuotaLease {})
    }

    fn return_allocation(&mut self, allocation: &Self) -> Result<(), QuotaError> {
        self.cpu_millis = self
            .cpu_millis
            .checked_add(allocation.cpu_millis)
            .ok_or(QuotaError::Accounting)?;
        self.memory_mb = self
            .memory_mb
            .checked_add(allocation.memory_mb)
            .ok_or(QuotaError::Accounting)?;
        self.max_steps = self
            .max_steps
            .checked_add(allocation.max_steps)
            .ok_or(QuotaError::Accounting)?;
        self.token_budget = self
            .token_budget
            .checked_add(allocation.token_budget)
            .ok_or(QuotaError::Accounting)?;
        Ok(())
    }
}

/// A reservation that can be attached to a child frame's live remaining-budget ledger.
pub struct QuotaLease {}
impl QuotaLease {
    pub(crate) fn attach(
        self,
        parent: Arc<Mutex<ResourceQuota>>,
        child_remaining: Arc<Mutex<ResourceQuota>>,
    ) -> AttachedQuotaLease {
        AttachedQuotaLease {
            child_remaining,
            parent,
        }
    }
}
impl std::fmt::Debug for QuotaLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuotaLease").finish()
    }
}

/// Child lease returns only unused quota. Spent steps and reserved model tokens stay charged.
pub(crate) struct AttachedQuotaLease {
    child_remaining: Arc<Mutex<ResourceQuota>>,
    parent: Arc<Mutex<ResourceQuota>>,
}
impl Drop for AttachedQuotaLease {
    fn drop(&mut self) {
        let unused = lock_recover(&self.child_remaining).clone();
        let mut parent = lock_recover(&self.parent);
        let mut candidate = parent.clone();
        if candidate.return_allocation(&unused).is_err() {
            parent.max_steps = 0;
            parent.token_budget = 0;
            parent.cpu_millis = 0;
            parent.memory_mb = 0;
        } else {
            *parent = candidate;
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
