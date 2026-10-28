//! Symbol-granular three-way merge.
//!
//! A file is cut into *units*: the named children of the root (declarations, imports, comments,
//! statements). Declarations are identified by kind, name and occurrence number; everything else
//! by its exact text. The unit sequences of `base`/`ours` and `base`/`theirs` are diffed and the
//! two edit scripts are combined diff3-style. Changes to different units merge; the same unit
//! changed differently on both sides is a conflict, except for classes, impls, traits and
//! modules, whose members are merged the same way (up to [`MAX_MERGE_DEPTH`] levels deep).
//!
//! All bytes of the result are copied from the three inputs, so EOL style, BOM, indentation and
//! comments are preserved exactly; nothing is ever re-printed from the syntax tree.

use crate::decl::{SymbolKind, SymbolRef, classify, is_type_scope};
use crate::error::CodeIntelError;
use crate::lang::Lang;
use crate::parse::Parsed;
use crate::walk::named_children;
use imara_diff::{Algorithm, Diff, Token};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use tree_sitter::Node;

/// Deepest level of nested declarations (class in module in ...) merged member by member.
/// Deeper nesting that changed on both sides is reported as a conflict.
const MAX_MERGE_DEPTH: usize = 8;

/// The result of [`merge3`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// A clean merge: the merged file.
    Merged(Vec<u8>),
    /// At least one symbol was changed incompatibly. No merged bytes are produced.
    Conflict {
        /// The conflicting symbols, sorted and deduplicated.
        symbols: Vec<SymbolRef>,
    },
}

/// Merges `ours` and `theirs` relative to their common ancestor `base`, at symbol granularity.
///
/// Rules: a unit changed on one side only takes that change; identical changes on both sides are
/// applied once; two different new symbols inserted at the same place are both kept (ours first);
/// the same symbol changed differently, or deleted on one side and changed on the other, is a
/// conflict. A conflict anywhere makes the whole outcome [`MergeOutcome::Conflict`]: the caller
/// never receives partially merged bytes.
///
/// Limits: members of a container are merged only if the container header and footer are
/// unchanged or changed on one side, and only for containers that have members in all three
/// versions. The merged text is not re-parsed, so two clean but semantically incompatible edits
/// (for example both sides renaming different callers of one function) are not detected.
///
/// # Errors
/// [`CodeIntelError::LanguageMismatch`] if the three trees are not all of the same language.
pub fn merge3(
    base: &Parsed,
    ours: &Parsed,
    theirs: &Parsed,
) -> Result<MergeOutcome, CodeIntelError> {
    for other in [ours, theirs] {
        if other.lang() != base.lang() {
            return Err(CodeIntelError::LanguageMismatch {
                old: base.lang(),
                new: other.lang(),
            });
        }
    }
    let lang = base.lang();
    let base_units = units_of(base.tree().root_node(), base.source(), lang, false);
    let ours_units = units_of(ours.tree().root_node(), ours.source(), lang, false);
    let theirs_units = units_of(theirs.tree().root_node(), theirs.source(), lang, false);
    let triple = Triple {
        base: &base_units,
        ours: &ours_units,
        theirs: &theirs_units,
    };
    let mut conflicts: Vec<SymbolRef> = Vec::new();
    let emits = merge_seq(&triple, lang, 0, "", &mut conflicts);

    let prefix = merge_text(
        prefix_of(&base_units, base.source()),
        prefix_of(&ours_units, ours.source()),
        prefix_of(&theirs_units, theirs.source()),
    );
    let suffix = merge_text(
        suffix_of(&base_units, base.source()),
        suffix_of(&ours_units, ours.source()),
        suffix_of(&theirs_units, theirs.source()),
    );
    if prefix.is_none() || suffix.is_none() {
        conflicts.push(SymbolRef {
            name: "<file prefix or suffix>".to_owned(),
            kind: SymbolKind::Other,
        });
    }
    if !conflicts.is_empty() {
        conflicts.sort();
        conflicts.dedup();
        return Ok(MergeOutcome::Conflict { symbols: conflicts });
    }
    let mut merged: Vec<u8> = Vec::with_capacity(ours.source().len().max(theirs.source().len()));
    merged.extend_from_slice(prefix.unwrap_or_default());
    merged.extend_from_slice(&render(&emits, &triple));
    merged.extend_from_slice(suffix.unwrap_or_default());
    Ok(MergeOutcome::Merged(merged))
}

