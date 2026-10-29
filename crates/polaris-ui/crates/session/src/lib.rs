//! Frontend-neutral offline session host (descriptor: specs/descriptors/ui_session_host.json).
//!
//! Extracted from the terminal client so every frontend (terminal, Slint desktop) drives the same
//! real local kernel, registry and bounded CAS. A submitted line is either applied as one signed
//! session directive or recorded verbatim as typed user input
//! ([`polaris_sdk::contracts::Event::UserInputSubmitted`]); [`chat`] then answers typed input
//! with a read-only agent frame on the same kernel when a model provider is available.
//!
//! Messages are not indexed for history search yet: [`search_history`] returns whatever the
//! registry holds, and no production code currently feeds the registry's message index.
#![forbid(unsafe_code)]

pub mod chat;
mod error;
pub mod host;
pub mod stats;
mod offline;
mod queries;

pub use chat::{ChatAgent, ChatError, ChatProgress, ChatSettings, ChatTurn, choose_provider};
pub use error::SessionHostError;
pub use host::{
    ConversationHost, HostEvent, TurnReport, default_env_file, records_until_own_input,
};
pub use offline::{OfflineSession, RecordTextKind, ResolvedRecord, is_single_directive};
pub use polaris_kernel::storage::registry::{Hit, SessionMeta};
pub use queries::{list_sessions, search_history, session_exists};
