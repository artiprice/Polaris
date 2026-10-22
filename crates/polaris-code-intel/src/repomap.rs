//! The repo map: files ranked by personalised PageRank, rendered under a token budget.

use crate::error::CodeIntelError;
use crate::graph::{SymbolGraph, normalize_path};
use polaris_sdk::ids::{FastFingerprint, TokenCount};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

/// PageRank damping factor (Aider uses the same value).
const DAMPING: f64 = 0.85;
/// Hard cap on power iterations; the loop normally converges long before.
const MAX_ITERATIONS: usize = 100;
/// L1 change of the rank vector below which iteration stops.
const TOLERANCE: f64 = 1e-9;
/// Bytes per token in the approximate token estimate (the real tokenizer is not available).
const BYTES_PER_TOKEN: usize = 4;
/// Bumped whenever the rendered map format changes, so cached prompt prefixes are invalidated.
const FORMAT_VERSION: u32 = 1;

// PageRank is approximate floating-point arithmetic. Build counts without an unchecked numeric
// cast; all byte steps are exact until the count exceeds f64's integral precision.
fn count_as_f64(value: usize) -> f64 {
    value
        .to_le_bytes()
        .iter()
        .rev()
        .fold(0.0, |count, byte| count * 256.0 + f64::from(*byte))
}

/// Converts a token budget to its approximate byte ceiling without overflowing.
pub(crate) fn max_render_bytes(max_tokens: u32) -> u64 {
    u64::from(max_tokens).saturating_mul(4)
}

/// Converts rendered bytes to the documented ceiling estimate, if it fits the public count type.
pub(crate) fn estimated_tokens(byte_len: usize) -> Option<u32> {
    u32::try_from(byte_len.div_ceil(BYTES_PER_TOKEN)).ok()
}

/// What the map is rendered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMapBudget {
    /// Largest map in estimated tokens (`ceil(bytes / 4)`, an approximation).
    pub max_tokens: TokenCount,
    /// Files being worked on; ranking is personalised towards them. Unknown paths are ignored.
    pub focus_files: Vec<PathBuf>,
}

/// One file of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedFile {
    /// The file, with `/` separators.
    pub path: PathBuf,
    /// PageRank score; all scores of a graph sum to 1.
    pub rank: f64,
    /// Skeleton lines of the file that fit the budget.
    pub outline: Vec<String>,
}

/// Identifies one rendering of the map, for cache invalidation of prompt prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepoMapVersion {
    /// Layout version of the rendered text.
    pub format: u32,
    /// xxh3 digest of the rendered text (a change detector, not a security identity).
    pub digest: FastFingerprint,
}

/// A rendered, ranked overview of a repository.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoMap {
    /// Files by descending rank (ties by path); files without declarations are omitted.
    pub entries: Vec<RankedFile>,
    /// Approximate size of [`RepoMap::render`] in tokens (`ceil(bytes / 4)`).
    pub token_estimate: TokenCount,
    /// Version of this rendering.
    pub version: RepoMapVersion,
}

impl RepoMap {
    /// The map as prompt text: `path:` followed by the outline lines indented by two spaces.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&header_line(&entry.path));
            for line in &entry.outline {
                out.push_str(&outline_line(line));
            }
        }
        out
    }
}

fn header_line(path: &std::path::Path) -> String {
    format!("{}:\n", normalize_path(path))
}

fn outline_line(line: &str) -> String {
    format!("  {line}\n")
}

