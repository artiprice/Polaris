//! Configured model selection and real loader-backed local lifecycle management.
//!
//! The built-in catalog records the PLAN's dated snapshot; it is not a fresh availability or
//! capability check. Providers and credentials are supplied at the composition root. Shared slot
//! loads survive cancellation of one waiter, leases prevent unloading active users, and forced
//! eviction pauses new admission until explicitly resumed. Focused catalog and lifecycle contracts
//! are exercised with test-only loaders; each real loader must uphold the documented resource
//! ownership contract. This crate does not itself provision or unload the native S1 engine.

mod catalog;
mod error;
mod lifecycle;
mod manager;

pub use catalog::{
    Availability, CapabilitySupport, CascadePlan, CatalogProvenance, ModelChoice, ModelEffort,
    ModelKind, ModelRegistry, ModelSpec, OverrideReport, PriceTable, ProviderKind, ReasoningBudget,
    RegistryConfig,
};
pub use error::{LoadError, ModelsError};
pub use lifecycle::{EvictOutcome, EvictableSlot, ModelLease, ModelLoader, SlotConfig, SlotState};
pub use manager::{EvictReason, Evictable, EvictionManager, EvictionReport};