/// A top-level (or member-level) piece of a file.
struct Unit<'a> {
    node: Node<'a>,
    src: &'a [u8],
    text: &'a [u8],
    start: usize,
    end: usize,
    /// Kind, name and occurrence number for declarations; `None` for everything else.
    key: Option<(SymbolKind, String, usize)>,
    /// The bytes between the previous unit and this one.
    sep_before: Option<&'a [u8]>,
    /// The bytes between this unit and the next one.
    sep_after: Option<&'a [u8]>,
}

fn units_of<'a>(parent: Node<'a>, src: &'a [u8], lang: Lang, in_type_scope: bool) -> Vec<Unit<'a>> {
    let kids = named_children(parent);
    let mut seen: HashMap<(SymbolKind, String), usize> = HashMap::new();
    let mut units = Vec::with_capacity(kids.len());
    for (i, node) in kids.iter().enumerate() {
        let key = classify(lang, *node, src).map(|c| {
            let kind = match c.kind {
                SymbolKind::Function if in_type_scope => SymbolKind::Method,
                other => other,
            };
            let counter = seen.entry((kind, c.name.clone())).or_insert(0);
            let ordinal = *counter;
            *counter = counter.saturating_add(1);
            (kind, c.name, ordinal)
        });
        let sep_before = i
            .checked_sub(1)
            .and_then(|p| kids.get(p))
            .and_then(|prev| src.get(prev.end_byte()..node.start_byte()));
        let sep_after = kids
            .get(i.saturating_add(1))
            .and_then(|next| src.get(node.end_byte()..next.start_byte()));
        units.push(Unit {
            node: *node,
            src,
            text: src
                .get(node.start_byte()..node.end_byte())
                .unwrap_or_default(),
            start: node.start_byte(),
            end: node.end_byte(),
            key,
            sep_before,
            sep_after,
        });
    }
    units
}

fn prefix_of<'a>(units: &[Unit<'a>], src: &'a [u8]) -> &'a [u8] {
    match units.first() {
        Some(first) => src.get(..first.start).unwrap_or_default(),
        None => src,
    }
}

fn suffix_of<'a>(units: &[Unit<'a>], src: &'a [u8]) -> &'a [u8] {
    match units.last() {
        Some(last) => src.get(last.end..).unwrap_or_default(),
        None => &[],
    }
}

/// Three-way merge of a plain text fragment: a side that equals the base yields to the other.
fn merge_text<'a>(base: &'a [u8], ours: &'a [u8], theirs: &'a [u8]) -> Option<&'a [u8]> {
    if ours == theirs || theirs == base {
        Some(ours)
    } else if ours == base {
        Some(theirs)
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Base,
    Ours,
    Theirs,
}

/// The unit sequences of the three versions.
struct Triple<'u, 'a> {
    base: &'u [Unit<'a>],
    ours: &'u [Unit<'a>],
    theirs: &'u [Unit<'a>],
}

impl<'u, 'a> Triple<'u, 'a> {
    fn units(&self, side: Side) -> &'u [Unit<'a>] {
        match side {
            Side::Base => self.base,
            Side::Ours => self.ours,
            Side::Theirs => self.theirs,
        }
    }

    fn unit(&self, side: Side, idx: usize) -> Option<&'u Unit<'a>> {
        self.units(side).get(idx)
    }

    fn emit(&self, side: Side, idx: usize) -> Option<Emit<'a>> {
        self.unit(side, idx).map(|u| Emit {
            side,
            idx,
            text: Cow::Borrowed(u.text),
        })
    }
}

