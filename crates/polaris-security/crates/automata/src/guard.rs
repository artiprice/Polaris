//! Guard for user-supplied regexes. Rust's engines run in linear time (no backtracking), so the
//! exploitable axis is compile-time size: long patterns, deep nesting, huge counted repetitions
//! and large Unicode classes. The guard checks the HIR structurally, then compiles under a size
//! limit as the final proof.

use regex_automata::meta::{self, Regex};
use regex_automata::util::syntax;
use regex_syntax::hir::{Class, Hir, HirKind};

use crate::error::AutomataError;
use crate::scan::build_error;

const HARD_MAX_USER_PATTERN_BYTES: usize = 16 * 1024;
const HARD_MAX_USER_NESTING: u32 = 128;
const HARD_MAX_USER_REPETITION: u32 = 1_000_000;
const HARD_MAX_CLASS_RANGES: usize = 4096;
const HARD_MAX_NFA_BYTES: usize = 16 * 1024 * 1024;

/// Bounds for user regexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserRegexLimits {
    /// Maximum pattern length in bytes.
    pub max_len: usize,
    /// Maximum group nesting depth.
    pub nest_limit: u32,
    /// Maximum product of nested counted repetitions (`(a{50}){50}` counts 2500).
    pub max_repetition: u32,
    /// Maximum number of ranges in one character class.
    pub max_class_ranges: usize,
    /// Maximum compiled NFA size in bytes.
    pub nfa_size_bytes: usize,
}

impl Default for UserRegexLimits {
    fn default() -> Self {
        Self {
            max_len: 1024,
            nest_limit: 32,
            max_repetition: 1000,
            max_class_ranges: 4096,
            nfa_size_bytes: 1 << 20,
        }
    }
}

/// A user pattern proven to compile within [`UserRegexLimits`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPattern(String);

impl ValidatedPattern {
    /// The pattern text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validates a user regex (Unicode mode, as used on text).
///
/// # Errors
/// [`AutomataError::TooLarge`], [`AutomataError::TooNested`],
/// [`AutomataError::RepetitionTooLarge`] or [`AutomataError::Syntax`].
pub fn validate_user_regex(
    pattern: &str,
    limits: &UserRegexLimits,
) -> Result<ValidatedPattern, AutomataError> {
    let effective = UserRegexLimits {
        max_len: limits.max_len.min(HARD_MAX_USER_PATTERN_BYTES),
        nest_limit: limits.nest_limit.min(HARD_MAX_USER_NESTING),
        max_repetition: limits.max_repetition.min(HARD_MAX_USER_REPETITION),
        max_class_ranges: limits.max_class_ranges.min(HARD_MAX_CLASS_RANGES),
        nfa_size_bytes: limits.nfa_size_bytes.min(HARD_MAX_NFA_BYTES),
    };
    if pattern.len() > effective.max_len {
        return Err(AutomataError::TooLarge {
            limit: effective.max_len,
        });
    }
    let hir = regex_syntax::ParserBuilder::new()
        .nest_limit(effective.nest_limit)
        .build()
        .parse(pattern)
        .map_err(|e| match &e {
            regex_syntax::Error::Parse(ast)
                if matches!(
                    ast.kind(),
                    regex_syntax::ast::ErrorKind::NestLimitExceeded(_)
                ) =>
            {
                AutomataError::TooNested
            }
            _ => AutomataError::Syntax(e.to_string()),
        })?;
    check_structure(&hir, &effective)?;
    Regex::builder()
        .syntax(syntax::Config::new().nest_limit(effective.nest_limit))
        .configure(meta::Config::new().nfa_size_limit(Some(effective.nfa_size_bytes)))
        .build(pattern)
        .map_err(|e| build_error(&e, effective.nfa_size_bytes))?;
    Ok(ValidatedPattern(pattern.to_owned()))
}

/// Iterative walk (no recursion on hostile input) carrying the repetition product of each path.
fn check_structure(hir: &Hir, limits: &UserRegexLimits) -> Result<(), AutomataError> {
    let mut stack: Vec<(&Hir, u64)> = vec![(hir, 1)];
    while let Some((node, product)) = stack.pop() {
        match node.kind() {
            HirKind::Empty | HirKind::Literal(_) | HirKind::Look(_) => {}
            HirKind::Class(class) => {
                let ranges = match class {
                    Class::Unicode(c) => c.ranges().len(),
                    Class::Bytes(c) => c.ranges().len(),
                };
                if ranges > limits.max_class_ranges {
                    return Err(AutomataError::TooLarge {
                        limit: limits.max_class_ranges,
                    });
                }
            }
            HirKind::Repetition(rep) => {
                let maximum = match rep.max {
                    Some(maximum) => maximum,
                    None => rep.min,
                };
                let count = u64::from(maximum.max(1));
                let next = product.saturating_mul(count);
                if next > u64::from(limits.max_repetition) {
                    return Err(AutomataError::RepetitionTooLarge {
                        limit: limits.max_repetition,
                    });
                }
                stack.push((&rep.sub, next));
            }
            HirKind::Capture(cap) => stack.push((&cap.sub, product)),
            HirKind::Concat(subs) | HirKind::Alternation(subs) => {
                stack.extend(subs.iter().map(|s| (s, product)));
            }
        }
    }
    Ok(())
}
