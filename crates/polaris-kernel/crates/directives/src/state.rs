//! Session directive state (D11): a deterministic, order-dependent fold where the last occurrence of
//! each directive wins. Persistence is the canonical text of each applied directive
//! ([`crate::render::render`]) stored in CAS behind a `DirectiveApplied` event; resume re-parses
//! and re-applies it in order (`parse(render(d)) == d` is property-tested).

use std::collections::BTreeMap;

use crate::directive::SessionDirective;

/// Directives in effect for a session, keyed by canonical name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectiveState {
    active: BTreeMap<String, SessionDirective>,
}

impl DirectiveState {
    /// An empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A new session's state, seeded from configuration defaults (provided by `polaris-config`).
    #[must_use]
    pub fn with_defaults(defaults: impl IntoIterator<Item = SessionDirective>) -> Self {
        let mut state = Self::new();
        for d in defaults {
            state.apply(d);
        }
        state
    }

    /// Applies a directive; returns the value it replaced.
    pub fn apply(&mut self, directive: SessionDirective) -> Option<SessionDirective> {
        self.active.insert(directive.key().to_owned(), directive)
    }

    /// The directive in effect under `key` (e.g. `"websearch"`, `"sandbox"`, a custom name).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&SessionDirective> {
        self.active.get(key)
    }

    /// All directives in effect, ordered by key.
    pub fn iter(&self) -> impl Iterator<Item = &SessionDirective> {
        self.active.values()
    }

    /// Number of directives in effect.
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// `true` when no directive is in effect.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}