/// One unit of the merged output, remembering where it came from for separator selection.
struct Emit<'a> {
    side: Side,
    idx: usize,
    text: Cow<'a, [u8]>,
}

/// A position in a cluster region: a base unit kept, or a unit of one side.
#[derive(Clone, Copy)]
enum Pick {
    Base(usize),
    Side(Side, usize),
}

impl Pick {
    fn unit(self) -> (Side, usize) {
        match self {
            Self::Base(i) => (Side::Base, i),
            Self::Side(s, i) => (s, i),
        }
    }
}

/// One hunk of a base-to-side diff.
struct Edit {
    start: usize,
    end: usize,
    repl: Range<usize>,
}

fn edits_of(diff: &Diff, base: &[Unit<'_>], side: &[Unit<'_>]) -> Vec<Edit> {
    let mut edits = Vec::new();
    for h in diff.hunks() {
        let bounds =
            [h.before.start, h.before.end, h.after.start, h.after.end].map(usize::try_from);
        let [Ok(start), Ok(end), Ok(repl_start), Ok(repl_end)] = bounds else {
            // A platform unable to represent the diff offsets must compare the whole units.
            // Never truncate a range or silently drop a proposed change.
            return vec![Edit {
                start: 0,
                end: base.len(),
                repl: 0..side.len(),
            }];
        };
        let edit = Edit {
            start,
            end,
            repl: repl_start..repl_end,
        };
        // Myers coalesces adjacent replacements. If identities align exactly, each declaration
        // is still its own merge unit; otherwise one shared conflict would absorb unrelated
        // one-sided changes. Insertions, deletions and reordered identities stay conservative.
        let aligned = base
            .get(edit.start..edit.end)
            .zip(side.get(edit.repl.clone()))
            .is_some_and(|(before, after)| {
                before.len() == after.len()
                    && before
                        .iter()
                        .zip(after)
                        .all(|(a, b)| a.key.is_some() && a.key == b.key)
            });
        if aligned {
            for (start, replacement) in (edit.start..edit.end).zip(edit.repl) {
                edits.push(Edit {
                    start,
                    end: start.saturating_add(1),
                    repl: replacement..replacement.saturating_add(1),
                });
            }
        } else {
            edits.push(edit);
        }
    }
    edits
}

/// Whether two base ranges conflict. Insertions (empty ranges) conflict with each other when at
/// the same point and with a replacement only when strictly inside it, so an insertion next to a
/// changed unit is independent of that change.
fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    match (a.0 == a.1, b.0 == b.1) {
        (false, false) => a.0 < b.1 && b.0 < a.1,
        (true, true) => a.0 == b.0,
        (true, false) => b.0 < a.0 && a.0 < b.1,
        (false, true) => a.0 < b.0 && b.0 < a.1,
    }
}

type TokenKey<'k> = (Option<(SymbolKind, &'k str, usize)>, &'k [u8]);

fn tokens_of<'k>(units: &'k [Unit<'_>], ids: &mut HashMap<TokenKey<'k>, u32>) -> Vec<Token> {
    units
        .iter()
        .map(|u| {
            let key: TokenKey<'k> = (u.key.as_ref().map(|(k, n, o)| (*k, n.as_str(), *o)), u.text);
            let next = u32::try_from(ids.len()).unwrap_or(u32::MAX);
            Token(*ids.entry(key).or_insert(next))
        })
        .collect()
}

fn qualify(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.to_owned()
    } else {
        format!("{scope}::{name}")
    }
}

fn symbol_of(scope: &str, unit: &Unit<'_>) -> SymbolRef {
    match &unit.key {
        Some((kind, name, _)) => SymbolRef {
            name: qualify(scope, name),
            kind: *kind,
        },
        None => SymbolRef {
            name: qualify(scope, "<non-declaration code>"),
            kind: SymbolKind::Other,
        },
    }
}

