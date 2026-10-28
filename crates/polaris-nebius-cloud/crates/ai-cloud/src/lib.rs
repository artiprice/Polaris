//! Deterministic policy core for Nebius AI Cloud gateway VM lifecycle decisions.
//!
//! This crate does not create, start, stop, inspect, or delete provider resources.
//! It implements local admission, cost, ingress, lifetime, and ownership rules.

#![forbid(unsafe_code)]
#![forbid(clippy::unwrap_used)]
#![forbid(clippy::expect_used)]
#![forbid(clippy::panic)]
#![forbid(clippy::indexing_slicing)]
#![forbid(clippy::arithmetic_side_effects)]
#![forbid(clippy::as_conversions)]
#![forbid(clippy::allow_attributes_without_reason)]

mod budget;
mod lifetime;
mod spec;
mod sweep;

pub use budget::{BudgetError, BudgetGuard, BudgetSnapshot, Reservation};
pub use lifetime::{LifetimeError, LifetimeGuard, LifetimeStatus};
pub use spec::{IngressPolicy, SpecError, ValidatedVmSpec};
pub use sweep::{OwnerMarker, OwnershipError, ResourceId, ResourceLabels, select_owned_resources};
