//! Host side of the in-sandbox browser: the guest renders and writes artifacts under
//! `/sandbox`, the host downloads them and accepts only bytes that match the reported digest.

use polaris_sdk::ids::Cid;

use crate::BrowserError;
use crate::outline::{Outline, OutlineBudget, a11y_outline};
use crate::visual::{MAX_IMAGE_BYTES, VisualDiff, decode, visual_diff};

/// Where the guest wrote an artifact and the BLAKE3 digest it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestArtifact {
    /// Path under the sandbox workspace (`/sandbox/...`).
    pub path: String,
    /// Reported content digest.
    pub content: Cid,
}

/// The guest agent's browser calls (`GuestService.Screenshot` / `GuestService.A11yTree`).
pub trait GuestBrowser: Send + Sync {
    /// Render `url` at `width`×`height` and write a PNG.
    fn screenshot(
        &self,
        url: &str,
        width: u32,
        height: u32,
    ) -> impl Future<Output = Result<GuestArtifact, BrowserError>> + Send;
    /// Write the current page's accessibility tree as JSON.
    fn a11y_tree(&self) -> impl Future<Output = Result<GuestArtifact, BrowserError>> + Send;
}

/// Downloads files from the sandbox workspace (the only place the gateway can read from).
pub trait SandboxFiles: Send + Sync {
    /// Read at most `limit` bytes of `path`; more is an error.
    fn download(
        &self,
        path: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<u8>, BrowserError>> + Send;
}

/// One browser in one sandbox, driven from the host.
#[derive(Debug)]
pub struct BrowserSession<G, F> {
    guest: G,
    files: F,
    budget: OutlineBudget,
}

/// A verified screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    /// Encoded PNG bytes, matching `content`.
    pub png: Vec<u8>,
    /// Verified digest.
    pub content: Cid,
    /// Decoded width in pixels.
    pub width: u32,
    /// Decoded height in pixels.
    pub height: u32,
}

impl<G: GuestBrowser, F: SandboxFiles> BrowserSession<G, F> {
    /// A session over `guest` and its sandbox files; outlines use `budget`.
    #[must_use]
    pub const fn new(guest: G, files: F, budget: OutlineBudget) -> Self {
        Self {
            guest,
            files,
            budget,
        }
    }

    /// Render `url` and return the verified PNG.
    /// # Errors
    /// Invalid size, guest or download failure, a digest mismatch, or an undecodable image.
    pub async fn screenshot(
        &self,
        url: &str,
        width: u32,
        height: u32,
    ) -> Result<Screenshot, BrowserError> {
        if !(url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("file://"))
        {
            return Err(BrowserError::Rejected(
                "only http, https and file URLs are rendered",
            ));
        }
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(BrowserError::Rejected(
                "screenshot size must be 1..=4096 pixels a side",
            ));
        }
        let artifact = self.guest.screenshot(url, width, height).await?;
        let png = self.fetch(&artifact, MAX_IMAGE_BYTES).await?;
        let image = decode(&png)?;
        Ok(Screenshot {
            content: artifact.content,
            width: image.width(),
            height: image.height(),
            png,
        })
    }

    /// The current page as a compact outline with action `ref`s.
    /// # Errors
    /// Guest or download failure, a digest mismatch, or a malformed tree.
    pub async fn outline(&self) -> Result<Outline, BrowserError> {
        let artifact = self.guest.a11y_tree().await?;
        let tree = self
            .fetch(&artifact, crate::outline::MAX_TREE_BYTES)
            .await?;
        a11y_outline(&tree, self.budget)
    }

    /// Render `url` and compare it with a baseline PNG.
    /// # Errors
    /// As [`Self::screenshot`], or an undecodable baseline.
    pub async fn compare_with(
        &self,
        url: &str,
        baseline: &[u8],
    ) -> Result<VisualDiff, BrowserError> {
        let base = decode(baseline)?;
        let current = self.screenshot(url, base.width(), base.height()).await?;
        visual_diff(baseline, &current.png)
    }

    async fn fetch(&self, artifact: &GuestArtifact, limit: usize) -> Result<Vec<u8>, BrowserError> {
        if !artifact.path.starts_with("/sandbox/") || artifact.path.contains("..") {
            return Err(BrowserError::Rejected(
                "artifact is outside the sandbox workspace",
            ));
        }
        let bytes = self.files.download(&artifact.path, limit).await?;
        if bytes.len() > limit {
            return Err(BrowserError::TooLarge {
                what: "artifact",
                limit,
            });
        }
        if Cid::from_bytes(*blake3::hash(&bytes).as_bytes()) != artifact.content {
            return Err(BrowserError::DigestMismatch);
        }
        Ok(bytes)
    }
}
