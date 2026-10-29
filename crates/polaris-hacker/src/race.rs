//! Lexical race candidates in Rust source (advisory heuristics, never proof):
//!
//! - `race-split-lock`: a value read under one lock (bound by `let` and used in the write, or
//!   tested in a condition just before it) is written under a second lock of the same mutex or
//!   `RwLock`, so another thread can change it in between (check-then-act);
//! - `race-atomic-load-store`: an atomic read with `load` (bound and used in the store, or tested
//!   just before it) is written back with a separate `store`, losing concurrent updates
//!   (`fetch_*` or `compare_exchange` do it atomically).
//!
//! Locks whose guard is bound (`let g = m.lock().unwrap();`, `if let Ok(g) = m.lock()`) hold it
//! for their block and are only paired when the guard is dropped explicitly first; writes such
//! as `*m.lock().unwrap() = v;` are not checks. `read()`/`write()` count as locks only when the
//! result is unwrapped, expected or awaited, so other methods with those names are ignored.
//!
//! Comments and literals are blanked first, so offsets stay those of the original source.

use std::sync::LazyLock;

use regex::Regex;

use crate::{Confidence, Finding, FindingClass};

/// Rule id: the same lock taken twice with the first guard dropped in between.
pub const SPLIT_LOCK: &str = "race-split-lock";
/// Rule id: an atomic read by `load` and written by a separate `store`.
pub const ATOMIC_LOAD_STORE: &str = "race-atomic-load-store";

const RECEIVER: &str = r"((?:[A-Za-z_]\w*\s*\.\s*)*[A-Za-z_]\w*)";

static FUNCTION: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\bfn\s+[A-Za-z_]\w*").ok());
static LOCK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{RECEIVER}\s*\.\s*(?:lock\s*\(\s*\)|(?:read|write)\s*\(\s*\)\s*\.\s*(?:unwrap\s*\(\s*\)|expect\s*\(|await\b))"
    ))
    .ok()
});
static LOAD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(&format!(r"{RECEIVER}\s*\.\s*load\s*\(")).ok());
static STORE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(&format!(r"{RECEIVER}\s*\.\s*store\s*\(")).ok());
static READ_MODIFY_WRITE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{RECEIVER}\s*\.\s*(?:fetch_\w+|compare_exchange\w*|swap|fetch_update)\s*\("
    ))
    .ok()
});
static BOUND_GUARD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\blet\s+(?:mut\s+)?([A-Za-z_]\w*)\s*(?::[^=]*)?=\s*$").ok());
static GUARD_TAIL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:\.\s*unwrap\s*\(\s*\)|\.\s*expect\s*\([^;]*?\)|\?)?\s*;").ok()
});
/// After a lock call: the unwrapping chain and then a plain `=` (the lock is written, not read).
static WRITE_TAIL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"^(?:\s*\.\s*unwrap\s*\(\s*\)|\s*\.\s*expect\s*\([^;]*?\)|\s*\.\s*map_err\s*\([^;]*?\)|\s*\?)*\s*=[^=]",
    )
    .ok()
});
static IDENTIFIER: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b[A-Za-z_]\w*\b").ok());
/// Lines within which a condition counts as the check of the following write.
const CONDITION_WINDOW: usize = 8;
/// Lines within which a bound value counts as the check of the following write.
const VALUE_WINDOW: usize = 25;

/// One located occurrence of a receiver call.
struct Call {
    receiver: String,
    start: usize,
    end: usize,
}

/// Race candidates in one Rust file, ordered by position.
#[must_use]
pub fn race_candidates(path: &str, source: &[u8]) -> Vec<Finding> {
    let Ok(text) = std::str::from_utf8(source) else {
        return Vec::new();
    };
    let code = blank_rust(text);
    let mut findings: Vec<Finding> = Vec::new();
    for body in function_bodies(&code) {
        let Some(region) = code.get(body.clone()) else {
            continue;
        };
        for (start, end, rule, evidence) in split_locks(&code, region, body.start)
            .into_iter()
            .chain(load_stores(&code, region, body.start))
        {
            if findings
                .iter()
                .any(|known| known.start == start && known.rule == rule)
            {
                continue;
            }
            findings.push(Finding {
                path: path.to_owned(),
                class: FindingClass::RaceCandidate,
                rule: rule.to_owned(),
                start,
                end,
                evidence,
                confidence: Confidence::AdvisoryHeuristic,
            });
        }
    }
    findings.sort_by(|a, b| a.start.cmp(&b.start).then(a.rule.cmp(&b.rule)));
    findings
}

