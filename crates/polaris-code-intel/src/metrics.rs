//! Mutation metrics for the verifier: churn, cyclomatic delta, tree-edit distance, changed
//! symbols, plus xxh3 subtree fingerprints.

use crate::decl::{DeclInfo, SymbolKind, SymbolRef, declarations};
use crate::error::CodeIntelError;
use crate::lang::Lang;
use crate::parse::{ByteSpan, Parsed};
use crate::skeleton::MAX_SKELETON_ITEMS;
use crate::walk::{Descend, node_text, walk_preorder};
use imara_diff::{Algorithm, Diff, InternedInput};
use polaris_sdk::ids::FastFingerprint;
use std::collections::BTreeMap;
use tree_sitter::Node;
use xxhash_rust::xxh3::{Xxh3, xxh3_64, xxh3_64_with_seed};

/// Default [`MetricsConfig::ted_node_cap`].
const DEFAULT_TED_NODE_CAP: usize = 2_000;
/// Above this many lines (both files together) churn uses the faster, non-minimal Histogram diff.
const MINIMAL_DIFF_LINE_CAP: usize = 50_000;

/// Tuning of [`metrics_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsConfig {
    /// Largest tree (in significant nodes) for which the exact tree-edit distance is computed.
    /// Zhang-Shasha is super-quadratic, so larger trees get a cheap lower bound instead. The
    /// default of 2000 is a **guess**; calibrate against real repositories.
    pub ted_node_cap: usize,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            ted_node_cap: DEFAULT_TED_NODE_CAP,
        }
    }
}

/// How a mutation changed a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationMetrics {
    /// Lines added plus lines deleted (line ending changes count as changed lines).
    pub churn_lines: u32,
    /// Cyclomatic complexity of the new file minus that of the old file.
    pub cyclomatic_delta: i32,
    /// Unit-cost tree-edit distance over named, non-comment nodes (see `tree_edit_distance_exact`).
    pub tree_edit_distance: u32,
    /// `false` when a tree exceeded the node cap and `tree_edit_distance` is only a lower bound.
    pub tree_edit_distance_exact: bool,
    /// Declarations that were added, removed or whose own text changed, sorted by name and kind.
    pub changed_symbols: Vec<SymbolRef>,
}

/// Computes the [`MutationMetrics`] of `old` to `new` with the default [`MetricsConfig`].
///
/// # Errors
/// [`CodeIntelError::LanguageMismatch`] if the two trees are of different languages and
/// [`CodeIntelError::TooLarge`] if a file has more than 100000 declarations.
pub fn metrics(old: &Parsed, new: &Parsed) -> Result<MutationMetrics, CodeIntelError> {
    metrics_with(old, new, &MetricsConfig::default())
}

/// Like [`metrics`] with an explicit [`MetricsConfig`].
///
/// # Errors
/// See [`metrics`].
pub fn metrics_with(
    old: &Parsed,
    new: &Parsed,
    cfg: &MetricsConfig,
) -> Result<MutationMetrics, CodeIntelError> {
    if old.lang() != new.lang() {
        return Err(CodeIntelError::LanguageMismatch {
            old: old.lang(),
            new: new.lang(),
        });
    }
    let old_complexity = i64::from(cyclomatic_complexity(old));
    let new_complexity = i64::from(cyclomatic_complexity(new));
    let delta = new_complexity - old_complexity;
    let (distance, exact) = tree_edit_distance(old, new, cfg.ted_node_cap);
    Ok(MutationMetrics {
        churn_lines: churn_lines(old.source(), new.source()),
        cyclomatic_delta: i32::try_from(delta).unwrap_or(if delta < 0 {
            i32::MIN
        } else {
            i32::MAX
        }),
        tree_edit_distance: distance,
        tree_edit_distance_exact: exact,
        changed_symbols: changed_symbols(old, new)?,
    })
}

/// Lines added plus lines deleted between two byte strings.
///
/// Small inputs use the minimal Myers diff, so the count is the true edit distance and symmetric;
/// very large ones use Histogram, which is faster but not guaranteed minimal.
fn churn_lines(old: &[u8], new: &[u8]) -> u32 {
    let input = InternedInput::new(old, new);
    let lines = input.before.len().saturating_add(input.after.len());
    let algorithm = if lines <= MINIMAL_DIFF_LINE_CAP {
        Algorithm::MyersMinimal
    } else {
        Algorithm::Histogram
    };
    let diff = Diff::compute(algorithm, &input);
    diff.count_additions().saturating_add(diff.count_removals())
}