/// Merges one sequence of units; conflicts are appended to `conflicts`.
fn merge_seq<'u, 'a>(
    t: &Triple<'u, 'a>,
    lang: Lang,
    depth: usize,
    scope: &str,
    conflicts: &mut Vec<SymbolRef>,
) -> Vec<Emit<'a>> {
    let mut ids: HashMap<TokenKey<'u>, u32> = HashMap::new();
    let base_tokens = tokens_of(t.base, &mut ids);
    let ours_tokens = tokens_of(t.ours, &mut ids);
    let theirs_tokens = tokens_of(t.theirs, &mut ids);
    let num_tokens = u32::try_from(ids.len()).unwrap_or(u32::MAX);
    let mut ours_diff = Diff::default();
    ours_diff.compute_with(Algorithm::Myers, &base_tokens, &ours_tokens, num_tokens);
    let mut theirs_diff = Diff::default();
    theirs_diff.compute_with(Algorithm::Myers, &base_tokens, &theirs_tokens, num_tokens);
    let ours_edits = edits_of(&ours_diff, t.base, t.ours);
    let theirs_edits = edits_of(&theirs_diff, t.base, t.theirs);
    let token = |pick: Pick| -> Option<Token> {
        match pick {
            Pick::Base(i) => base_tokens.get(i).copied(),
            Pick::Side(Side::Ours, i) => ours_tokens.get(i).copied(),
            Pick::Side(_, i) => theirs_tokens.get(i).copied(),
        }
    };

    let mut out: Vec<Emit<'a>> = Vec::new();
    let mut pos = 0usize;
    let (mut oi, mut ti) = (0usize, 0usize);
    loop {
        let take_ours = match (ours_edits.get(oi), theirs_edits.get(ti)) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(o), Some(h)) => (o.start, o.end != o.start) <= (h.start, h.end != h.start),
        };
        let first = if take_ours {
            ours_edits.get(oi)
        } else {
            theirs_edits.get(ti)
        };
        let Some(first) = first else { break };
        let (mut cs, mut ce) = (first.start, first.end);
        let mut ours_in: Vec<&Edit> = Vec::new();
        let mut theirs_in: Vec<&Edit> = Vec::new();
        if take_ours {
            ours_in.push(first);
            oi = oi.saturating_add(1);
        } else {
            theirs_in.push(first);
            ti = ti.saturating_add(1);
        }
        loop {
            let mut grew = false;
            if let Some(e) = ours_edits
                .get(oi)
                .filter(|e| overlaps((cs, ce), (e.start, e.end)))
            {
                (cs, ce) = (cs.min(e.start), ce.max(e.end));
                ours_in.push(e);
                oi = oi.saturating_add(1);
                grew = true;
            }
            if let Some(e) = theirs_edits
                .get(ti)
                .filter(|e| overlaps((cs, ce), (e.start, e.end)))
            {
                (cs, ce) = (cs.min(e.start), ce.max(e.end));
                theirs_in.push(e);
                ti = ti.saturating_add(1);
                grew = true;
            }
            if !grew {
                break;
            }
        }
        out.extend((pos..cs).filter_map(|k| t.emit(Side::Base, k)));
        pos = pos.max(ce);

        let ours_picks = region(&ours_in, Side::Ours, cs, ce);
        let theirs_picks = region(&theirs_in, Side::Theirs, cs, ce);
        let picks_emit = |picks: &[Pick]| -> Vec<Emit<'a>> {
            picks
                .iter()
                .filter_map(|p| {
                    let (side, idx) = p.unit();
                    t.emit(side, idx)
                })
                .collect()
        };
        match (ours_in.is_empty(), theirs_in.is_empty()) {
            (false, true) => out.extend(picks_emit(&ours_picks)),
            (true, false) => out.extend(picks_emit(&theirs_picks)),
            _ => {
                let same = ours_picks.len() == theirs_picks.len()
                    && ours_picks
                        .iter()
                        .zip(&theirs_picks)
                        .all(|(a, b)| token(*a) == token(*b));
                if same {
                    out.extend(picks_emit(&ours_picks));
                } else if cs == ce {
                    union_insertions(
                        t,
                        scope,
                        &ours_picks,
                        &theirs_picks,
                        &token,
                        &mut out,
                        conflicts,
                    );
                } else if let Some(merged) = merge_container(
                    t,
                    (cs, ce),
                    (&ours_picks, &theirs_picks),
                    lang,
                    depth,
                    scope,
                    conflicts,
                ) {
                    out.push(merged);
                } else {
                    let mut involved: Vec<SymbolRef> = (cs..ce)
                        .filter_map(|k| t.unit(Side::Base, k))
                        .chain(ours_picks.iter().chain(&theirs_picks).filter_map(|p| {
                            let (side, idx) = p.unit();
                            t.unit(side, idx)
                        }))
                        .map(|u| symbol_of(scope, u))
                        .collect();
                    let named: Vec<SymbolRef> = involved
                        .iter()
                        .filter(|s| s.kind != SymbolKind::Other)
                        .cloned()
                        .collect();
                    if !named.is_empty() {
                        involved = named;
                    }
                    involved.sort();
                    involved.dedup();
                    conflicts.extend(involved);
                }
            }
        }
    }
    out.extend((pos..t.base.len()).filter_map(|k| t.emit(Side::Base, k)));
    out
}

