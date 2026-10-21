//! Guard for user-supplied regexes. Rust's engines run in linear time (no backtracking), so the
//! exploitable axis is compile-time size: long patterns, deep nesting, huge counted repetitions
//! and large Unicode classes. The guard checks the HIR structurally, then compiles under a size
//! limit as the final proof.

use regex_automata::meta::{self, Regex};
use regex_automata::util::syntax;
use regex_syntax::hir::{Class, Hir, HirKind};

use crate::error::AutomataError;
use crate::scan::build_error;

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
    if pattern.len() > limits.max_len {
        return Err(AutomataError::TooLarge {
            limit: limits.max_len,
        });
    }
    let hir = regex_syntax::ParserBuilder::new()
        .nest_limit(limits.nest_limit)
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
    check_structure(&hir, limits)?;
    Regex::builder()
        .syntax(syntax::Config::new().nest_limit(limits.nest_limit))
        .configure(meta::Config::new().nfa_size_limit(Some(limits.nfa_size_bytes)))
        .build(pattern)
        .map_err(|e| build_error(&e, limits.nfa_size_bytes))?;
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
                let count = u64::from(rep.max.unwrap_or(rep.min).max(1));
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
