//! `Redactor` for logs and model-bound text: secret/PII rules plus high-entropy runs, merged in
//! one pass into the caller's buffer.

use std::iter::Peekable;

use polaris_sdk::registry::{RedactStats, Redactor};
use regex_automata::Input;

use crate::entropy::{EntropyCfg, EntropySpans};
use crate::error::AutomataError;
use crate::scan::{Category, DfaScanner, rule_of};

/// Masks matches of a scanner (and optionally entropy candidates) as `[REDACTED:<category>]`.
#[derive(Debug, Clone)]
pub struct DfaRedactor {
    scanner: DfaScanner,
    entropy: Option<EntropyCfg>,
}

const fn marker(category: Category) -> &'static [u8] {
    match category {
        Category::Secret => b"[REDACTED:secret]",
        Category::Pii => b"[REDACTED:pii]",
        Category::DangerousShell => b"[REDACTED:command]",
    }
}

impl DfaRedactor {
    /// Redacts every match of `scanner`, plus entropy candidates when `entropy` is set.
    #[must_use]
    pub const fn new(scanner: DfaScanner, entropy: Option<EntropyCfg>) -> Self {
        Self { scanner, entropy }
    }

    /// The default secret + PII rules and the default entropy heuristic. Shell rules are not
    /// redacted (they are flagged by the safety gate instead).
    ///
    /// # Errors
    /// Only if the built-in pack fails to compile.
    pub fn default_redactor() -> Result<Self, AutomataError> {
        Ok(Self::new(
            DfaScanner::default_pack_for(&[Category::Secret, Category::Pii])?,
            Some(EntropyCfg::default()),
        ))
    }
}

/// Takes the earlier of two sorted span streams (rules first on ties).
fn next_span<A, B>(
    rules: &mut Peekable<A>,
    entropy: &mut Peekable<B>,
) -> Option<(usize, usize, Category)>
where
    A: Iterator<Item = (usize, usize, Category)>,
    B: Iterator<Item = (usize, usize, Category)>,
{
    match (rules.peek(), entropy.peek()) {
        (Some(r), Some(e)) if e.0 < r.0 => entropy.next(),
        (Some(_), _) => rules.next(),
        (None, _) => entropy.next(),
    }
}

impl Redactor for DfaRedactor {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let mut rules = self
            .scanner
            .regex
            .find_iter(Input::new(input))
            .map(|m| {
                let category = self
                    .scanner
                    .rule_category(rule_of(m.pattern()))
                    .unwrap_or(Category::Secret);
                (m.start(), m.end(), category)
            })
            .peekable();
        let mut entropy = self
            .entropy
            .map(|cfg| EntropySpans::new(input, cfg))
            .into_iter()
            .flatten()
            .map(|(s, e)| (s, e, Category::Secret))
            .peekable();
        let mut pos = 0;
        let mut replacements: u32 = 0;
        while let Some((start, end, category)) = next_span(&mut rules, &mut entropy) {
            if start < pos {
                // Overlaps the span just replaced: extend the masked region, no second marker.
                pos = pos.max(end);
                continue;
            }
            out.extend_from_slice(input.get(pos..start).unwrap_or_default());
            out.extend_from_slice(marker(category));
            replacements = replacements.saturating_add(1);
            pos = end;
        }
        out.extend_from_slice(input.get(pos..).unwrap_or_default());
        RedactStats {
            bytes_in: input.len(),
            replacements,
        }
    }
}
