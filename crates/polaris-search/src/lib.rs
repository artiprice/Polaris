//! Redacting web providers and bounded, provenance-preserving retrieval orchestration.
//!
//! Inject a provider into [`sift::WebSearchSource`], then use [`sift::Sifter::race`] with named
//! concurrency pools, a validation budget and a cancellation token. Selection never turns remote
//! content into trusted instructions. [`sift::Sifter::sift`] retains dropped source chunks for
//! recovery. [`sift::Distilled::persist_dropped`] stores a bounded CAS manifest through an injected
//! SDK content store; [`sift::recover_dropped`] verifies and restores exact source chunks.
//!
//! Local semantic `/search-local` retrieval belongs to the vector/embedding adapters. This facade
//! does not start services or infer credentials, provider quotas, or acceptance thresholds.
//! ```
//! use polaris_search::{providers::Fetcher, sift::{RaceMode, SemaphoreSet}};
//! let _mode = RaceMode::BestWithinDeadline;
//! let _pools = SemaphoreSet::new(2, 2, 2)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub use polaris_search_providers as providers;
pub use polaris_search_sift as sift;