/// Cyclomatic complexity of the whole file: 1 plus its decision points.
///
/// Decision points per language: `if`/`else if`/`elif`, loops, `match` arms (Rust) and `case`
/// clauses, `catch`/`except`, ternaries, `&&` / `||` (and `??` in TypeScript, `and` / `or` in
/// Python) and comprehension `for` / `if` clauses. A `default` case and `?` do not count.
#[must_use]
pub fn cyclomatic_complexity(p: &Parsed) -> u32 {
    let lang = p.lang();
    let mut points = 0u32;
    walk_preorder(p.tree().root_node(), |node, _| {
        points = points.saturating_add(decision_weight(lang, node));
        Descend::Into
    });
    points.saturating_add(1)
}

fn has_short_circuit_operator(node: Node<'_>, ops: &[&str]) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| !c.is_named() && ops.contains(&c.kind()))
}

fn decision_weight(lang: Lang, node: Node<'_>) -> u32 {
    let kind = node.kind();
    let hit = match lang {
        Lang::Rust => match kind {
            "if_expression" | "while_expression" | "for_expression" | "loop_expression"
            | "match_arm" => true,
            "binary_expression" => has_short_circuit_operator(node, &["&&", "||"]),
            _ => false,
        },
        Lang::Python => matches!(
            kind,
            "if_statement"
                | "elif_clause"
                | "while_statement"
                | "for_statement"
                | "except_clause"
                | "except_group_clause"
                | "conditional_expression"
                | "boolean_operator"
                | "case_clause"
                | "for_in_clause"
                | "if_clause"
        ),
        Lang::Cpp | Lang::Cuda => match kind {
            "if_statement"
            | "while_statement"
            | "for_statement"
            | "for_range_loop"
            | "do_statement"
            | "catch_clause"
            | "conditional_expression" => true,
            "case_statement" => node.child_by_field_name("value").is_some(),
            "binary_expression" => has_short_circuit_operator(node, &["&&", "||"]),
            _ => false,
        },
        Lang::TypeScript => match kind {
            "if_statement" | "while_statement" | "do_statement" | "for_statement"
            | "for_in_statement" | "switch_case" | "catch_clause" | "ternary_expression" => true,
            "binary_expression" => has_short_circuit_operator(node, &["&&", "||", "??"]),
            _ => false,
        },
    };
    u32::from(hit)
}

/// The significant nodes of a tree for the edit distance: named, non-extra (comment) nodes.
struct SigTree {
    /// Hash of kind (and text, for leaves) per node, in pre-order.
    labels: Vec<u64>,
    /// Index of the nearest significant ancestor per node, in pre-order.
    parents: Vec<Option<usize>>,
}

fn significant_tree(p: &Parsed) -> SigTree {
    let src = p.source();
    let mut labels = Vec::new();
    let mut parents = Vec::new();
    // Significant ancestors of the node being visited: (depth, index).
    let mut ancestors: Vec<(u32, usize)> = Vec::new();
    walk_preorder(p.tree().root_node(), |node, depth| {
        if node.is_extra() {
            return Descend::Skip;
        }
        if !node.is_named() {
            return Descend::Into;
        }
        while ancestors.last().is_some_and(|&(d, _)| d >= depth) {
            ancestors.pop();
        }
        let index = labels.len();
        parents.push(ancestors.last().map(|&(_, i)| i));
        ancestors.push((depth, index));
        let kind = u64::from(node.kind_id());
        let label = if node.child_count() == 0 {
            xxh3_64_with_seed(node_text(node, src).as_bytes(), kind)
        } else {
            xxh3_64(&kind.to_le_bytes())
        };
        labels.push(label);
        Descend::Into
    });
    SigTree { labels, parents }
}

