//! Bounded file and AST-scope leases with optimistic concurrency control.
//!
//! The resolver at the workspace boundary must establish that an input path is inside its
//! workspace before constructing [`LeaseKey`]. This crate performs conservative lexical Windows
//! alias checks; it does not resolve symlinks, junctions, or filesystem identities.
//!
//! Lease acquisition is FIFO and multi-key acquisition is atomic. Dropping a guard releases its
//! leases synchronously. Expired records are removed when the manager is used or when
//! [`LeaseManager::reap_expired`] is called.

mod key;
mod manager;
mod scope;

pub use key::LeaseKey;
pub use manager::{
    ChurnVerdict, Invalidation, LeaseConfig, LeaseError, LeaseGuard, LeaseId, LeaseManager,
    LeaseRequest, RevokeReason, WriteTicket,
};
pub use scope::{LeaseScope, SymbolPath, scopes_conflict};
