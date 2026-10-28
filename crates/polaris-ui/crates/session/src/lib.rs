//! Frontend-neutral offline session host (descriptor: specs/descriptors/ui_session_host.json).
//!
//! Extracted from the terminal client so every frontend (terminal, Slint desktop) drives the same
//! real local kernel, registry and bounded CAS. Nothing here generates model replies: a submitted
//! line is either applied as one signed session directive or recorded verbatim as typed user
//! input ([`polaris_sdk::contracts::Event::UserInputSubmitted`]).
//!
//! Messages are not indexed for history search yet: [`search_history`] returns whatever the
//! registry holds, and no production code currently feeds the registry's message index.
#![forbid(unsafe_code)]

mod error;
mod offline;
mod queries;

pub use error::SessionHostError;
pub use offline::{OfflineSession, RecordTextKind, ResolvedRecord};
pub use polaris_kernel::storage::registry::{Hit, SessionMeta};
pub use queries::{list_sessions, search_history, session_exists};
