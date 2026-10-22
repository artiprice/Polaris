//! Signature-level blast estimates over code-intel skeletons.

use crate::error::{GuardError, ViewError};
use polaris_code_intel::{ByteSpan, Skeleton, SkeletonItem, SymbolRef};
use std::collections::{BTreeMap, BTreeSet};

/// Confidence of injected call-site facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceConfidence {
    /// The index is syntax-based and does not resolve types or macro expansion.
    SyntacticEstimate,
    /// The provider cannot establish complete reference facts.
    Unknown,
}

/// Typed call-site counts provided by an AST/reference index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSiteFacts {
    /// Number of indexed call sites.
    pub call_sites: u32,
    /// Number of files containing those call sites.
    pub files: u32,
    /// Number of crates containing those call sites.
    pub crates: u32,
    /// Number of call sites in tests.
    pub test_call_sites: u32,
    /// Whether counts are an AST approximation or unavailable/incomplete.
    pub confidence: ReferenceConfidence,
}

/// Injected reference index. The provider owns parsing and language-specific reference extraction.
pub trait SymbolReferenceView {
    /// Counts references to a symbol, or returns `None` when the index has no evidence for it.
    fn call_sites(&self, symbol: &SymbolRef) -> Result<Option<CallSiteFacts>, ViewError>;
}

/// Symbol-level change observed between two code-intel skeletons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstChange {
    /// The symbol was added.
    Added,
    /// The symbol was removed.
    Removed,
    /// The symbol remains but its signature text changed.
    SignatureChanged,
}

/// A changed symbol and its injected caller estimate, if available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstImpact {
    /// Symbol identity from code-intel.
    pub symbol: SymbolRef,
    /// Kind of signature change.
    pub change: AstChange,
    /// Old declaration span, if present.
    pub old_span: Option<ByteSpan>,
    /// New declaration span, if present.
    pub new_span: Option<ByteSpan>,
    /// Injected syntactic caller estimate. `None` means no reference evidence was available.
    pub call_sites: Option<CallSiteFacts>,
}

/// Aggregate signature change report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AstBlastReport {
    /// Changed symbols in deterministic order.
    pub impacts: Vec<AstImpact>,
    /// Whether all impacted symbols had call-site evidence.
    pub references_complete: bool,
}

/// Bound on changed symbols accepted in one diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AstImpactLimits {
    /// Maximum number of changed symbol identities.
    pub max_changed_symbols: usize,
}

impl AstImpactLimits {
    /// Creates a positive changed-symbol bound.
    pub fn new(max_changed_symbols: usize) -> Result<Self, GuardError> {
        if max_changed_symbols == 0 {
            return Err(GuardError::InvalidLimits);
        }
        Ok(Self {
            max_changed_symbols,
        })
    }
}

/// Compares signature skeletons and asks an injected index for affected call-site counts.
///
/// Body-only edits have no impact entry. A changed signature with no indexed references is
/// retained with `call_sites: None`; it is never silently reported as zero callers.
///
/// # Errors
/// Returns [`GuardError::TooManySymbols`] when the explicit changed-symbol bound is exceeded, or
/// wraps an error from the injected reference view.
pub fn ast_blast_radius<V: SymbolReferenceView>(
    old: &Skeleton,
    new: &Skeleton,
    references: &V,
    limits: AstImpactLimits,
) -> Result<AstBlastReport, GuardError> {
    if limits.max_changed_symbols == 0 {
        return Err(GuardError::InvalidLimits);
    }
    let old_symbols = signatures(old);
    let new_symbols = signatures(new);
    let symbols: BTreeSet<SymbolRef> = old_symbols
        .keys()
        .chain(new_symbols.keys())
        .cloned()
        .collect();
    let mut changed = Vec::new();
    for symbol in symbols {
        let old_items = old_symbols.get(&symbol);
        let new_items = new_symbols.get(&symbol);
        let old_signatures = signatures_for(old_items);
        let new_signatures = signatures_for(new_items);
        let old_missing = old_items.is_none();
        let new_missing = new_items.is_none();
        let change = match (old_missing, new_missing) {
            (true, false) => Some(AstChange::Added),
            (false, true) => Some(AstChange::Removed),
            (false, false) if old_signatures != new_signatures => Some(AstChange::SignatureChanged),
            _ => None,
        };
        if let Some(change) = change {
            if changed.len() >= limits.max_changed_symbols {
                return Err(GuardError::TooManySymbols {
                    limit: limits.max_changed_symbols,
                });
            }
            let old_span = old_items.and_then(|items| items.first().map(|item| item.span));
            let new_span = new_items.and_then(|items| items.first().map(|item| item.span));
            changed.push((symbol, change, old_span, new_span));
        }
    }
    let mut impacts = Vec::with_capacity(changed.len());
    let mut references_complete = true;
    for (symbol, change, old_span, new_span) in changed {
        let call_sites = references
            .call_sites(&symbol)
            .map_err(GuardError::Workspace)?;
        if call_sites.is_some_and(|facts| {
            facts.test_call_sites > facts.call_sites
                || facts.files > facts.call_sites
                || facts.crates > facts.files
                || facts.call_sites == 0 && (facts.files != 0 || facts.crates != 0)
        }) {
            return Err(GuardError::Workspace(ViewError::InvalidFacts));
        }
        if call_sites.is_none()
            || call_sites.is_some_and(|facts| facts.confidence == ReferenceConfidence::Unknown)
        {
            references_complete = false;
        }
        impacts.push(AstImpact {
            symbol,
            change,
            old_span,
            new_span,
            call_sites,
        });
    }
    Ok(AstBlastReport {
        impacts,
        references_complete,
    })
}

fn signatures(skeleton: &Skeleton) -> BTreeMap<SymbolRef, Vec<&SkeletonItem>> {
    let mut map: BTreeMap<SymbolRef, Vec<&SkeletonItem>> = BTreeMap::new();
    let mut scopes: Vec<(u32, String)> = Vec::new();
    for item in &skeleton.items {
        while scopes.last().is_some_and(|(depth, _)| *depth >= item.depth) {
            scopes.pop();
        }
        let mut name = scopes
            .iter()
            .map(|(_, scope)| scope.as_str())
            .collect::<Vec<_>>();
        name.push(item.name.as_str());
        map.entry(SymbolRef {
            name: name.join("::"),
            kind: item.kind,
        })
        .or_default()
        .push(item);
        if matches!(
            item.kind,
            polaris_code_intel::SymbolKind::Class
                | polaris_code_intel::SymbolKind::Struct
                | polaris_code_intel::SymbolKind::Trait
                | polaris_code_intel::SymbolKind::Impl
                | polaris_code_intel::SymbolKind::Interface
                | polaris_code_intel::SymbolKind::Module
        ) {
            scopes.push((item.depth, item.name.clone()));
        }
    }
    map
}

fn signatures_for(items: Option<&Vec<&SkeletonItem>>) -> Vec<String> {
    match items {
        Some(items) => {
            let mut signatures: Vec<String> =
                items.iter().map(|item| item.signature.clone()).collect();
            signatures.sort_unstable();
            signatures
        }
        None => Vec::new(),
    }
}
