//! Multi-pattern scanning: one leftmost-first pass over all rules (regex-automata `meta` engine,
//! which picks a lazy DFA / one-pass / literal prefilter per pattern set and reports start and
//! end offsets).

use regex_automata::meta::{self, Regex};
use regex_automata::util::syntax;
use regex_automata::{Input, PatternID};
use regex_syntax::hir::{Hir, HirKind};

use crate::error::AutomataError;

const HARD_MAX_PATTERNS: u32 = 5000;
const HARD_MAX_PATTERN_BYTES: usize = 16 * 1024;
const HARD_MAX_NESTING: u32 = 256;
const HARD_MAX_AUTOMATA_BYTES: usize = 64 * 1024 * 1024;

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

    fn try_with_capacity(capacity: usize) -> Result<Self, AutomataError> {
        let mut matches = Vec::new();
        matches
            .try_reserve_exact(capacity)
            .map_err(|_| AutomataError::Allocation)?;
        Ok(Self { matches, capacity })
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
        match u32::try_from(self.rules.len()) {
            Ok(count) if count < HARD_MAX_PATTERNS => {}
            _ => {
                return Err(AutomataError::TooManyPatterns {
                    limit: HARD_MAX_PATTERNS,
                });
            }
        }
        if pattern.len() > HARD_MAX_PATTERN_BYTES {
            return Err(AutomataError::TooLarge {
                limit: HARD_MAX_PATTERN_BYTES,
            });
        }
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
        let pattern_limit = limits.max_patterns.min(HARD_MAX_PATTERNS);
        if count > pattern_limit {
            return Err(AutomataError::TooManyPatterns {
                limit: pattern_limit,
            });
        }
        let nest_limit = limits.nest_limit.min(HARD_MAX_NESTING);
        let nfa_size_bytes = limits.nfa_size_bytes.min(HARD_MAX_AUTOMATA_BYTES);
        let dfa_size_bytes = limits.dfa_size_bytes.min(HARD_MAX_AUTOMATA_BYTES);
        let lazy_cache_bytes = limits.lazy_cache_bytes.min(HARD_MAX_AUTOMATA_BYTES);
        let patterns: Vec<&str> = self.rules.iter().map(|(_, p, _)| p.as_str()).collect();
        let mut max_match_bytes = Some(0_usize);
        let mut stream_safe = true;
        for pattern in &patterns {
            let hir = regex_syntax::ParserBuilder::new()
                .unicode(false)
                .utf8(false)
                .nest_limit(nest_limit)
                .build()
                .parse(pattern)
                .map_err(|error| AutomataError::Syntax(error.to_string()))?;
            max_match_bytes = match (max_match_bytes, hir.properties().maximum_len()) {
                (Some(current), Some(width)) => Some(current.max(width)),
                _ => None,
            };
            stream_safe &= !has_context_assertion(&hir);
        }
        let regex = Regex::builder()
            .syntax(
                syntax::Config::new()
                    .unicode(false)
                    .utf8(false)
                    .nest_limit(nest_limit),
            )
            .configure(
                meta::Config::new()
                    .match_kind(regex_automata::MatchKind::LeftmostFirst)
                    .utf8_empty(false)
                    .nfa_size_limit(Some(nfa_size_bytes))
                    .dfa_size_limit(Some(dfa_size_bytes))
                    .hybrid_cache_capacity(lazy_cache_bytes),
            )
            .build_many(&patterns)
            .map_err(|e| build_error(&e, nfa_size_bytes.max(dfa_size_bytes)))?;
        Ok(DfaScanner {
            regex,
            max_match_bytes,
            stream_safe,
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
    if let Some(regex_syntax::Error::Parse(ast)) = e.syntax_error()
        && matches!(
            ast.kind(),
            regex_syntax::ast::ErrorKind::NestLimitExceeded(_)
        )
    {
        return AutomataError::TooNested;
    }
    AutomataError::Syntax(e.to_string())
}

/// A compiled multi-pattern scanner. Building allocates; scanning does not (after the first
/// search on a thread warms the engine's cache pool).
#[derive(Debug, Clone)]
pub struct DfaScanner {
    pub(crate) regex: Regex,
    max_match_bytes: Option<usize>,
    stream_safe: bool,
    rules: Vec<(String, Category)>,
}

/// Stateful scanner for bounded streams. It retains at most `maximum_match_width - 1` bytes
/// between chunks, so a match crossing a chunk boundary is emitted exactly once. Every rule
/// must have a finite maximum byte width and no word-boundary or anchor assertions; other rules
/// stay available through [`DfaScanner::scan`] on complete slices.
#[derive(Debug)]
pub struct StreamScanner<'a> {
    scanner: &'a DfaScanner,
    max_chunk_bytes: usize,
    overlap_bytes: usize,
    total_bytes: u32,
    started: bool,
    tail: Vec<u8>,
    scratch: Vec<u8>,
    matches: ScanBuf,
}

impl<'a> StreamScanner<'a> {
    /// Reserves the bounded scratch and internal match storage before scanning starts.
    ///
    /// # Errors
    /// Returns [`AutomataError::UnboundedMatch`] for a scanner with an unbounded rule,
    /// [`AutomataError::TooLarge`] for an invalid chunk size, or [`AutomataError::Allocation`]
    /// when the workspace cannot be reserved.
    pub fn new(
        scanner: &'a DfaScanner,
        max_chunk_bytes: usize,
        max_matches_per_chunk: usize,
    ) -> Result<Self, AutomataError> {
        if max_chunk_bytes == 0 {
            return Err(AutomataError::TooLarge { limit: 1 });
        }
        let width = scanner
            .max_match_bytes
            .ok_or(AutomataError::UnboundedMatch)?;
        if !scanner.stream_safe {
            return Err(AutomataError::UnsupportedStreamPattern);
        }
        let overlap_bytes = width.saturating_sub(1);
        let scratch_capacity = overlap_bytes
            .checked_add(max_chunk_bytes)
            .filter(|capacity| u32::try_from(*capacity).is_ok())
            .ok_or(AutomataError::TooLarge {
                limit: max_offset_limit(),
            })?;
        let mut tail = Vec::new();
        tail.try_reserve_exact(overlap_bytes)
            .map_err(|_| AutomataError::Allocation)?;
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(scratch_capacity)
            .map_err(|_| AutomataError::Allocation)?;
        Ok(Self {
            scanner,
            max_chunk_bytes,
            overlap_bytes,
            total_bytes: 0,
            started: false,
            tail,
            scratch,
            matches: ScanBuf::try_with_capacity(max_matches_per_chunk)?,
        })
    }

    /// Scans one chunk in order and writes newly completed matches with stream-global offsets.
    /// A stream should feed each chunk once; no final flush is required because a finite-width
    /// match becomes complete in the chunk containing its final byte.
    ///
    /// # Errors
    /// Returns [`AutomataError::TooLarge`] for a chunk/stream offset past its configured bounds
    /// or [`AutomataError::BufferFull`] when either bounded match buffer is too small.
    pub fn push(&mut self, chunk: &[u8], out: &mut ScanBuf) -> Result<usize, AutomataError> {
        if chunk.len() > self.max_chunk_bytes {
            return Err(AutomataError::TooLarge {
                limit: self.max_chunk_bytes,
            });
        }
        let chunk_len = u32::try_from(chunk.len()).map_err(|_| AutomataError::TooLarge {
            limit: self.max_chunk_bytes,
        })?;
        let next_total =
            self.total_bytes
                .checked_add(chunk_len)
                .ok_or(AutomataError::TooLarge {
                    limit: max_offset_limit(),
                })?;
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.tail);
        self.scratch.extend_from_slice(chunk);
        self.scanner.scan(&self.scratch, &mut self.matches)?;
        out.clear();
        let previous_bytes =
            u32::try_from(self.tail.len()).map_err(|_| AutomataError::TooLarge {
                limit: self.overlap_bytes,
            })?;
        let base = self
            .total_bytes
            .checked_sub(previous_bytes)
            .ok_or(AutomataError::TooLarge {
                limit: max_offset_limit(),
            })?;
        for matched in self.matches.matches() {
            if matched.end <= previous_bytes && !(!self.started && matched.end == 0) {
                continue;
            }
            let start = base
                .checked_add(matched.start)
                .ok_or(AutomataError::TooLarge {
                    limit: max_offset_limit(),
                })?;
            let end = base
                .checked_add(matched.end)
                .ok_or(AutomataError::TooLarge {
                    limit: max_offset_limit(),
                })?;
            out.push(Match {
                rule: matched.rule,
                start,
                end,
            })?;
        }
        let keep = self.overlap_bytes.min(self.scratch.len());
        let tail_start = self.scratch.len().saturating_sub(keep);
        self.tail.clear();
        self.tail
            .extend_from_slice(
                self.scratch
                    .get(tail_start..)
                    .ok_or(AutomataError::TooLarge {
                        limit: self.scratch.len(),
                    })?,
            );
        self.total_bytes = next_total;
        self.started = true;
        Ok(out.len())
    }
}

