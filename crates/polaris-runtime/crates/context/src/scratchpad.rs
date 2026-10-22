//! Append-only epistemic scratchpad with lossless CAS overflow.

use polaris_sdk::ids::ShortRef;
use polaris_storage_cas::Cas;

use crate::ContextError;

/// Type of a persistent scratchpad fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// User or system goal for the current frame.
    Goal,
    /// Invariant that must remain true.
    Invariant,
    /// Fact supported by recorded evidence.
    Confirmed,
    /// Hypothesis shown to be false.
    Disproven,
    /// Unresolved blocker.
    Blocker,
}

impl EntryKind {
    fn label(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::Invariant => "invariant",
            Self::Confirmed => "confirmed",
            Self::Disproven => "disproven",
            Self::Blocker => "blocker",
        }
    }
}

/// One append-only epistemic record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Turn that established or updated this record.
    pub turn: u32,
    /// Semantic category.
    pub kind: EntryKind,
    /// Text rendered after the stable prefix. Oversized values use a CAS marker here.
    pub text: String,
    /// CAS reference retaining the exact original text when the entry overflowed.
    pub evidence: Option<ShortRef>,
    /// Original UTF-8 byte count before optional CAS folding.
    pub original_bytes: u64,
}

/// Scratchpad overflow threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScratchpadConfig {
    /// Entry text at or below this byte count remains inline.
    pub max_inline_bytes: usize,
}

impl Default for ScratchpadConfig {
    fn default() -> Self {
        Self {
            max_inline_bytes: 2_048,
        }
    }
}

/// Append-only facts mounted after the stable prefix and before the sliding window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpistemicScratchpad {
    entries: Vec<Entry>,
    last_turn: Option<u32>,
}

impl EpistemicScratchpad {
    /// Read entries in append order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Append a fact, retaining oversized content in CAS instead of dropping it.
    ///
    /// CAS access is synchronous, matching the injected `Cas` API. Callers on async executors
    /// should move this operation to a blocking worker when the backing store may perform I/O.
    pub fn append(
        &mut self,
        turn: u32,
        kind: EntryKind,
        text: &str,
        cas: &Cas,
        config: ScratchpadConfig,
    ) -> Result<&Entry, ContextError> {
        if self.last_turn.is_some_and(|last| turn < last) {
            return Err(ContextError::TurnOrder);
        }
        let original_bytes = u64::try_from(text.len()).map_err(|_| ContextError::PrefixTooLarge)?;
        let (rendered, evidence) = if text.len() <= config.max_inline_bytes {
            (text.to_owned(), None)
        } else {
            let cid = cas.put(text.as_bytes())?;
            let short = cas.short_ref_checked(&cid)?;
            (short.to_marker(), Some(short))
        };
        self.entries.push(Entry {
            turn,
            kind,
            text: rendered,
            evidence,
            original_bytes,
        });
        self.last_turn = Some(turn);
        self.entries.last().ok_or(ContextError::PrefixTooLarge)
    }

    /// Render current entries in append order. The stable prefix is never mutated by this method.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            use std::fmt::Write as _;
            let _ = writeln!(
                out,
                "[turn {} {}] {}",
                entry.turn,
                entry.kind.label(),
                entry.text
            );
        }
        out
    }

    /// Replace a matching oversized-entry display with a verified expanded rendering.
    ///
    /// This returns a new string and leaves the append-only records untouched.
    pub fn expanded_entry(
        &self,
        short: ShortRef,
        cas: &Cas,
    ) -> Result<Option<String>, ContextError> {
        for entry in &self.entries {
            if entry.evidence == Some(short) {
                let bytes = cas.get(&cas.resolve(&short)?)?;
                let text =
                    String::from_utf8(bytes.to_vec()).map_err(|_| ContextError::InvalidUtf8)?;
                return Ok(Some(text));
            }
        }
        Ok(None)
    }

    /// Number of retained facts, including facts whose body is folded to CAS.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the scratchpad contains no facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
