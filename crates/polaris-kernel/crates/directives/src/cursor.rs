//! A forward-only cursor over `&str`: checked slicing only, never indexing, never panics.

/// Position in a source string; `base` is added to every reported offset.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor<'a> {
    src: &'a str,
    pos: usize,
    base: usize,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `src`, reporting offsets shifted by `base`.
    pub(crate) const fn new(src: &'a str, base: usize) -> Self {
        Self { src, pos: 0, base }
    }

    /// Absolute offset of the current position.
    pub(crate) const fn offset(&self) -> usize {
        self.base + self.pos
    }

    /// Position relative to `src`.
    pub(crate) const fn pos(&self) -> usize {
        self.pos
    }

    /// The unread remainder.
    pub(crate) fn rest(&self) -> &'a str {
        self.src.get(self.pos..).unwrap_or("")
    }

    /// `src[from..to]` when both ends are char boundaries.
    pub(crate) fn slice(&self, from: usize, to: usize) -> &'a str {
        self.src.get(from..to).unwrap_or("")
    }

    /// The next char without consuming it.
    pub(crate) fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// The char after the next one.
    pub(crate) fn peek2(&self) -> Option<char> {
        let mut chars = self.rest().chars();
        chars.next();
        chars.next()
    }

    /// Consumes and returns the next char.
    pub(crate) fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// Consumes `c` if it is next.
    pub(crate) fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    /// Consumes `prefix` if the remainder starts with it.
    pub(crate) fn eat_str(&mut self, prefix: &str) -> bool {
        if self.rest().starts_with(prefix) {
            self.pos += prefix.len();
            true
        } else {
            false
        }
    }

    /// Consumes chars while `pred` holds; returns the consumed slice.
    pub(crate) fn take_while(&mut self, pred: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if !pred(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        self.slice(start, self.pos)
    }

    /// Skips any whitespace (space, tab, CR, LF, other Unicode whitespace).
    pub(crate) fn skip_ws(&mut self) {
        self.take_while(char::is_whitespace);
    }

    /// `true` when nothing is left.
    pub(crate) fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Moves to `pos` (a value previously returned by [`Self::pos`]).
    pub(crate) fn reset(&mut self, pos: usize) {
        self.pos = pos;
    }
}
