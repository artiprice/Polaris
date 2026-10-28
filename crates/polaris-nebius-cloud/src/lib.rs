//! Nebius adapter facade with explicit implementation boundaries.
//!
//! [`token_factory`] supplies the bounded OpenAI-compatible transport; [`data_lab`] supplies
//! local consent, masking, and export. [`ai_cloud`] supplies local budget, ingress, lifetime,
//! and ownership policy. [`sandboxes`] supplies a typed stateless Contree MCP adapter. AI Cloud
//! provisioning, supervised Contree deployment and live remote-isolation acceptance remain open.
//!
//! ```
//! use polaris_nebius_cloud::ai_cloud::BudgetGuard;
//! let budget = BudgetGuard::new(10);
//! let reservation = budget.reserve(10, std::time::Duration::from_secs(3600))?;
//! assert!(budget.reserve(1, std::time::Duration::from_secs(3600)).is_err());
//! reservation.settle(10)?;
//! # Ok::<(), polaris_nebius_cloud::ai_cloud::BudgetError>(())
//! ```

pub use polaris_nebius_cloud_ai_cloud as ai_cloud;
pub use polaris_nebius_cloud_data_lab as data_lab;
pub use polaris_nebius_cloud_sandboxes as sandboxes;
pub use polaris_nebius_cloud_token_factory as token_factory;