fn has_context_assertion(hir: &Hir) -> bool {
    let mut stack = vec![hir];
    while let Some(node) = stack.pop() {
        match node.kind() {
            HirKind::Look(_) => return true,
            HirKind::Repetition(repetition) => stack.push(&repetition.sub),
            HirKind::Capture(capture) => stack.push(&capture.sub),
            HirKind::Concat(parts) | HirKind::Alternation(parts) => stack.extend(parts.iter()),
            HirKind::Empty | HirKind::Literal(_) | HirKind::Class(_) => {}
        }
    }
    false
}

/// `start..end` as `u32` offsets; haystacks are limited to `u32::MAX` bytes.
fn offsets(m: &regex_automata::Match) -> Option<(u32, u32)> {
    Some((u32::try_from(m.start()).ok()?, u32::try_from(m.end()).ok()?))
}

fn max_offset_limit() -> usize {
    match usize::try_from(u32::MAX) {
        Ok(limit) => limit,
        Err(_) => usize::MAX,
    }
}

pub(crate) fn check_len(haystack: &[u8]) -> Result<(), AutomataError> {
    if u32::try_from(haystack.len()).is_err() {
        return Err(AutomataError::TooLarge {
            limit: max_offset_limit(),
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

    /// Estimated heap memory used by the compiled regex engine.
    ///
    /// This reports the underlying `regex-automata` meta engine's accounting; it does not
    /// include rule-name/category metadata or caller-owned scan buffers.
    #[must_use]
    pub fn engine_memory_bytes(&self) -> usize {
        self.regex.memory_usage()
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
