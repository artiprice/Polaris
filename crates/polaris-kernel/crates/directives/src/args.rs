//! The argument grammar inside `$name( … )`:
//! `arg = kv | call | quoted | atom-sequence`, with depth and count limits.

use crate::cursor::Cursor;
use crate::error::DirectiveError;
use crate::parser::ParseLimits;

/// One directive argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Arg {
    /// Unquoted text; consecutive words separated by whitespace are joined with one space.
    Atom(String),
    /// Quoted text (escapes `\\`, `\"`, `\'` only; any other backslash is literal).
    Quoted(String),
    /// `key = value` (key is a lowercase identifier).
    Kv {
        /// The key.
        key: String,
        /// The value (atom, quoted or call; never another kv).
        value: Box<Arg>,
    },
    /// `name(args…)` — nested calls and `$subagent-models` entries such as `nano(1-3)`.
    Call {
        /// Callee text.
        name: String,
        /// `true` when the callee was quoted (`"nvidia/x"(2)`).
        quoted: bool,
        /// Arguments.
        args: Vec<Arg>,
    },
}

impl Arg {
    /// The text of an atom or quoted argument.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Atom(t) | Self::Quoted(t) => Some(t),
            Self::Kv { .. } | Self::Call { .. } => None,
        }
    }
}

/// Characters allowed in an atom.
pub(crate) fn is_atom_char(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, ',' | '(' | ')' | '"' | '\''))
}

/// First character of an identifier (`kv` keys, call names).
pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_lowercase() || c == '_'
}

/// Subsequent identifier characters.
pub(crate) fn is_ident_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-')
}

/// Parses quoted text; the cursor is on the opening quote.
pub(crate) fn quoted(cur: &mut Cursor<'_>) -> Result<String, DirectiveError> {
    let at = cur.offset();
    let quote = cur.bump().ok_or(DirectiveError::UnterminatedQuote { at })?;
    let mut out = String::new();
    loop {
        match cur.bump() {
            None => return Err(DirectiveError::UnterminatedQuote { at }),
            Some('\\') => match cur.peek() {
                Some(next @ ('\\' | '"' | '\'')) => {
                    cur.bump();
                    out.push(next);
                }
                _ => out.push('\\'),
            },
            Some(c) if c == quote => return Ok(out),
            Some(c) => out.push(c),
        }
    }
}

/// Recursive-descent argument parser with limits.
pub(crate) struct ArgParser<'a, 'l> {
    pub(crate) cur: Cursor<'a>,
    limits: &'l ParseLimits,
    count: u16,
}

impl<'a, 'l> ArgParser<'a, 'l> {
    pub(crate) const fn new(cur: Cursor<'a>, limits: &'l ParseLimits) -> Self {
        Self {
            cur,
            limits,
            count: 0,
        }
    }

    fn counted(&mut self, arg: Arg) -> Result<Arg, DirectiveError> {
        self.count = self.count.saturating_add(1);
        if self.count > self.limits.max_args {
            return Err(DirectiveError::TooManyArgs);
        }
        Ok(arg)
    }

    /// Parses `args… )` after an opening `(` at `open_at`, at nesting `depth` (1 = directive level).
    pub(crate) fn parenthesised(
        &mut self,
        depth: u8,
        open_at: usize,
    ) -> Result<Vec<Arg>, DirectiveError> {
        if depth > self.limits.max_depth {
            return Err(DirectiveError::TooDeep);
        }
        let mut args = Vec::new();
        self.cur.skip_ws();
        if self.cur.eat(')') {
            return Ok(args);
        }
        loop {
            let arg = self.arg(depth, true)?;
            args.push(arg);
            self.cur.skip_ws();
            match self.cur.bump() {
                Some(',') => self.cur.skip_ws(),
                Some(')') => return Ok(args),
                None => return Err(DirectiveError::UnbalancedParen { at: open_at }),
                Some(_) => return Err(DirectiveError::UnbalancedParen { at: open_at }),
            }
        }
    }

    /// Parses a whole string as an argument list (no surrounding parentheses).
    pub(crate) fn bare_list(&mut self) -> Result<Vec<Arg>, DirectiveError> {
        let mut args = Vec::new();
        self.cur.skip_ws();
        if self.cur.at_end() {
            return Ok(args);
        }
        loop {
            let at = self.cur.offset();
            let arg = self.arg(1, true)?;
            args.push(arg);
            self.cur.skip_ws();
            match self.cur.bump() {
                Some(',') => self.cur.skip_ws(),
                None => return Ok(args),
                Some(_) => return Err(DirectiveError::UnbalancedParen { at }),
            }
        }
    }

    /// One argument. `allow_kv` is false for kv values (no `a=b=c`).
    fn arg(&mut self, depth: u8, allow_kv: bool) -> Result<Arg, DirectiveError> {
        self.cur.skip_ws();
        let start = self.cur.pos();
        match self.cur.peek() {
            Some('"' | '\'') => {
                let text = quoted(&mut self.cur)?;
                self.after_callee(text, true, depth)
            }
            Some(c) if allow_kv && is_ident_start(c) => {
                // `key =` is tried first; anything else (`c:\dir`, `https://…`, `nano(1-3)`) re-reads as
                // an atom or call from the same start.
                let key = self.cur.take_while(is_ident_char);
                self.cur.skip_ws();
                if self.cur.eat('=') {
                    let value = self.arg(depth, false)?;
                    let kv = Arg::Kv {
                        key: key.to_owned(),
                        value: Box::new(value),
                    };
                    return self.counted(kv);
                }
                self.cur.reset(start);
                self.atom_or_call(depth)
            }
            Some(c) if is_atom_char(c) => self.atom_or_call(depth),
            _ => Err(DirectiveError::UnbalancedParen {
                at: self.cur.offset(),
            }),
        }
    }

    fn after_callee(
        &mut self,
        name: String,
        quoted: bool,
        depth: u8,
    ) -> Result<Arg, DirectiveError> {
        let open_at = self.cur.offset();
        if self.cur.eat('(') {
            let args = self.parenthesised(depth.saturating_add(1), open_at)?;
            return self.counted(Arg::Call { name, quoted, args });
        }
        if quoted {
            self.counted(Arg::Quoted(name))
        } else {
            self.counted(Arg::Atom(name))
        }
    }

    fn atom_or_call(&mut self, depth: u8) -> Result<Arg, DirectiveError> {
        let first = self.cur.take_while(is_atom_char).to_owned();
        if self.cur.peek() == Some('(') {
            return self.after_callee(first, false, depth);
        }
        let mut text = first;
        loop {
            let save = self.cur.pos();
            self.cur.skip_ws();
            match self.cur.peek() {
                Some(c) if is_atom_char(c) => {
                    let word = self.cur.take_while(is_atom_char);
                    if self.cur.peek() == Some('(') {
                        self.cur.reset(save);
                        break;
                    }
                    text.push(' ');
                    text.push_str(word);
                }
                _ => {
                    self.cur.reset(save);
                    break;
                }
            }
        }
        self.counted(Arg::Atom(text))
    }
}