fn calls(pattern: &LazyLock<Option<Regex>>, region: &str, offset: usize) -> Vec<Call> {
    let Some(pattern) = pattern.as_ref() else {
        return Vec::new();
    };
    pattern
        .captures_iter(region)
        .filter_map(|found| {
            let whole = found.get(0)?;
            let receiver = found.get(1)?;
            Some(Call {
                receiver: receiver
                    .as_str()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect(),
                start: offset.saturating_add(whole.start()),
                end: offset.saturating_add(whole.end()),
            })
        })
        .collect()
}

/// How the statement holding a lock or `load` uses it.
enum Use {
    /// `let guard = m.lock().unwrap();` or `if let Ok(guard) = m.lock()`: the guard itself.
    Guard(String),
    /// `let value = *m.lock().unwrap();`: a value read out under the lock, bound to these names.
    Value(Vec<String>),
    /// Inside an `if`/`while` condition.
    Condition,
    /// Anything else (a write, a plain expression).
    Other,
}

/// The text of the statement up to `at`.
fn statement_head(code: &str, at: usize) -> &str {
    let before = code.get(..at).unwrap_or_default();
    let start = before
        .rfind([';', '{', '}'])
        .map_or(0, |found| found.saturating_add(1));
    before.get(start..).unwrap_or_default()
}

