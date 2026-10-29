//! # polaris-nvidia-docs
//!
//! NVIDIA knowledge engine: docs MCP / llms.txt + .md / HTML ingestion, FTS+vector index,
//! $nvidia(auto) profiles, /nv-profile and /nv-verify advisor packs.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L3 — domain |
//! | Kind | lib |
//! | Phase | P5 |
//! | Parent facade | — |
//! | Status | profiles, CUDA advisor, ingestion, conditional fetching and BM25 index implemented; vector search is not |
//!
//! ## What is here
//!
//! - [`auto_profile`] (`$nvidia(auto)`, `/nv-profile`): the documentation topics and target GPU
//!   architectures a workspace and prompt involve, with the evidence for each, and
//!   [`parse_nvidia_smi`] plus [`NvidiaProfile::hardware_notes`] for "the build targets sm_90,
//!   this machine has sm_75".
//! - [`advise_cuda`] (`/nv-verify`): CUDA rules for unchecked launches and API calls, missing
//!   grid bounds, barriers in divergent branches, strided global access and shared-memory bank
//!   conflicts.
//! - Ingestion: [`parse_llms_txt`], [`html_to_markdown`], [`chunk_markdown`], and
//!   [`NvidiaKnowledge`] — a BM25 index queried per topic, loaded from and saved to the
//!   JSON-lines offline seed.
//! - [`DocFetcher`]: incremental refresh over HTTPS from NVIDIA documentation hosts only
//!   ([`DOC_HOSTS`]), with `If-None-Match` / `If-Modified-Since`, a bounded body and a pause
//!   between requests to one host.
//!
//! ## Not here yet
//!
//! Vector search over the same chunks, and the xtask that builds the offline seed (which must
//! review each documentation set's licence before shipping it).
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-storage-registry`, `polaris-storage-vector`, `polaris-code-intel`, `polaris-ratelimit`

pub mod advisor;
pub mod docs;
pub mod fetch;
pub mod profile;

pub use advisor::{AdvisorFinding, Severity, advise_cuda};
pub use fetch::{DOC_HOSTS, DocFetcher, FetchOutcome, Validators};
pub use docs::{
    DocChunk, DocHit, DocIndex, DocLink, LlmsTxt, chunk_markdown, html_to_markdown, parse_llms_txt,
};
pub use profile::{
    ComputeCapability, DocTopic, Gpu, NvidiaProfile, arch_mentions, auto_profile, parse_nvidia_smi,
};

/// NVIDIA knowledge failures; messages carry no document text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NvidiaError {
    /// The workspace could not be listed.
    #[error("workspace is unavailable")]
    Workspace,
    /// An input exceeded its bound.
    #[error("{0} is too large")]
    TooLarge(&'static str),
    /// An input could not be parsed.
    #[error("malformed {0}")]
    Malformed(&'static str),
    /// The offline seed could not be read or written.
    #[error("documentation seed is unreadable or malformed")]
    Seed,
    /// A page could not be fetched (disallowed host, network or HTTP failure).
    #[error("documentation fetch failed: {0}")]
    Fetch(String),
}

/// The documentation index with the profile that scopes it.
#[derive(Debug, Default)]
pub struct NvidiaKnowledge {
    index: DocIndex,
}

impl NvidiaKnowledge {
    /// Knowledge over an existing index (for example one read from the offline seed).
    #[must_use]
    pub const fn new(index: DocIndex) -> Self {
        Self { index }
    }

    /// The underlying index.
    #[must_use]
    pub const fn index(&self) -> &DocIndex {
        &self.index
    }

    /// Ranked chunks about `question`, within `topic` when given.
    #[must_use]
    pub fn query(&self, topic: Option<DocTopic>, question: &str, limit: usize) -> Vec<DocHit> {
        self.index.query(topic, question, limit)
    }

    /// Ranked chunks for `question` across the topics `profile` found, best first; every topic
    /// when the profile found none.
    #[must_use]
    pub fn query_profile(
        &self,
        profile: &NvidiaProfile,
        question: &str,
        limit: usize,
    ) -> Vec<DocHit> {
        if profile.topics.is_empty() {
            return self.index.query(None, question, limit);
        }
        let mut hits: Vec<DocHit> = profile
            .topics
            .iter()
            .flat_map(|topic| self.index.query(Some(*topic), question, limit))
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        hits
    }
}
