//! # polaris-browser
//!
//! Host-side contract to the in-sandbox browser: navigation/click/type tools, a11y-tree
//! compression, pHash visual diff (`/ui-diff`).
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L2 — adapters & infrastructure |
//! | Kind | lib |
//! | Phase | P7 |
//! | Parent facade | — |
//! | Status | outline, visual diff and verified artifact session implemented; the guest-side Chromium driver is not |
//!
//! ## What is here
//!
//! - [`a11y_outline`]: an accessibility tree (Chromium/Playwright snapshot JSON) becomes an
//!   indented outline of a few hundred tokens, folding unnamed structural nodes and giving every
//!   interactive node a `ref` (`e1`, `e2`, …) that click/type tools can target.
//! - [`visual_diff`]: a 64-bit DCT perceptual hash per screenshot (similar at Hamming distance
//!   ≤ [`visual::SIMILAR_DISTANCE`]) plus an 8×8 grid of changed regions for same-sized images.
//! - [`BrowserSession`]: drives a [`GuestBrowser`] (the guest agent's `Screenshot` and `A11yTree`
//!   calls) and downloads its artifacts through [`SandboxFiles`], accepting only bytes under
//!   `/sandbox/` whose BLAKE3 digest matches what the guest reported.
//!
//! ## Not here yet
//!
//! Driving Chromium inside the microVM (navigation, click, type) belongs to the guest agent;
//! the host tools that call it arrive with that agent.
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-proto`

pub mod outline;
pub mod session;
pub mod visual;

pub use outline::{A11yNode, ActionTarget, Outline, OutlineBudget, a11y_outline, outline};
pub use session::{BrowserSession, GuestArtifact, GuestBrowser, SandboxFiles, Screenshot};
pub use visual::{PerceptualHash, VisualDiff, compare, perceptual_hash, visual_diff};

/// Browser contract failures; messages carry no page content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    /// An input exceeded its byte bound.
    #[error("{what} exceeds {limit} bytes")]
    TooLarge {
        /// What was too large.
        what: &'static str,
        /// The bound in bytes.
        limit: usize,
    },
    /// An input could not be parsed or decoded.
    #[error("malformed {0}")]
    Malformed(&'static str),
    /// A request or artifact violates the browser policy.
    #[error("rejected: {0}")]
    Rejected(&'static str),
    /// Downloaded bytes do not match the digest the guest reported.
    #[error("artifact digest does not match the guest's report")]
    DigestMismatch,
    /// The guest or the download failed.
    #[error("guest browser unavailable: {0}")]
    Guest(String),
}
