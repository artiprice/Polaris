//! Multi-pattern scanning: one leftmost-first pass over all rules (regex-automata `meta` engine,
//! which picks a lazy DFA / one-pass / literal prefilter per pattern set and reports start and
//! end offsets).

use regex_automata::meta::{self, Regex};
use regex_automata::util::syntax;
use regex_automata::{Input, PatternID};

use crate::error::AutomataError;

/// Rule index inside one scanner (or [`RuleId::ENTROPY`] for entropy candidates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId(pub u32);

impl RuleId {
    /// Marks spans found by [`crate::high_entropy_spans`].
    pub const ENTROPY: Self = Self(u32::MAX);
}

/// What a rule detects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Credentials and keys (redacted).
    Secret,
    /// Personal data (redacted).
    Pii,
    /// Destructive shell commands (flagged for the safety gate, not redacted).
    DangerousShell,
}

/// A match: byte offsets `start..end` into the haystack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// Rule that matched.
    pub rule: RuleId,
    /// Start offset (inclusive).
    pub start: u32,
    /// End offset (exclusive).
    pub end: u32,
}

/// Caller-owned match storage with a fixed capacity (never grows on the scan path).
#[derive(Debug, Clone)]
pub struct ScanBuf {
    matches: Vec<Match>,
    capacity: usize,
}

impl ScanBuf {
    /// Room for exactly `capacity` matches.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            matches: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Matches of the last scan, ordered by start offset.
    #[must_use]
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    /// Number of stored matches.
    #[must_use]
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// `true` when no match is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// The fixed capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Removes all matches.
    pub fn clear(&mut self) {
        self.matches.clear();
    }

    pub(crate) fn push(&mut self, m: Match) -> Result<(), AutomataError> {
        if self.matches.len() >= self.capacity {
            return Err(AutomataError::BufferFull {
                capacity: self.capacity,
            });
        }
        self.matches.push(m);
        Ok(())
    }
}

/// Compilation bounds for a rule set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileLimits {
    /// Maximum number of patterns.
    pub max_patterns: u32,
    /// Maximum group nesting depth.
    pub nest_limit: u32,
    /// Maximum compiled NFA size in bytes.
    pub nfa_size_bytes: usize,
    /// Maximum fully compiled DFA size in bytes (larger sets use the lazy DFA).
    pub dfa_size_bytes: usize,
    /// Lazy-DFA cache capacity in bytes.
    pub lazy_cache_bytes: usize,
}

impl Default for CompileLimits {
    fn default() -> Self {
        Self {
            max_patterns: 1000,
            nest_limit: 64,
            nfa_size_bytes: 10 << 20,
            dfa_size_bytes: 4 << 20,
            lazy_cache_bytes: 2 << 20,
        }
    }
}

/// Collects named patterns before compilation.
#[derive(Debug, Clone, Default)]
pub struct RuleSetBuilder {
    rules: Vec<(String, String, Category)>,
}

impl RuleSetBuilder {
    /// Adds a rule. Syntax is checked at [`RuleSetBuilder::build`].
    ///
    /// # Errors
    /// [`AutomataError::DuplicateRule`] when the name is already used.
    pub fn add(
        &mut self,
        name: &str,
        pattern: &str,
        category: Category,
    ) -> Result<&mut Self, AutomataError> {
        if self.rules.iter().any(|(n, _, _)| n == name) {
            return Err(AutomataError::DuplicateRule(name.to_owned()));
        }
        self.rules
            .push((name.to_owned(), pattern.to_owned(), category));
        Ok(self)
    }