/// Tree-edit distance with unit costs, or a lower bound above `cap`.
fn tree_edit_distance(old: &Parsed, new: &Parsed, cap: usize) -> (u32, bool) {
    let a = significant_tree(old);
    let b = significant_tree(new);
    if a.labels == b.labels && a.parents == b.parents {
        return (0, true);
    }
    if a.labels.len().max(b.labels.len()) <= cap {
        (saturate(zhang_shasha(&a, &b)), true)
    } else {
        (saturate(label_lower_bound(&a.labels, &b.labels)), false)
    }
}

fn saturate(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `max(|A|, |B|)` minus the size of the label multiset intersection: every node of the larger
/// tree without a same-labelled partner needs at least one edit operation.
fn label_lower_bound(a: &[u64], b: &[u64]) -> usize {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort_unstable();
    b.sort_unstable();
    let (mut i, mut j, mut common) = (0usize, 0usize, 0usize);
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        match x.cmp(y) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    a.len().max(b.len()).saturating_sub(common)
}

/// A dense `rows x cols` matrix of `u32` with bounds-checked, non-panicking access.
struct Grid {
    cols: usize,
    data: Vec<u32>,
}

impl Grid {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            cols,
            data: vec![0; rows.saturating_mul(cols)],
        }
    }

    fn get(&self, row: usize, col: usize) -> u32 {
        self.data
            .get(row.saturating_mul(self.cols).saturating_add(col))
            .copied()
            .unwrap_or(0)
    }

    fn set(&mut self, row: usize, col: usize, value: u32) {
        if let Some(slot) = self
            .data
            .get_mut(row.saturating_mul(self.cols).saturating_add(col))
        {
            *slot = value;
        }
    }
}

/// A tree in post-order, the form Zhang-Shasha works on.
struct PostTree {
    labels: Vec<u64>,
    /// Post-order index of the left-most leaf descendant of each node.
    leftmost: Vec<usize>,
    /// Post-order indices whose left-most leaf is not shared with a later node.
    keyroots: Vec<usize>,
}

impl PostTree {
    fn label(&self, i: usize) -> u64 {
        self.labels.get(i).copied().unwrap_or(0)
    }

    fn leftmost(&self, i: usize) -> usize {
        self.leftmost.get(i).copied().unwrap_or(i)
    }
}

fn post_tree(t: &SigTree) -> PostTree {
    let n = t.labels.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for (i, parent) in t.parents.iter().enumerate() {
        match parent.and_then(|p| children.get_mut(p)) {
            Some(list) => list.push(i),
            None => roots.push(i),
        }
    }
    // Iterative post-order over the forest (normally a single root).
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut stack: Vec<(usize, bool)> = roots.iter().rev().map(|&r| (r, false)).collect();
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            order.push(node);
            continue;
        }
        stack.push((node, true));
        if let Some(kids) = children.get(node) {
            stack.extend(kids.iter().rev().map(|&k| (k, false)));
        }
    }
    let mut post_of = vec![0usize; n];
    for (post, &pre) in order.iter().enumerate() {
        if let Some(slot) = post_of.get_mut(pre) {
            *slot = post;
        }
    }
    let mut labels = Vec::with_capacity(n);
    let mut leftmost: Vec<usize> = Vec::with_capacity(n);
    for (post, &pre) in order.iter().enumerate() {
        labels.push(t.labels.get(pre).copied().unwrap_or(0));
        let first_child = children.get(pre).and_then(|k| k.first()).copied();
        let l = first_child
            .and_then(|c| post_of.get(c).copied())
            .and_then(|c| leftmost.get(c).copied())
            .unwrap_or(post);
        leftmost.push(l);
    }
    let mut last_with: BTreeMap<usize, usize> = BTreeMap::new();
    for (post, &l) in leftmost.iter().enumerate() {
        last_with.insert(l, post);
    }
    let mut keyroots: Vec<usize> = last_with.into_values().collect();
    keyroots.sort_unstable();
    PostTree {
        labels,
        leftmost,
        keyroots,
    }
}

