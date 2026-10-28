//! # polaris-sdk
//!
//! Layer L0: the only crate every other Polaris crate may depend on. It owns the vocabulary and the
//! contracts, and contains no I/O and no third-party adapters.
//!
//! | Module | Contents |
//! |---|---|
//! | [`ids`] | `SessionId`, `FrameId`, `CommandId`, `Seq`, `Cid`, `ShortRef` (`§ref:<12hex>`), `FastFingerprint`, `ModelId`, `MicroUsd`, `TokenCount`, `AgentRole` |
//! | [`effort`] | `Effort` (`low`…`max` or `1`–`5`) |
//! | [`error`] | `ErrorKind` + `HasKind` (supervision taxonomy), `ParseError` |
//! | [`system_one`] | Choice / Score / Noul questions and answers, `Probability`, `Decision`, validated `DecisionSchema` snapshots and `DecisionSchemaCatalog` |
//! | [`contracts`] | `TokenBytes`, `Command<Unverified \| Verified>` typestate, `ApprovalWitness`, `Event` v5, `EventRecord` |
//! | [`registry`] | API-registry traits (prompt.md §6.2): `ChatModel`, `PromptTokenizer`, `EmbeddingProvider`, `DecisionEngine`, `SandboxBackend`, `WebSearch`, `DocParser`, `VectorStore`, `EventStore`, `KernelApi`, `PlatformOps`, `Redactor`, `FunctionalOperator`, `ExactCacheStore`, `Tool` |
//! | [`human`] | Bounded human question and atomic answer contracts; event schema v5 |
//! | [`secrets`] | Validated secret locations and the credential-source registry contract |
//! | [`backends`] | `Backends` bundle (composition root, PLAN ADR-002) and `OneOf2` / `OneOf3` enum dispatch |
//!
//! Design rules: newtypes validate on construction and on deserialization; secret-bearing values
//! never print their content; events are fixed-size (`Copy`) with text offloaded to CAS by
//! [`ids::Cid`]; async trait methods return `impl Future + Send` (no `dyn`).
//!
//! Not in this crate yet (added when their owning phase starts): `Sealer` (P4 secrets),
//! `GatewayEndpoint` (P2 OpenShell), `FrameView` /
//! `ScratchpadRef` / `ContextRecall` (P3–P4 runtime).

pub mod backends;
pub mod content;
pub mod contracts;
pub mod effort;
pub mod error;
mod hex;
pub mod human;
pub mod ids;
pub mod interactive;
#[cfg(feature = "kani")]
mod kani_proofs;
pub mod registry;
pub mod secrets;
pub mod system_one;
