//! # polaris-code-intel
//!
//! Code understanding over bytes (L1, P1): tree-sitter parsing for Rust, Python, C++, CUDA and
//! TypeScript, signature-only skeletons, a PageRank repo map, the mutation metrics consumed by
//! the kernel verifier and a symbol-granular three-way merge. The crate does no file discovery
//! and no I/O: callers hand in `&[u8]`, which keeps every function deterministic and testable.
//!
//! | Module | Contents |
//! |---|---|
//! | [`Lang`], [`detect_lang`] | language enum, extension / shebang detection |
//! | [`ParserPool`], [`Parsed`], [`ParseLimits`], [`ByteSpan`] | parsing with size and time limits, incremental reparse |
//! | [`skeleton`], [`Skeleton`] | signatures without bodies |
//! | [`SymbolGraph`], [`rank_files`], [`repo_map`] | personalised PageRank repo map under a token budget |
//! | [`metrics`], [`subtree_fingerprints`] | churn, cyclomatic delta, tree-edit distance, changed symbols, xxh3 fingerprints |
//! | [`merge3`] | symbol-granular three-way merge that never writes on conflict |
//!
//! ## Safety properties
//!
//! - Every walk over a syntax tree is iterative (explicit stacks or tree-sitter cursors), except
//!   the merge, which recurses only through nested declarations and stops at a fixed depth.
//! - Spans are byte offsets into the original text: EOLs are never rewritten and a UTF-8 BOM is
//!   kept in place.
//! - tree-sitter and the grammars are C code and a segfault cannot be caught: size and time
//!   limits ([`ParseLimits`]) reduce, but do not remove, the risk of hostile input.
//! - xxh3 fingerprints are dirty-check aids and are never used as a security identity.
//!
//! Test-first descriptors: `specs/descriptors/codeintel_*.json`.

mod decl;
mod error;
mod graph;
mod lang;
mod merge;
mod metrics;
mod parse;
mod repomap;
mod safety;
mod skeleton;
mod walk;

#[cfg(feature = "kani")]
mod kani_proofs;

pub use decl::{SymbolKind, SymbolRef};
pub use error::CodeIntelError;
pub use graph::SymbolGraph;
pub use lang::{Lang, detect_lang};
pub use merge::{MergeOutcome, merge3};
pub use metrics::{
    MetricsConfig, MutationMetrics, cyclomatic_complexity, metrics, metrics_with,
    subtree_fingerprints,
};
pub use parse::{ByteSpan, ParseLimits, Parsed, ParserPool};
pub use repomap::{RankedFile, RepoMap, RepoMapBudget, RepoMapVersion, rank_files, repo_map};
pub use safety::{
    AnalysisCoverage, AssertionObservation, AstObservation, MAX_SAFETY_NODES, MarkerKind,
    SafetyAnalysis, SafetyCounts, TautologyKind, TautologyObservation, TestMarker, TestObservation,
    analyze_safety,
};
pub use skeleton::{MAX_SIGNATURE_BYTES, MAX_SKELETON_ITEMS, Skeleton, SkeletonItem, skeleton};