/// Zhang-Shasha tree-edit distance with unit insert / delete / relabel costs.
fn zhang_shasha(a: &SigTree, b: &SigTree) -> usize {
    let t1 = post_tree(a);
    let t2 = post_tree(b);
    let (m, n) = (t1.labels.len(), t2.labels.len());
    if m == 0 || n == 0 {
        return m.max(n);
    }
    let mut treedist = Grid::new(m, n);
    let mut forest = Grid::new(m + 1, n + 1);
    for &i in &t1.keyroots {
        for &j in &t2.keyroots {
            let (li, lj) = (t1.leftmost(i), t2.leftmost(j));
            let (rows, cols) = (i - li + 2, j - lj + 2);
            forest.set(0, 0, 0);
            for di in 1..rows {
                forest.set(di, 0, forest.get(di - 1, 0) + 1);
            }
            for dj in 1..cols {
                forest.set(0, dj, forest.get(0, dj - 1) + 1);
            }
            for di in 1..rows {
                for dj in 1..cols {
                    let (i1, j1) = (li + di - 1, lj + dj - 1);
                    let delete = forest.get(di - 1, dj) + 1;
                    let insert = forest.get(di, dj - 1) + 1;
                    if t1.leftmost(i1) == li && t2.leftmost(j1) == lj {
                        let relabel = u32::from(t1.label(i1) != t2.label(j1));
                        let value = delete.min(insert).min(forest.get(di - 1, dj - 1) + relabel);
                        forest.set(di, dj, value);
                        treedist.set(i1, j1, value);
                    } else {
                        let (p, q) = (t1.leftmost(i1) - li, t2.leftmost(j1) - lj);
                        let value = delete
                            .min(insert)
                            .min(forest.get(p, q) + treedist.get(i1, j1));
                        forest.set(di, dj, value);
                    }
                }
            }
        }
    }
    treedist.get(m - 1, n - 1) as usize
}

/// The identity of a declaration within a file: qualified name, kind and occurrence number
/// (overloads and duplicates are told apart by order).
type DeclKey = (String, SymbolKind, usize);

/// xxh3 of the part of a declaration that belongs to the declaration itself: the whole text for
/// leaf declarations, header and footer (without the nested declarations) for containers.
fn own_fingerprint(d: &DeclInfo, src: &[u8]) -> u64 {
    let mut hasher = Xxh3::new();
    match (d.is_container, d.body) {
        (true, Some(body)) => {
            hasher.update(src.get(d.span.start..body.start).unwrap_or_default());
            hasher.update(src.get(body.end..d.span.end).unwrap_or_default());
        }
        _ => hasher.update(d.span.slice(src).unwrap_or_default()),
    }
    hasher.digest()
}

fn keyed_fingerprints(p: &Parsed) -> Result<BTreeMap<DeclKey, u64>, CodeIntelError> {
    let mut seen: BTreeMap<(String, SymbolKind), usize> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for d in declarations(p, MAX_SKELETON_ITEMS)? {
        let counter = seen.entry((d.qualified.clone(), d.kind)).or_insert(0);
        let key = (d.qualified.clone(), d.kind, *counter);
        *counter += 1;
        out.insert(key, own_fingerprint(&d, p.source()));
    }
    Ok(out)
}

fn changed_symbols(old: &Parsed, new: &Parsed) -> Result<Vec<SymbolRef>, CodeIntelError> {
    let before = keyed_fingerprints(old)?;
    let after = keyed_fingerprints(new)?;
    let mut changed: Vec<SymbolRef> = Vec::new();
    for (key, fp) in &after {
        if before.get(key) != Some(fp) {
            changed.push(SymbolRef {
                name: key.0.clone(),
                kind: key.1,
            });
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            changed.push(SymbolRef {
                name: key.0.clone(),
                kind: key.1,
            });
        }
    }
    changed.sort();
    changed.dedup();
    Ok(changed)
}

/// xxh3-64 fingerprints of every declaration node (functions, methods, classes, structs, ...),
/// nested ones included, in source order.
///
/// Callers compare fingerprints across revisions to skip re-analysing unchanged declarations.
/// Fingerprints are cheap dirty-check values, **not** collision resistant: never use them as a
/// security identity or a content address.
///
/// Declarations beyond 100000 per file are not fingerprinted.
#[must_use]
pub fn subtree_fingerprints(p: &Parsed) -> Vec<(ByteSpan, FastFingerprint)> {
    let src = p.source();
    match declarations(p, MAX_SKELETON_ITEMS) {
        Ok(decls) => decls
            .iter()
            .filter_map(|d| {
                d.span
                    .slice(src)
                    .map(|bytes| (d.span, FastFingerprint::new(xxh3_64(bytes))))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}