/// The units of base range `cs..ce` after applying the given edits of one side.
fn region(edits: &[&Edit], side: Side, cs: usize, ce: usize) -> Vec<Pick> {
    let mut picks = Vec::new();
    let mut p = cs;
    for e in edits {
        picks.extend((p..e.start).map(Pick::Base));
        picks.extend(e.repl.clone().map(|k| Pick::Side(side, k)));
        p = e.end;
    }
    picks.extend((p..ce).map(Pick::Base));
    picks
}

/// Both sides inserted at the same point: keep both (ours first) unless they define the same
/// symbol differently; identical insertions are kept once.
fn union_insertions<'a>(
    t: &Triple<'_, 'a>,
    scope: &str,
    ours: &[Pick],
    theirs: &[Pick],
    token: &dyn Fn(Pick) -> Option<Token>,
    out: &mut Vec<Emit<'a>>,
    conflicts: &mut Vec<SymbolRef>,
) {
    let mut kept: Vec<Pick> = ours.to_vec();
    let mut clash = false;
    for tp in theirs {
        if ours.iter().any(|op| token(*op) == token(*tp)) {
            continue;
        }
        let (ts, ti) = tp.unit();
        let their_unit = t.unit(ts, ti);
        let key = their_unit.and_then(|u| u.key.as_ref());
        let collides = key.is_some()
            && ours.iter().any(|op| {
                let (os, oi) = op.unit();
                t.unit(os, oi).and_then(|u| u.key.as_ref()) == key
            });
        if collides {
            clash = true;
            if let Some(u) = their_unit {
                conflicts.push(symbol_of(scope, u));
            }
        } else {
            kept.push(*tp);
        }
    }
    if !clash {
        out.extend(kept.iter().filter_map(|p| {
            let (side, idx) = p.unit();
            t.emit(side, idx)
        }));
    }
}