    /// Compiles every rule into one scanner. Patterns run in ASCII byte mode: `\b`, `\w` and
    /// classes are ASCII, and haystacks need not be UTF-8.
    ///
    /// # Errors
    /// [`AutomataError::TooManyPatterns`], [`AutomataError::TooNested`],
    /// [`AutomataError::TooLarge`] or [`AutomataError::Syntax`].
    pub fn build(&self, limits: &CompileLimits) -> Result<DfaScanner, AutomataError> {
        let count = u32::try_from(self.rules.len()).unwrap_or(u32::MAX);
        if count > limits.max_patterns {
            return Err(AutomataError::TooManyPatterns {
                limit: limits.max_patterns,
            });
        }
        let patterns: Vec<&str> = self.rules.iter().map(|(_, p, _)| p.as_str()).collect();
        let regex = Regex::builder()
            .syntax(
                syntax::Config::new()
                    .unicode(false)
                    .utf8(false)
                    .nest_limit(limits.nest_limit),
            )
            .configure(
                meta::Config::new()
                    .match_kind(regex_automata::MatchKind::LeftmostFirst)
                    .utf8_empty(false)
                    .nfa_size_limit(Some(limits.nfa_size_bytes))
                    .dfa_size_limit(Some(limits.dfa_size_bytes))
                    .hybrid_cache_capacity(limits.lazy_cache_bytes),
            )
            .build_many(&patterns)
            .map_err(|e| build_error(&e, limits.nfa_size_bytes))?;
        Ok(DfaScanner {
            regex,
            rules: self
                .rules
                .iter()
                .map(|(name, _, cat)| (name.clone(), *cat))
                .collect(),
        })
    }
}

pub(crate) fn build_error(e: &meta::BuildError, size_limit: usize) -> AutomataError {
    if e.size_limit().is_some() {
        return AutomataError::TooLarge { limit: size_limit };
    }
    if let Some(regex_syntax::Error::Parse(ast)) = e.syntax_error() {
        if matches!(
            ast.kind(),
            regex_syntax::ast::ErrorKind::NestLimitExceeded(_)
        ) {
            return AutomataError::TooNested;
        }
    }
    AutomataError::Syntax(e.to_string())
}

/// A compiled multi-pattern scanner. Building allocates; scanning does not (after the first
/// search on a thread warms the engine's cache pool).
#[derive(Debug, Clone)]
pub struct DfaScanner {
    pub(crate) regex: Regex,
    rules: Vec<(String, Category)>,
}

/// `start..end` as `u32` offsets; haystacks are limited to `u32::MAX` bytes.
fn offsets(m: &regex_automata::Match) -> Option<(u32, u32)> {
    Some((u32::try_from(m.start()).ok()?, u32::try_from(m.end()).ok()?))
}

pub(crate) fn check_len(haystack: &[u8]) -> Result<(), AutomataError> {
    if u32::try_from(haystack.len()).is_err() {
        return Err(AutomataError::TooLarge {
            limit: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}

pub(crate) fn rule_of(pattern: PatternID) -> RuleId {
    RuleId(pattern.as_u32())
}

impl DfaScanner {
    /// A builder for a custom rule set.
    #[must_use]
    pub fn builder() -> RuleSetBuilder {
        RuleSetBuilder::default()
    }

    /// Finds every leftmost-first, non-overlapping match, ordered by offset. `out` is cleared
    /// first.
    ///
    /// # Errors
    /// [`AutomataError::BufferFull`] when `out` is too small; [`AutomataError::TooLarge`] for a
    /// haystack of 4 GiB or more.
    pub fn scan(&self, haystack: &[u8], out: &mut ScanBuf) -> Result<usize, AutomataError> {
        check_len(haystack)?;
        out.clear();
        for m in self.regex.find_iter(Input::new(haystack)) {
            if let Some((start, end)) = offsets(&m) {
                out.push(Match {
                    rule: rule_of(m.pattern()),
                    start,
                    end,
                })?;
            }
        }
        Ok(out.len())
    }

    /// `true` if any rule matches.
    ///
    /// # Errors
    /// [`AutomataError::TooLarge`] for a haystack of 4 GiB or more.
    pub fn any_match(&self, haystack: &[u8]) -> Result<bool, AutomataError> {
        check_len(haystack)?;
        Ok(self.regex.is_match(Input::new(haystack)))
    }

    /// Number of rules.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// The rule's name.
    #[must_use]
    pub fn rule_name(&self, rule: RuleId) -> Option<&str> {
        if rule == RuleId::ENTROPY {
            return Some("high_entropy");
        }
        self.rules
            .get(usize::try_from(rule.0).ok()?)
            .map(|(n, _)| n.as_str())
    }

    /// The rule's category.
    #[must_use]
    pub fn rule_category(&self, rule: RuleId) -> Option<Category> {
        if rule == RuleId::ENTROPY {
            return Some(Category::Secret);
        }
        self.rules
            .get(usize::try_from(rule.0).ok()?)
            .map(|(_, c)| *c)
    }
}
