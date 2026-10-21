//! Presentation spans share the completion scanner so colouring and replacement boundaries agree.

use crate::completion::{CompletionKind, tokens};
use polaris_kernel_directives::names::{DIRECTIVES, SLASH_COMMANDS};

/// Semantic colour role; unknown syntax keeps the frontend's neutral colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    /// Recognised slash command.
    Command,
    /// Incomplete or unknown slash command.
    CommandUnknown,
    /// Recognised session directive.
    Directive,
    /// Custom or incomplete directive.
    DirectiveUnknown,
    /// File or document mention.
    Mention,
}

/// A nonempty byte range, always on UTF-8 boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassifiedSpan {
    /// Inclusive start.
    pub start: usize,
    /// Exclusive end.
    pub end: usize,
    /// Presentation role.
    pub class: TokenClass,
}

/// Classify syntax outside code spans; prose and bare sigils have no styled span.
#[must_use]
pub fn classify(input: &str) -> Vec<ClassifiedSpan> {
    tokens(input)
        .into_iter()
        .filter_map(|token| {
            let body = input.get(token.body..token.end)?;
            if body.is_empty() {
                return None;
            }
            let class = match token.kind {
                CompletionKind::Command if SLASH_COMMANDS.contains(&body) => TokenClass::Command,
                CompletionKind::Command => TokenClass::CommandUnknown,
                CompletionKind::Directive if DIRECTIVES.contains(&body) => TokenClass::Directive,
                CompletionKind::Directive => TokenClass::DirectiveUnknown,
                CompletionKind::Mention => TokenClass::Mention,
            };
            Some(ClassifiedSpan {
                start: token.start,
                end: token.end,
                class,
            })
        })
        .collect()
}