/// Both sides changed one container (class, impl, trait, module) that has the same identity in
/// all three versions: merge its members. Returns `None` if the container cannot be merged
/// member-wise, in which case the caller reports a conflict.
fn merge_container<'a>(
    t: &Triple<'_, 'a>,
    base_range: (usize, usize),
    picks: (&[Pick], &[Pick]),
    lang: Lang,
    depth: usize,
    scope: &str,
    conflicts: &mut Vec<SymbolRef>,
) -> Option<Emit<'a>> {
    if depth >= MAX_MERGE_DEPTH || base_range.0.checked_add(1) != Some(base_range.1) {
        return None;
    }
    let ([Pick::Side(_, oi)], [Pick::Side(_, ti)]) = picks else {
        return None;
    };
    let bu = t.unit(Side::Base, base_range.0)?;
    let ou = t.unit(Side::Ours, *oi)?;
    let tu = t.unit(Side::Theirs, *ti)?;
    let key = bu.key.as_ref()?;
    if ou.key.as_ref() != Some(key) || tu.key.as_ref() != Some(key) {
        return None;
    }
    let cb = classify(lang, bu.node, bu.src)?;
    let co = classify(lang, ou.node, ou.src)?;
    let ct = classify(lang, tu.node, tu.src)?;
    let (mb, mo, mt) = (cb.members?, co.members?, ct.members?);
    let in_scope = is_type_scope(cb.kind);
    let lb = units_of(mb, bu.src, lang, in_scope);
    let lo = units_of(mo, ou.src, lang, in_scope);
    let lt = units_of(mt, tu.src, lang, in_scope);
    let (first_b, last_b) = (lb.first()?, lb.last()?);
    let (first_o, last_o) = (lo.first()?, lo.last()?);
    let (first_t, last_t) = (lt.first()?, lt.last()?);
    let header = merge_text(
        bu.src.get(bu.start..first_b.start)?,
        ou.src.get(ou.start..first_o.start)?,
        tu.src.get(tu.start..first_t.start)?,
    )?;
    let footer = merge_text(
        bu.src.get(last_b.end..bu.end)?,
        ou.src.get(last_o.end..ou.end)?,
        tu.src.get(last_t.end..tu.end)?,
    )?;
    let sub = Triple {
        base: &lb,
        ours: &lo,
        theirs: &lt,
    };
    let child_scope = qualify(scope, &key.1);
    let child_depth = depth.checked_add(1)?;
    let emits = merge_seq(&sub, lang, child_depth, &child_scope, conflicts);
    let mut bytes = Vec::with_capacity(ou.text.len().max(tu.text.len()));
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(&render(&emits, &sub));
    bytes.extend_from_slice(footer);
    Some(Emit {
        side: Side::Ours,
        idx: *oi,
        text: Cow::Owned(bytes),
    })
}

/// Concatenates the emitted units with the separators of the versions they came from.
fn render(emits: &[Emit<'_>], t: &Triple<'_, '_>) -> Vec<u8> {
    let fallback = default_gap(t);
    let mut out = Vec::new();
    for (n, e) in emits.iter().enumerate() {
        if let Some(prev) = n.checked_sub(1).and_then(|p| emits.get(p)) {
            out.extend_from_slice(gap_between(prev, e, t).unwrap_or(&fallback));
        }
        out.extend_from_slice(&e.text);
    }
    out
}

/// The separator to put between two consecutive output units.
fn gap_between<'a>(a: &Emit<'_>, b: &Emit<'_>, t: &Triple<'_, 'a>) -> Option<&'a [u8]> {
    let before = |e: &Emit<'_>| t.unit(e.side, e.idx).and_then(|u| u.sep_before);
    let after = |e: &Emit<'_>| t.unit(e.side, e.idx).and_then(|u| u.sep_after);
    if a.side == b.side && a.idx.checked_add(1) == Some(b.idx) {
        return before(b);
    }
    if a.side != Side::Base
        && let Some(g) = after(a)
    {
        return Some(g);
    }
    if b.side != Side::Base
        && let Some(g) = before(b)
    {
        return Some(g);
    }
    after(a).or_else(|| before(b))
}

/// A separator for the rare case that no version supplies one: the file's EOL plus the indent of
/// the first unit.
fn default_gap(t: &Triple<'_, '_>) -> Vec<u8> {
    let first = t
        .base
        .first()
        .or_else(|| t.ours.first())
        .or_else(|| t.theirs.first());
    let Some(unit) = first else {
        return b"\n".to_vec();
    };
    let crlf = unit.src.windows(2).any(|w| w == b"\r\n");
    let mut gap = if crlf {
        b"\r\n".to_vec()
    } else {
        b"\n".to_vec()
    };
    let before = unit.src.get(..unit.start).unwrap_or_default();
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i.saturating_add(1));
    let indent = before.get(line_start..).unwrap_or_default();
    if indent.iter().all(|b| *b == b' ' || *b == b'\t') {
        gap.extend_from_slice(indent);
    }
    gap
}
