//! Bounded file and AST-scope leases with optimistic concurrency control.
//!
//! The resolver at the workspace boundary must establish that an input path is inside its
//! workspace before constructing [`LeaseKey`]. This crate performs conservative lexical Windows
//! alias checks; [`LeaseKey::from_path`] resolves existing filesystem identities and contained
//! parents for missing final filenames. Containment must be checked again at the write boundary.
//!
//! Lease acquisition is FIFO and multi-key acquisition is atomic. Dropping a guard releases its
//! leases synchronously. Expired records are removed when the manager is used or when
//! [`LeaseManager::reap_expired`] is called.

#[cfg(feature = "kani")]
mod kani_proofs;
mod key;
mod manager;
mod scope;

pub use key::LeaseKey;
pub use manager::{
    ChurnVerdict, Invalidation, LeaseConfig, LeaseError, LeaseGuard, LeaseId, LeaseManager,
    LeaseRequest, RevokeReason, WriteTicket,
};
pub use scope::{LeaseScope, SymbolPath, scopes_conflict};