fn classify(code: &str, call: &Call) -> Use {
    let head = statement_head(code, call.start);
    let tail = code.get(call.end..).unwrap_or_default();
    if WRITE_TAIL
        .as_ref()
        .is_some_and(|write| write.is_match(tail))
    {
        return Use::Other;
    }
    let trimmed = head.trim_start();
    let condition = trimmed.starts_with("if ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("else if ")
        || trimmed.starts_with("match ");
    if let Some(pattern) = trimmed
        .strip_prefix("if let ")
        .or_else(|| trimmed.strip_prefix("while let "))
        .or_else(|| trimmed.strip_prefix("let "))
    {
        let pattern = pattern.trim_end();
        // The guard itself is bound when the lock call directly follows `=`.
        if let Some(binding) = pattern.strip_suffix('=') {
            let names = names(binding);
            let direct = BOUND_GUARD
                .as_ref()
                .is_some_and(|bound| bound.is_match(head))
                && GUARD_TAIL
                    .as_ref()
                    .is_some_and(|guard| guard.is_match(tail));
            if direct || binding.contains("Ok(") || binding.contains("Some(") {
                return Use::Guard(names.into_iter().next().unwrap_or_default());
            }
            if !condition {
                return Use::Value(names);
            }
        }
        return if condition {
            Use::Condition
        } else {
            Use::Value(names(pattern))
        };
    }
    if trimmed.starts_with("match ") {
        // The scrutinee's guard lives through the arms.
        return Use::Guard(String::new());
    }
    if condition {
        Use::Condition
    } else {
        Use::Other
    }
}

/// Binding names in a `let` pattern (`(a, mut b)`, `Some(x)`, `y: T`).
fn names(pattern: &str) -> Vec<String> {
    let pattern = pattern.split(':').next().unwrap_or_default();
    IDENTIFIER.as_ref().map_or_else(Vec::new, |identifier| {
        identifier
            .find_iter(pattern)
            .map(|found| found.as_str())
            .filter(|name| {
                !matches!(*name, "mut" | "ref" | "Ok" | "Some" | "Err" | "None" | "_")
                    && name.chars().next().is_some_and(char::is_lowercase)
            })
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn lines_between(code: &str, from: usize, to: usize) -> usize {
    code.get(from..to)
        .map_or(usize::MAX, |between| between.matches('\n').count())
}

/// Whether one of `names` is used between `from` and the end of the statement holding `to`.
fn used(code: &str, names: &[String], from: usize, to: usize) -> bool {
    let end = code
        .get(to..)
        .and_then(|rest| rest.find(';'))
        .map_or(code.len(), |found| to.saturating_add(found));
    let Some(between) = code.get(from..end) else {
        return false;
    };
    IDENTIFIER.as_ref().is_some_and(|identifier| {
        identifier
            .find_iter(between)
            .any(|found| names.iter().any(|name| name == found.as_str()))
    })
}

/// Whether `first` is the check of a write at `second`.
fn checks(code: &str, first: &Call, second: &Call) -> bool {
    match classify(code, first) {
        Use::Value(names) => {
            !names.is_empty()
                && lines_between(code, first.start, second.start) <= VALUE_WINDOW
                && used(code, &names, first.end, second.start)
        }
        Use::Condition => lines_between(code, first.start, second.start) <= CONDITION_WINDOW,
        Use::Guard(name) => {
            !name.is_empty()
                && code
                    .get(first.end..second.start)
                    .is_some_and(|between| dropped(between, &name))
        }
        Use::Other => false,
    }
}

/// `(start, end, rule, evidence)` of each lock that writes what an earlier lock of the same
/// receiver checked.
fn split_locks(
    code: &str,
    region: &str,
    offset: usize,
) -> Vec<(usize, usize, &'static str, String)> {
    let locks = calls(&LOCK, region, offset);
    let mut found = Vec::new();
    for (index, first) in locks.iter().enumerate() {
        let Some(second) = locks
            .iter()
            .skip(index.saturating_add(1))
            .find(|later| later.receiver == first.receiver)
        else {
            continue;
        };
        if checks(code, first, second) {
            found.push((
                second.start,
                second.end,
                SPLIT_LOCK,
                format!("heuristic:{}:released-between-locks", first.receiver),
            ));
        }
    }
    found
}

fn dropped(between: &str, name: &str) -> bool {
    let compact: String = between.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains(&format!("drop({name})"))
}

/// `(start, end, rule, evidence)` of each store that writes what an earlier `load` of the same
/// atomic checked, with no read-modify-write of it in between.
fn load_stores(
    code: &str,
    region: &str,
    offset: usize,
) -> Vec<(usize, usize, &'static str, String)> {
    let loads = calls(&LOAD, region, offset);
    let stores = calls(&STORE, region, offset);
    let atomic = calls(&READ_MODIFY_WRITE, region, offset);
    let locks = calls(&LOCK, region, offset);
    // A guard held across both (`let _gate = inner.lock();`) serialises the load and the store.
    let serialised = |load: &Call, store: &Call| {
        locks.iter().any(|lock| {
            lock.start < load.start
                && match classify(code, lock) {
                    Use::Guard(name) => {
                        name.is_empty()
                            || !code
                                .get(lock.end..store.start)
                                .is_some_and(|between| dropped(between, &name))
                    }
                    _ => false,
                }
        })
    };
    stores
        .iter()
        .filter(|store| {
            loads.iter().any(|load| {
                load.receiver == store.receiver
                    && load.start < store.start
                    && !atomic.iter().any(|rmw| {
                        rmw.receiver == store.receiver
                            && rmw.start > load.start
                            && rmw.start < store.start
                    })
                    && checks(code, load, store)
                    && !serialised(load, store)
            })
        })
        .map(|store| {
            (
                store.start,
                store.end,
                ATOMIC_LOAD_STORE,
                format!("heuristic:{}:load-then-store", store.receiver),
            )
        })
        .collect()
}

/// Byte ranges of function bodies (between the braces), nested ones included.
fn function_bodies(code: &str) -> Vec<std::ops::Range<usize>> {
    let Some(function) = FUNCTION.as_ref() else {
        return Vec::new();
    };
    let bytes = code.as_bytes();
    let mut bodies = Vec::new();
    for found in function.find_iter(code) {
        let mut at = found.end();
        // The signature ends at the body's `{`, or at `;` for a declaration without one.
        let open = loop {
            match bytes.get(at) {
                Some(b'{') => break Some(at),
                Some(b';') | None => break None,
                Some(_) => at = at.saturating_add(1),
            }
        };
        let Some(open) = open else { continue };
        let mut depth = 0_usize;
        let mut close = None;
        for (index, byte) in bytes.iter().enumerate().skip(open) {
            match byte {
                b'{' => depth = depth.saturating_add(1),
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(close) = close {
            bodies.push(open.saturating_add(1)..close);
        }
    }
    bodies
}

/// `source` with comments, string and character literals replaced by spaces (newlines kept), so
/// byte offsets and line numbers are unchanged.
#[must_use]
pub fn blank_rust(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0_usize;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in out.iter_mut().take(to).skip(from) {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    while at < bytes.len() {
        let rest = bytes.get(at..).unwrap_or_default();
        if rest.starts_with(b"//") {
            let end = rest
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |n| at.saturating_add(n));
            blank(&mut out, at, end);
            at = end;
        } else if rest.starts_with(b"/*") {
            let mut depth = 0_usize;
            let mut index = at;
            while index < bytes.len() {
                let here = bytes.get(index..).unwrap_or_default();
                if here.starts_with(b"/*") {
                    depth = depth.saturating_add(1);
                    index = index.saturating_add(2);
                } else if here.starts_with(b"*/") {
                    depth = depth.saturating_sub(1);
                    index = index.saturating_add(2);
                    if depth == 0 {
                        break;
                    }
                } else {
                    index = index.saturating_add(1);
                }
            }
            blank(&mut out, at, index);
            at = index;
        } else if let Some(hashes) = raw_string_start(rest) {
            // r"..." / r#"..."#: ends at a quote followed by the same number of hashes.
            let body = at.saturating_add(hashes).saturating_add(2);
            let closing: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let end = bytes
                .get(body..)
                .and_then(|tail| tail.windows(closing.len()).position(|w| w == closing))
                .map_or(bytes.len(), |n| {
                    body.saturating_add(n).saturating_add(closing.len())
                });
            blank(&mut out, at, end);
            at = end;
        } else if rest.first() == Some(&b'"') {
            let mut index = at.saturating_add(1);
            while index < bytes.len() {
                match bytes.get(index) {
                    Some(b'\\') => index = index.saturating_add(2),
                    Some(b'"') => {
                        index = index.saturating_add(1);
                        break;
                    }
                    _ => index = index.saturating_add(1),
                }
            }
            blank(&mut out, at, index.min(bytes.len()));
            at = index;
        } else if rest.first() == Some(&b'\'') {
            // A character literal ('x', '\n', '\u{1F600}'); a lifetime has no closing quote.
            let end = if rest.get(1) == Some(&b'\\') {
                rest.iter()
                    .skip(2)
                    .position(|b| *b == b'\'')
                    .map(|n| at.saturating_add(n).saturating_add(3))
            } else {
                let width = std::str::from_utf8(rest.get(1..).unwrap_or_default())
                    .ok()
                    .and_then(|tail| tail.chars().next())
                    .map_or(1, char::len_utf8);
                (rest.get(width.saturating_add(1)) == Some(&b'\''))
                    .then(|| at.saturating_add(width).saturating_add(2))
            };
            match end {
                Some(end) => {
                    blank(&mut out, at, end);
                    at = end;
                }
                None => at = at.saturating_add(1),
            }
        } else {
            at = at.saturating_add(1);
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Number of `#` of a raw string starting here (`r"`, `r#"`, `br"`...), if one does.
fn raw_string_start(rest: &[u8]) -> Option<usize> {
    let rest = rest.strip_prefix(b"b").unwrap_or(rest);
    let after = rest.strip_prefix(b"r")?;
    let hashes = after.iter().take_while(|b| **b == b'#').count();
    (after.get(hashes) == Some(&b'"')).then_some(hashes)
}
