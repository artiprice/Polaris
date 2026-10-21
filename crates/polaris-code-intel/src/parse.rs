//! Parsing: the [`ParserPool`], the [`Parsed`] handle and byte spans.

use crate::error::CodeIntelError;
use crate::lang::{BOM, Lang, detect_lang};
use crate::walk::{Descend, walk_preorder};
use polaris_sdk::ids::FastFingerprint;
use std::borrow::Cow;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tree_sitter::{InputEdit, ParseOptions, ParseState, Parser, Point, Tree};
use xxhash_rust::xxh3::Xxh3;

/// Most idle parsers a pool keeps; extra ones are dropped after use.
const MAX_IDLE_PARSERS: usize = 64;

/// A half-open byte range `start..end` into the original source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteSpan {
    /// First byte of the range.
    pub start: usize,
    /// One past the last byte of the range.
    pub end: usize,
}

impl ByteSpan {
    /// Creates a span; `end` is raised to `start` if it is smaller.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end: if end < start { start } else { end },
        }
    }

    /// Length of the range in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// The bytes of `src` covered by this span, or `None` if it lies outside `src`.
    #[must_use]
    pub fn slice<'a>(&self, src: &'a [u8]) -> Option<&'a [u8]> {
        src.get(self.start..self.end)
    }

    /// The text of `src` covered by this span, or `None` if it is out of range or not valid UTF-8.
    #[must_use]
    pub fn text<'a>(&self, src: &'a [u8]) -> Option<&'a str> {
        std::str::from_utf8(self.slice(src)?).ok()
    }
}

/// Resource limits applied to every parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// Largest accepted source in bytes; larger inputs return [`CodeIntelError::TooLarge`].
    pub max_source_bytes: usize,
    /// Wall-clock budget of one parse; exceeding it returns [`CodeIntelError::ParseTimeout`].
    pub timeout: Duration,
}

impl Default for ParseLimits {
    /// 16 MiB and 5 seconds (the time budget is a **guess**; tune against real repositories).
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024 * 1024,
            timeout: Duration::from_secs(5),
        }
    }
}

/// A parsed source file: the syntax tree plus the exact original bytes.
///
/// All spans produced from a `Parsed` are byte offsets into [`Parsed::source`], including when the
/// source starts with a UTF-8 BOM or uses CRLF line endings.
#[derive(Debug, Clone)]
pub struct Parsed {
    lang: Lang,
    source: Vec<u8>,
    tree: Tree,
}

impl Parsed {
    /// The language the source was parsed as.
    #[must_use]
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// The original source bytes, unmodified.
    #[must_use]
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// Whether the tree contains syntax errors or missing tokens.
    #[must_use]
    pub fn has_syntax_error(&self) -> bool {
        self.tree.root_node().has_error()
    }

    /// Total number of nodes in the tree (named and anonymous).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.tree.root_node().descendant_count()
    }

    /// The span of the root node.
    #[must_use]
    pub fn root_span(&self) -> ByteSpan {
        let root = self.tree.root_node();
        ByteSpan::new(root.start_byte(), root.end_byte())
    }

    /// An xxh3 digest of the tree shape (node kinds and byte ranges).
    ///
    /// Computed with an iterative walk, so it is safe on arbitrarily deep trees. A dirty-check
    /// aid only: it is not collision resistant and must never be used as a security identity.
    #[must_use]
    pub fn tree_fingerprint(&self) -> FastFingerprint {
        let mut hasher = Xxh3::new();
        walk_preorder(self.tree.root_node(), |node, depth| {
            hasher.update(&node.kind_id().to_le_bytes());
            hasher.update(&(node.start_byte() as u64).to_le_bytes());
            hasher.update(&(node.end_byte() as u64).to_le_bytes());
            hasher.update(&depth.to_le_bytes());
            Descend::Into
        });
        FastFingerprint::new(hasher.digest())
    }

    pub(crate) fn tree(&self) -> &Tree {
        &self.tree
    }
}

/// A pool of reusable tree-sitter parsers, safe to share between threads.
///
/// A tree-sitter `Parser` needs `&mut` access, so the pool hands each caller its own parser for
/// the duration of one parse and takes it back afterwards; concurrent parses never contend on a
/// parser, only briefly on the pool's bookkeeping.
pub struct ParserPool {
    limits: ParseLimits,
    idle: Mutex<Vec<(Lang, Parser)>>,
}

impl std::fmt::Debug for ParserPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserPool")
            .field("limits", &self.limits)
            .field("idle_parsers", &self.idle().len())
            .finish()
    }
}

