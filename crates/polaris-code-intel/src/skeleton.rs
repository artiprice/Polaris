//! File skeletons: signatures without bodies, for context folding and the repo map.

use crate::decl::{DeclInfo, SymbolKind, declarations};
use crate::error::CodeIntelError;
use crate::parse::{ByteSpan, Parsed};
use std::borrow::Cow;

/// Longest signature kept per item, in bytes; longer ones are truncated with `...`.
pub const MAX_SIGNATURE_BYTES: usize = 1024;

/// Most items a skeleton may hold; a file with more declarations returns
/// [`CodeIntelError::TooLarge`] (a guard against hostile or generated files).
pub const MAX_SKELETON_ITEMS: usize = 100_000;

/// One declaration with its body elided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonItem {
    /// What the declaration introduces.
    pub kind: SymbolKind,
    /// The declared name (unqualified).
    pub name: String,
    /// The declaration up to its body, on one line (whitespace runs collapsed).
    pub signature: String,
    /// The whole declaration including its body, as byte offsets of the original source.
    pub span: ByteSpan,
    /// Nesting depth: 0 at the top level, +1 inside each enclosing class, impl or module.
    pub depth: u32,
}

/// The signatures of a file in source order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Skeleton {
    /// Items in source order; nested items directly follow their container.
    pub items: Vec<SkeletonItem>,
}

impl Skeleton {
    /// One line per item: the signature indented by four spaces per nesting level.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            for _ in 0..item.depth {
                out.push_str("    ");
            }
            out.push_str(&item.signature);
            out.push('\n');
        }
        out
    }
}

/// Extracts the skeleton of `p`: every tracked declaration with its signature and without its
/// body. Syntax errors do not fail the extraction; whatever tree-sitter recovered is listed.
///
/// The walk is iterative, so deeply nested input cannot overflow the stack.
///
/// # Errors
/// [`CodeIntelError::TooLarge`] if the file has more than [`MAX_SKELETON_ITEMS`] declarations.
pub fn skeleton(p: &Parsed) -> Result<Skeleton, CodeIntelError> {
    let items = declarations(p, MAX_SKELETON_ITEMS)?
        .into_iter()
        .map(|d| item_of(&d, p.source()))
        .collect();
    Ok(Skeleton { items })
}

fn item_of(d: &DeclInfo, src: &[u8]) -> SkeletonItem {
    SkeletonItem {
        kind: d.kind,
        name: d.name.clone(),
        signature: signature_text(src, d.sig_start, d.sig_end),
        span: d.span,
        depth: d.depth,
    }
}

/// Normalises the signature bytes `start..end` of `src` to a single bounded line.
pub(crate) fn signature_text(src: &[u8], start: usize, end: usize) -> String {
    let raw: Cow<'_, str> = src
        .get(start..end)
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let mut collapsed = String::with_capacity(raw.len());
    let mut whitespace = false;
    for character in raw.chars() {
        if character.is_whitespace() {
            whitespace |= !collapsed.is_empty();
            continue;
        }
        if whitespace {
            collapsed.push(' ');
            whitespace = false;
        }
        collapsed.push(character);
    }
    let trimmed = collapsed
        .trim_end_matches(|c: char| c == ';' || c == ':' || c.is_whitespace())
        .to_owned();
    truncate_on_boundary(trimmed)
}

fn truncate_on_boundary(mut text: String) -> String {
    if text.len() <= MAX_SIGNATURE_BYTES {
        return text;
    }
    let mut cut = MAX_SIGNATURE_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str("...");
    text
}
