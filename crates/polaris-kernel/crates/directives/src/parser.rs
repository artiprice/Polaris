//! Top-level scanner: slash command line, or prose with `$directives`, `@mentions`, code spans and
//! fences. Total (never panics), linear in the input, bounded nesting.

use core::ops::Range;

use crate::args::ArgParser;
use crate::cursor::Cursor;
use crate::directive::{SessionDirective, typed};
use crate::error::DirectiveError;
use crate::mention::{self, Mention};
use crate::slash::{self, SlashCommand};

/// Resource limits applied before any recursion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// Largest accepted input in bytes.
    pub max_input_bytes: usize,
    /// Deepest parenthesis nesting inside a directive.
    pub max_depth: u8,
    /// Most arguments (including nested ones) in one directive.
    pub max_args: u16,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024,
            max_depth: 8,
            max_args: 64,
        }
    }
}

/// A value with its byte span in the input (both ends on char boundaries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Byte range in the input.
    pub span: Range<usize>,
}

/// A piece of a composed message: typed text is parsed, attachments are copied verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment<'a> {
    /// Text the user typed.
    Typed(&'a str),
    /// Pasted or attached content; never parsed.
    Attachment(&'a str),
}

/// Result of parsing one message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedInput {
    /// The slash command, when the message is a command line.
    pub command: Option<SlashCommand>,
    /// Directives in order of appearance.
    pub directives: Vec<Spanned<SessionDirective>>,
    /// Mentions in order of appearance (they also remain in `prose`).
    pub mentions: Vec<Spanned<Mention>>,
    /// The message without directives, trimmed.
    pub prose: String,
}

/// `$` and `@` only start a token after the start of input, whitespace, `(` or `[`.
fn is_boundary(prev: Option<char>) -> bool {
    prev.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '['))
}

fn is_dname_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_')
}

/// Appends a code span or fence starting at the cursor (on a backtick) to `prose`.
fn code(cur: &mut Cursor<'_>, prose: &mut String) {
    let start = cur.pos();
    let run = cur.take_while(|c| c == '`');
    let fence = if run.len() >= 3 { "```" } else { run };
    let close = cur
        .rest()
        .find(fence)
        .map_or(cur.rest().len(), |i| i + fence.len());
    cur.reset(cur.pos() + close);
    prose.push_str(cur.slice(start, cur.pos()));
}

struct Scanner<'l> {
    limits: &'l ParseLimits,
    out: ParsedInput,
}

impl Scanner<'_> {
    /// Scans typed text starting at absolute offset `base`.
    fn typed(
        &mut self,
        text: &str,
        base: usize,
        mut prev: Option<char>,
    ) -> Result<Option<char>, DirectiveError> {
        let mut cur = Cursor::new(text, base);
        while let Some(c) = cur.peek() {
            match c {
                '`' => {
                    code(&mut cur, &mut self.out.prose);
                    prev = Some('`');
                }
                '$' if is_boundary(prev) && cur.peek2().is_some_and(|n| n.is_ascii_lowercase()) => {
                    self.directive(&mut cur)?;
                    prev = cur.slice(0, cur.pos()).chars().next_back();
                }
                '@' if is_boundary(prev) => {
                    let start = cur.pos();
                    let at = cur.offset();
                    cur.bump();
                    let mut probe = cur;
                    if let Some(m) = mention::scan(&mut probe) {
                        let end = probe.offset();
                        self.out.mentions.push(Spanned {
                            value: m,
                            span: at..end,
                        });
                        self.out.prose.push_str(cur.slice(start, probe.pos()));
                        cur = probe;
                    } else {
                        self.out.prose.push('@');
                    }
                    prev = cur.slice(0, cur.pos()).chars().next_back();
                }
                _ => {
                    cur.bump();
                    self.out.prose.push(c);
                    prev = Some(c);
                }
            }
        }
        Ok(prev)
    }

    /// Parses `$name[(args)]` at the cursor and removes it from the prose.
    fn directive(&mut self, cur: &mut Cursor<'_>) -> Result<(), DirectiveError> {
        let at = cur.offset();
        cur.bump();
        let name = cur.take_while(is_dname_char).to_owned();
        let open_at = cur.offset();
        let args = if cur.eat('(') {
            let mut parser = ArgParser::new(*cur, self.limits);
            let args = parser.parenthesised(1, open_at)?;
            *cur = parser.cur;
            Some(args)
        } else {
            None
        };
        let value = typed(&name, args, at)?;
        self.out.directives.push(Spanned {
            value,
            span: at..cur.offset(),
        });
        // Drop one separating space so "a $rag b" reads "a b".
        if (self.out.prose.is_empty() || self.out.prose.ends_with(char::is_whitespace))
            && cur.peek() == Some(' ')
        {
            cur.bump();
        }
        Ok(())
    }
}

/// Parses one message.
///
/// # Errors
/// [`DirectiveError`] for limit violations, malformed directive arguments, invalid typed directive
/// arguments and unknown or malformed slash commands. Unknown `$names` are not errors (`Custom`).
pub fn parse(input: &str, limits: &ParseLimits) -> Result<ParsedInput, DirectiveError> {
    parse_segments(&[Segment::Typed(input)], limits)
}

/// Parses a message composed of typed text and attachments; spans index the concatenation of all
/// segments.
///
/// # Errors
/// As [`parse`].
pub fn parse_segments(
    segments: &[Segment<'_>],
    limits: &ParseLimits,
) -> Result<ParsedInput, DirectiveError> {
    let total = segments.iter().map(|s| match s {
        Segment::Typed(t) | Segment::Attachment(t) => t.len(),
    });
    if total.fold(0usize, usize::saturating_add) > limits.max_input_bytes {
        return Err(DirectiveError::TooLong);
    }
    if let [Segment::Typed(only)] = segments {
        if let Some((name, rest, at)) = slash::recognise(only) {
            let command = slash::parse_command(name, rest, at)?;
            return Ok(ParsedInput {
                command: Some(command),
                ..ParsedInput::default()
            });
        }
    }
    let mut scanner = Scanner {
        limits,
        out: ParsedInput::default(),
    };
    let mut base = 0usize;
    let mut prev = None;
    for segment in segments {
        match segment {
            Segment::Typed(text) => prev = scanner.typed(text, base, prev)?,
            Segment::Attachment(text) => {
                scanner.out.prose.push_str(text);
                prev = text.chars().next_back().or(prev);
            }
        }
        base = base.saturating_add(match segment {
            Segment::Typed(t) | Segment::Attachment(t) => t.len(),
        });
    }
    let trimmed = scanner.out.prose.trim();
    if trimmed.len() != scanner.out.prose.len() {
        scanner.out.prose = trimmed.to_owned();
    }
    Ok(scanner.out)
}