impl Default for ParserPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserPool {
    /// A pool with the default [`ParseLimits`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(ParseLimits::default())
    }

    /// A pool with explicit limits.
    #[must_use]
    pub fn with_limits(limits: ParseLimits) -> Self {
        Self {
            limits,
            idle: Mutex::new(Vec::new()),
        }
    }

    /// The limits applied by this pool.
    #[must_use]
    pub fn limits(&self) -> ParseLimits {
        self.limits
    }

    /// Detects the language of `path` (see [`detect_lang`]) and parses `src` with it.
    ///
    /// # Errors
    /// [`CodeIntelError::UnsupportedLanguage`] if the file is not a supported language, otherwise
    /// the errors of [`ParserPool::parse`].
    pub fn parse_file(&self, path: &Path, src: &[u8]) -> Result<Parsed, CodeIntelError> {
        let head = src.get(..src.len().min(256)).unwrap_or(src);
        let lang = detect_lang(path, head).ok_or(CodeIntelError::UnsupportedLanguage)?;
        self.parse(lang, src, None)
    }

    /// Parses `src` as `lang`.
    ///
    /// With `prev` (a parse of an earlier revision of the same file in the same language) the
    /// previous tree is reused: the changed range is found by comparing the two texts, so no edit
    /// bookkeeping is needed from the caller. A `prev` of another language is ignored.
    ///
    /// The source is not validated as UTF-8. A leading UTF-8 BOM is skipped by the grammar but
    /// all offsets still refer to the original bytes.
    ///
    /// # Errors
    /// [`CodeIntelError::TooLarge`] above [`ParseLimits::max_source_bytes`],
    /// [`CodeIntelError::ParseTimeout`] when the time budget is exceeded,
    /// [`CodeIntelError::ParseFailed`] if no tree was produced and
    /// [`CodeIntelError::GrammarVersionMismatch`] if the grammar cannot be loaded.
    pub fn parse(
        &self,
        lang: Lang,
        src: &[u8],
        prev: Option<&Parsed>,
    ) -> Result<Parsed, CodeIntelError> {
        if src.len() > self.limits.max_source_bytes {
            return Err(CodeIntelError::TooLarge {
                limit: self.limits.max_source_bytes,
            });
        }
        let buffer = parse_buffer(src);
        let old_tree = prev
            .filter(|p| p.lang == lang)
            .map(|p| reusable_tree(p, &buffer));
        let mut parser = self.checkout(lang)?;
        let deadline = Instant::now().checked_add(self.limits.timeout);
        let mut timed_out = false;
        let mut progress = |_: &ParseState| {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                timed_out = true;
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let bytes: &[u8] = &buffer;
        let tree = parser.parse_with_options(
            &mut |offset, _| bytes.get(offset..).unwrap_or_default(),
            old_tree.as_ref(),
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        let Some(tree) = tree else {
            // A cancelled parse would otherwise be resumed by the next call on this parser.
            parser.reset();
            self.checkin(lang, parser);
            return Err(if timed_out {
                CodeIntelError::ParseTimeout
            } else {
                CodeIntelError::ParseFailed
            });
        };
        self.checkin(lang, parser);
        Ok(Parsed {
            lang,
            source: src.to_vec(),
            tree,
        })
    }

    fn idle(&self) -> MutexGuard<'_, Vec<(Lang, Parser)>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn checkout(&self, lang: Lang) -> Result<Parser, CodeIntelError> {
        {
            let mut idle = self.idle();
            if let Some(pos) = idle.iter().position(|(l, _)| *l == lang) {
                return Ok(idle.swap_remove(pos).1);
            }
        }
        let mut parser = Parser::new();
        parser
            .set_language(&lang.grammar())
            .map_err(|e| CodeIntelError::GrammarVersionMismatch(e.to_string()))?;
        Ok(parser)
    }

    fn checkin(&self, lang: Lang, parser: Parser) {
        let mut idle = self.idle();
        if idle.len() < MAX_IDLE_PARSERS {
            idle.push((lang, parser));
        }
    }
}

/// The bytes handed to tree-sitter: the source, with a leading BOM replaced by spaces so every
/// offset in the tree equals the offset in the original text.
fn parse_buffer(src: &[u8]) -> Cow<'_, [u8]> {
    match src.strip_prefix(BOM) {
        Some(rest) => {
            let mut buf = Vec::with_capacity(src.len());
            buf.extend_from_slice(&[b' '; 3]);
            buf.extend_from_slice(rest);
            Cow::Owned(buf)
        }
        None => Cow::Borrowed(src),
    }
}

/// A copy of the previous tree, edited to describe the change to `new_buffer`.
fn reusable_tree(prev: &Parsed, new_buffer: &[u8]) -> Tree {
    let mut tree = prev.tree.clone();
    let old_buffer = parse_buffer(&prev.source);
    if let Some(edit) = input_edit(&old_buffer, new_buffer) {
        tree.edit(&edit);
    }
    tree
}

/// The single edit that turns `old` into `new`, found by trimming the common prefix and suffix.
fn input_edit(old: &[u8], new: &[u8]) -> Option<InputEdit> {
    if old == new {
        return None;
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = old.len().saturating_sub(suffix);
    let new_end = new.len().saturating_sub(suffix);
    Some(InputEdit {
        start_byte: prefix,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old, prefix),
        old_end_position: point_at(old, old_end),
        new_end_position: point_at(new, new_end),
    })
}

/// The tree-sitter row/column (column in bytes) of byte offset `byte` in `text`.
fn point_at(text: &[u8], byte: usize) -> Point {
    let before = text.get(..byte).unwrap_or(text);
    let row = before.iter().filter(|&&b| b == b'\n').count();
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    Point::new(row, before.len().saturating_sub(line_start))
}
