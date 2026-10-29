//! Frontend-neutral offline session host (descriptor: specs/descriptors/ui_session_host.json).
//!
//! Extracted from the terminal client so every frontend (terminal, Slint desktop) drives the same
//! real local kernel, registry and bounded CAS. A submitted line is either applied as one signed
//! session directive or recorded verbatim as typed user input
//! ([`polaris_sdk::contracts::Event::UserInputSubmitted`]); [`chat`] then answers typed input
//! with an agent frame on the same kernel when a model provider is available; every file change
//! it proposes waits for the person's review ([`answer_review`]).
//!
//! Messages are not indexed for history search yet: [`search_history`] returns whatever the
//! registry holds, and no production code currently feeds the registry's message index.
#![forbid(unsafe_code)]

pub mod chat;
pub mod ci;
pub mod commands;
mod error;
pub mod hack;
pub mod host;
pub mod nvidia;
mod offline;
mod queries;
mod remote;
pub mod stats;
mod walk;

pub use chat::{
    ChangeReview, ChatAgent, ChatError, ChatProgress, ChatSettings, ChatTurn, answer_review,
    choose_provider,
};
pub use error::SessionHostError;
pub use host::{
    ConversationHost, HostEvent, TurnReport, default_env_file, records_until_own_input,
};
pub use offline::{OfflineSession, RecordTextKind, ResolvedRecord, is_single_directive};
pub use polaris_kernel::storage::registry::{Hit, SessionMeta};
pub use queries::{list_sessions, search_history, session_exists};
pub use remote::{AttachTarget, RemoteSession};
