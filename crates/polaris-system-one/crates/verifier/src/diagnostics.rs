//! Ranking operates on structured diagnostics; compiler text is not reparsed here.
use polaris_code_intel::ByteSpan;
use std::collections::BTreeMap;

/// Opaque diagnostic identity assigned by the diagnostic producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticId(pub u64);

/// Producer-supplied diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    /// Informational note.
    Note,
    /// Non-fatal warning.
    Warning,
    /// Compilation or execution error.
    Error,
    /// Fatal tool failure.
    Fatal,
}

/// Sanitized structured diagnostic supplied by RTK or the execution owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable producer identity.
    pub id: DiagnosticId,
    /// Severity established by the producer.
    pub severity: DiagnosticSeverity,
    /// Location in the current source, if known.
    pub span: Option<ByteSpan>,
    /// Whether it identifies an independent root cause rather than a follow-on error.
    pub primary: bool,
    /// Producer-supplied duplicate-group identity; equal values are one cause.
    pub group: u64,
}

/// One deduplicated diagnostic and deterministic ranking evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedDiagnostic {
    /// Selected representative id.
    pub id: DiagnosticId,
    /// Severity of the representative.
    pub severity: DiagnosticSeverity,
    /// Whether the diagnostic intersects a changed source range.
    pub touches_change: bool,
    /// Number of additional entries in the same group.
    pub duplicates: usize,
}

/// Deduplicate by producer group and prioritize severity, changed ranges and primary causes.
#[must_use]
pub fn rank_diagnostics(diagnostics: &[Diagnostic], changed: &[ByteSpan], limit: usize) -> Vec<RankedDiagnostic> {
    let mut groups: BTreeMap<u64, (&Diagnostic, usize)> = BTreeMap::new();
    let touches = |diagnostic: &Diagnostic| diagnostic.span.is_some_and(|span| {
        changed.iter().any(|range| range.start < span.end && span.start < range.end)
    });
    for diagnostic in diagnostics {
        let entry = groups.entry(diagnostic.group).or_insert((diagnostic, 0));
        entry.1 = entry.1.saturating_add(1);
        let candidate = (diagnostic.severity, touches(diagnostic), diagnostic.primary);
        let current = (entry.0.severity, touches(entry.0), entry.0.primary);
        if candidate > current || (candidate == current && diagnostic.id < entry.0.id) {
            entry.0 = diagnostic;
        }
    }
    let mut ranked: Vec<_> = groups.into_values().map(|(diagnostic, count)| {
        (diagnostic.primary, RankedDiagnostic {
            id: diagnostic.id,
            severity: diagnostic.severity,
            touches_change: touches(diagnostic),
            duplicates: count.saturating_sub(1),
        })
    }).collect();
    ranked.sort_by(|(ap, a), (bp, b)| {
        b.severity.cmp(&a.severity)
            .then_with(|| b.touches_change.cmp(&a.touches_change))
            .then_with(|| bp.cmp(ap))
            .then_with(|| a.id.cmp(&b.id))
    });
    ranked.into_iter().take(limit).map(|(_, diagnostic)| diagnostic).collect()
}
