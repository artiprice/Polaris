//! `@` mentions: `@path`, `@"quoted path"`, `@docx(path)`, `@pdf(path)`, `@file(path)`,
//! `@xlsx[(path)]`, `@img[(path)]`, `@scans[(path)]`. A mention must follow the start of input,
//! whitespace, `(` or `[` (so `dev@company.com` stays prose). Mentions stay in the prose too.

use crate::args::quoted;
use crate::cursor::Cursor;

/// A workspace path as written (backslashes kept; resolved against the workspace root later).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathArg(String);

impl PathArg {
    /// The path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PathArg {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for PathArg {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// A file or document reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Mention {
    /// `@path` or `@file(path)`.
    File(PathArg),
    /// `@docx(path)`.
    Docx(PathArg),
    /// `@pdf(path)`.
    Pdf(PathArg),
    /// `@xlsx(path)` or bare `@xlsx`.
    Xlsx(Option<PathArg>),
    /// `@img(path)` or bare `@img`.
    Img(Option<PathArg>),
    /// `@scans(path)` or bare `@scans`.
    Scans(Option<PathArg>),
}

/// Characters of an unquoted path token (stops at whitespace, quotes and closing brackets).
fn is_path_char(c: char) -> bool {
    !(c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']'))
}

/// Sentence punctuation stripped from the end of an unquoted path.
fn is_trailing_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
}

const KINDS: &[&str] = &["file", "docx", "pdf", "xlsx", "img", "scans"];

fn typed(kind: &str, path: Option<PathArg>) -> Option<Mention> {
    match (kind, path) {
        ("file", Some(p)) => Some(Mention::File(p)),
        ("docx", Some(p)) => Some(Mention::Docx(p)),
        ("pdf", Some(p)) => Some(Mention::Pdf(p)),
        ("xlsx", p) => Some(Mention::Xlsx(p)),
        ("img", p) => Some(Mention::Img(p)),
        ("scans", p) => Some(Mention::Scans(p)),
        _ => None,
    }
}

/// `kind( path )` with the cursor just after `(`; `None` (prose) when malformed.
fn typed_path(cur: &mut Cursor<'_>) -> Option<PathArg> {
    cur.skip_ws();
    let path = match cur.peek()? {
        '"' | '\'' => quoted(cur).ok()?,
        _ => cur
            .take_while(|c| !(c.is_whitespace() || c == ')'))
            .to_owned(),
    };
    cur.skip_ws();
    (cur.eat(')') && !path.is_empty()).then(|| PathArg::from(path))
}

/// Scans a mention with the cursor just after `@`. Returns the mention and leaves the cursor at its
/// end; returns `None` (and leaves the cursor unspecified) when the `@` is plain text.
pub(crate) fn scan(cur: &mut Cursor<'_>) -> Option<Mention> {
    match cur.peek()? {
        '"' | '\'' => {
            let path = quoted(cur).ok()?;
            (!path.is_empty()).then(|| Mention::File(PathArg::from(path)))
        }
        _ => {
            for kind in KINDS {
                let save = cur.pos();
                if cur.eat_str(kind) {
                    if cur.eat('(') {
                        let path = typed_path(cur)?;
                        return typed(kind, Some(path));
                    }
                    if !cur.peek().is_some_and(is_path_char) {
                        return typed(kind, None);
                    }
                }
                cur.reset(save);
            }
            let start = cur.pos();
            let token = cur.take_while(is_path_char);
            let trimmed = token.trim_end_matches(is_trailing_punct);
            if trimmed.is_empty() {
                return None;
            }
            cur.reset(start + trimmed.len());
            Some(Mention::File(PathArg::from(trimmed)))
        }
    }
}