/// Ranks every file of `g` with personalised PageRank (damping 0.85, at most 100 iterations).
///
/// A reference from file A to a name defined in file B is an edge A to B weighted by the square
/// root of the use count and split evenly between all definers of the name; references within
/// one file are ignored. Teleportation (and the mass of files without outgoing edges) goes to the
/// `focus` files, or uniformly when none of them is in the graph. Scores are non-negative and sum
/// to 1. The result is sorted by descending score, ties by path, and is bit-identical for
/// identical graphs regardless of insertion order.
#[must_use]
pub fn rank_files(g: &SymbolGraph, focus: &[PathBuf]) -> Vec<(PathBuf, f64)> {
    let files = g.files();
    let names: Vec<&String> = files.keys().collect();
    let n = names.len();
    if n == 0 {
        return Vec::new();
    }
    let index: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    let mut definers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, symbols) in files.values().enumerate() {
        for def in &symbols.defs {
            definers.entry(def.as_str()).or_default().push(i);
        }
    }
    // edges[i]: (target, weight) sorted by target; out_weight[i]: sum of the weights.
    let mut edges: Vec<Vec<(usize, f64)>> = Vec::with_capacity(n);
    for (i, symbols) in files.values().enumerate() {
        let mut acc: BTreeMap<usize, f64> = BTreeMap::new();
        for (name, count) in &symbols.refs {
            let Some(targets) = definers.get(name.as_str()) else {
                continue;
            };
            let others: Vec<usize> = targets.iter().copied().filter(|&t| t != i).collect();
            if others.is_empty() {
                continue;
            }
            let share = f64::from(*count).sqrt() / count_as_f64(others.len());
            for t in others {
                *acc.entry(t).or_insert(0.0) += share;
            }
        }
        edges.push(acc.into_iter().collect());
    }
    let out_weight: Vec<f64> = edges
        .iter()
        .map(|e| e.iter().map(|(_, w)| *w).sum())
        .collect();

    let focus_ids: BTreeSet<usize> = focus
        .iter()
        .filter_map(|f| index.get(normalize_path(f).as_str()).copied())
        .collect();
    let personalization: Vec<f64> = if focus_ids.is_empty() {
        vec![1.0 / count_as_f64(n); n]
    } else {
        let share = 1.0 / count_as_f64(focus_ids.len());
        (0..n)
            .map(|i| if focus_ids.contains(&i) { share } else { 0.0 })
            .collect()
    };

    let mut rank = personalization.clone();
    for _ in 0..MAX_ITERATIONS {
        let mut next = vec![0.0f64; n];
        let mut dangling = 0.0f64;
        for (i, r) in rank.iter().enumerate() {
            let total = out_weight.get(i).copied().unwrap_or(0.0);
            if total > 0.0 {
                for &(t, w) in edges.get(i).map(Vec::as_slice).unwrap_or_default() {
                    if let Some(slot) = next.get_mut(t) {
                        *slot += DAMPING * r * w / total;
                    }
                }
            } else {
                dangling += r;
            }
        }
        let teleport = (1.0 - DAMPING) + DAMPING * dangling;
        for (slot, p) in next.iter_mut().zip(&personalization) {
            *slot += teleport * p;
        }
        let delta: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if delta < TOLERANCE {
            break;
        }
    }
    let total: f64 = rank.iter().sum();
    if total > 0.0 && total.is_finite() {
        for r in &mut rank {
            *r /= total;
        }
    }
    let mut ranked: Vec<(PathBuf, f64)> = names
        .iter()
        .zip(&rank)
        .map(|(name, r)| (PathBuf::from(name.as_str()), *r))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

/// Builds the repo map of `g` that fits `b.max_tokens`.
///
/// Files are taken in rank order. The last file that does not fit completely is cut to as many
/// outline lines as fit (if at least one does) and the map ends there, so the estimate never
/// exceeds the budget. Identical graphs and budgets give byte-identical maps.
///
/// # Errors
/// [`CodeIntelError::TooLarge`] if the map would exceed `u32::MAX` estimated tokens.
pub fn repo_map(g: &SymbolGraph, b: &RepoMapBudget) -> Result<RepoMap, CodeIntelError> {
    let max_bytes = usize::try_from(max_render_bytes(b.max_tokens.get())).unwrap_or(usize::MAX);
    let mut entries: Vec<RankedFile> = Vec::new();
    let mut used = 0usize;
    'files: for (path, rank) in rank_files(g, &b.focus_files) {
        let Some(symbols) = g.files().get(normalize_path(&path).as_str()) else {
            continue;
        };
        if symbols.outline.is_empty() {
            continue;
        }
        let header = header_line(&path).len();
        if used.saturating_add(header) > max_bytes {
            break;
        }
        let mut file_used = used + header;
        let mut outline: Vec<String> = Vec::new();
        for line in &symbols.outline {
            let len = outline_line(line).len();
            if file_used.saturating_add(len) > max_bytes {
                if !outline.is_empty() {
                    entries.push(RankedFile {
                        path,
                        rank,
                        outline,
                    });
                }
                break 'files;
            }
            file_used += len;
            outline.push(line.clone());
        }
        used = file_used;
        entries.push(RankedFile {
            path,
            rank,
            outline,
        });
    }
    let mut map = RepoMap {
        entries,
        token_estimate: TokenCount::new(0),
        version: RepoMapVersion {
            format: FORMAT_VERSION,
            digest: FastFingerprint::new(0),
        },
    };
    let text = map.render();
    map.token_estimate = TokenCount::new(estimated_tokens(text.len()).ok_or(
        CodeIntelError::TooLarge {
            limit: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
        },
    )?);
    map.version.digest = FastFingerprint::new(xxh3_64(text.as_bytes()));
    Ok(map)
}
