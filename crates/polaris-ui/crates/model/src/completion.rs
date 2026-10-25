//! Boundary-aware completion over the grammar's authoritative names and an injected file index.

use core::ops::Range;

/// Maximum suggestions returned to the frontend.
pub const MAX_RESULTS: usize = 100;
/// Maximum candidates requested from an external path index before ranking.
pub const INDEX_FETCH_LIMIT: usize = 1000;

/// The syntax introduced by a completion sigil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// A slash command at the start of the composer.
    Command,
    /// A dollar-prefixed session directive.
    Directive,
    /// An at-prefixed path mention.
    Mention,
}

/// A matching composer token, with byte offsets into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionToken<'a> {
    /// Syntax category.
    pub kind: CompletionKind,
    /// Beginning of the sigil.
    pub start: usize,
    /// End of the complete token, including text after the cursor.
    pub end: usize,
    /// Text between the sigil (or opening quote) and cursor.
    pub query: &'a str,
}

/// Ranking tiers, from strongest to weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    /// Path or basename starts with the query.
    Prefix,
    /// Path contains the query.
    Substring,
    /// Query characters occur in order in the path.
    Fuzzy,
}

/// One path returned by the application's index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathHit {
    /// Workspace-relative path.
    pub path: String,
    /// Whether this is a directory.
    pub is_dir: bool,
}

/// The application owns discovery and I/O; completion only queries this interface.
pub trait PathIndex {
    /// Return up to `limit` candidates for `q`.
    fn query(&self, q: &str, limit: usize) -> Vec<PathHit>;
}

/// A deterministic, in-memory path index useful for small workspaces.
#[derive(Debug, Clone)]
pub struct StaticPathIndex {
    paths: Vec<String>,
}

impl StaticPathIndex {
    /// Store unique paths in lexicographic order.
    pub fn new(paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut paths: Vec<String> = paths.into_iter().map(Into::into).collect();
        paths.sort();
        paths.dedup();
        Self { paths }
    }
}

impl PathIndex for StaticPathIndex {
    fn query(&self, q: &str, limit: usize) -> Vec<PathHit> {
        let mut hits: Vec<_> = self
            .paths
            .iter()
            .filter_map(|path| match_tier(q, path).map(|tier| (tier, path)))
            .collect();
        hits.sort();
        hits.into_iter()
            .take(limit)
            .map(|(_, path)| PathHit {
                path: path.clone(),
                is_dir: false,
            })
            .collect()
    }
}

/// A completion candidate and the source range it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Visible name (paths omit their sigil).
    pub label: String,
    /// Text inserted when accepted.
    pub insert: String,
    /// Help shown alongside the candidate.
    pub description: &'static str,
    /// Syntax category.
    pub kind: CompletionKind,
    /// Byte range replaced on acceptance.
    pub range: Range<usize>,
}

/// Presentation metadata for one grammar name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpEntry {
    /// Name without its sigil.
    pub name: &'static str,
    /// Human-readable action or setting.
    pub description: &'static str,
}

macro_rules! help {
    ($($name:literal => $description:literal),+ $(,)?) => {
        &[$(HelpEntry { name: $name, description: $description }),+]
    };
}

/// Help for every slash command; drift tests compare this with the parser.
pub const SLASH_HELP: &[HelpEntry] = help! {
    "model" => "Choose the model and effort",
    "usage" => "View token usage and cost",
    "stats" => "View usage history",
    "status" => "Inspect session settings and provenance",
    "quit" => "Close Polaris",
    "exit" => "Close Polaris",
    "plan" => "Toggle plan mode",
    "goal" => "Manage the active goal",
    "diff" => "Inspect proposed changes",
    "rollback" => "Restore an earlier checkpoint",
    "undo" => "Undo the latest change",
    "sessions" => "List saved sessions",
    "session" => "Manage or resume a session",
    "settings" => "Inspect settings",
    "help" => "Show commands and shortcuts",
    "clear" => "Clear the conversation view",
    "providers" => "Configure model providers",
    "onboarding" => "Set up local dependencies and access",
    "nv-profile" => "Inspect the NVIDIA workspace profile",
    "nv-verify" => "Run NVIDIA rule checks",
    "nebius-status" => "Inspect Nebius services",
    "contree-branches" => "Inspect sandbox branches",
    "datalab-sync" => "Export selected records to Data Lab",
    "datalab-finetune" => "Start a Data Lab fine-tune",
    "concurrency-stats" => "Inspect active work and queue limits",
    "cache" => "Inspect or purge caches",
    "quota" => "Inspect request and token quotas",
    "research" => "Manage a research run",
    "context-map" => "Inspect the context map",
    "context-expand" => "Expand a folded context reference",
    "parallel-dag" => "Inspect the task dependency graph",
    "screenshot" => "Capture a sandbox screenshot",
    "browser" => "Control the sandbox browser",
    "ui-diff" => "Inspect a visual difference",
    "hack" => "Run a security investigation",
    "redteam" => "Run adversarial checks",
    "search-local" => "Search indexed local knowledge",
    "ci" => "Inspect continuous integration settings",
    "ci-run" => "Run the sandbox CI pipeline",
    "nix-flake" => "Manage a Nix development environment",
    "proof-of-done" => "Inspect completion evidence",
    "panel" => "Open or pin the cockpit panel",
    "agents" => "Inspect active agents",
    "questions" => "Review pending human questions",
};

/// Help for every typed directive; drift tests compare this with the parser.
pub const DIRECTIVE_HELP: &[HelpEntry] = help! {
    "websearch" => "Enable web search",
    "subagents" => "Configure subagent use",
    "subagent-models" => "Choose subagent models and effort ranges",
    "links" => "Configure link handling",
    "skills" => "Configure available skills",
    "mcp" => "Configure MCP tools",
    "rag" => "Configure retrieval",
    "nvidia" => "Choose the NVIDIA profile",
    "nvidia-docs" => "Configure NVIDIA documentation retrieval",
    "sandbox" => "Choose the execution sandbox",
    "nebius-sandbox" => "Choose the execution sandbox",
    "nebius-branching" => "Configure sandbox branching",
    "nebius-datalab" => "Configure Data Lab export",
    "nebius-speculative" => "Configure speculative execution",
    "concurrency" => "Set concurrency limits",
    "ratelimit" => "Set request and token limits",
    "budget" => "Set the session spending budget",
    "storage-level" => "Set storage retention",
    "browser" => "Configure sandbox browser access",
    "needle" => "Configure local Needle inference",
    "env" => "Choose the development environment",
    "ci" => "Configure the CI environment",
};

/// Rank a path case-insensitively, using Unicode lowercasing and a subsequence fallback.
#[must_use]
pub fn match_tier(query: &str, path: &str) -> Option<MatchTier> {
    let query = query.to_lowercase();
    let path = path.to_lowercase();
    let basename = match path.rsplit(['/', '\\']).next() {
        Some(name) => name,
        None => path.as_str(),
    };
    if path.starts_with(&query) || basename.starts_with(&query) {
        return Some(MatchTier::Prefix);
    }
    if path.contains(&query) {
        return Some(MatchTier::Substring);
    }
    let mut remaining = path.chars();
    if query.chars().all(|needle| remaining.any(|ch| ch == needle)) {
        Some(MatchTier::Fuzzy)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LexToken {
    pub kind: CompletionKind,
    pub start: usize,
    pub body: usize,
    pub end: usize,
    pub quoted: bool,
}

fn name_char(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'
}

/// Scan syntax tokens once. Backtick runs only close code spans of the same width.
pub(crate) fn tokens(input: &str) -> Vec<LexToken> {
    let mut result = Vec::new();
    let mut chars = input.char_indices().peekable();
    let mut code_width = None;
    while let Some((start, ch)) = chars.next() {
        if ch == '`' {
            let mut width = 1usize;
            while matches!(chars.peek(), Some((_, '`'))) {
                let _next = chars.next();
                width = width.saturating_add(1);
            }
            code_width = match code_width {
                None => Some(width),
                Some(open) if open == width => None,
                Some(open) => Some(open),
            };
            continue;
        }
        if code_width.is_some() {
            continue;
        }
        let boundary = input
            .get(..start)
            .and_then(|s| s.chars().next_back())
            .is_none_or(|prev| prev.is_whitespace() || matches!(prev, '(' | '['));
        let kind = match ch {
            '/' if start == 0 => CompletionKind::Command,
            '$' if boundary => CompletionKind::Directive,
            '@' if boundary => CompletionKind::Mention,
            _ => continue,
        };
        let mut body = start.saturating_add(ch.len_utf8());
        let quoted = kind == CompletionKind::Mention && matches!(chars.peek(), Some((_, '"')));
        if quoted {
            let _quote = chars.next();
            body = body.saturating_add(1);
        }
        let mut end = body;
        if quoted {
            let mut escaped = false;
            for (at, c) in chars.by_ref() {
                end = at.saturating_add(c.len_utf8());
                if c == '"' && !escaped {
                    break;
                }
                escaped = c == '\\' && !escaped;
            }
        } else if kind == CompletionKind::Mention {
            let mut depth = 0usize;
            while let Some(&(at, c)) = chars.peek() {
                if c.is_whitespace()
                    || matches!(c, '`' | '"' | ']' | '}' | ',' | ';')
                    || (c == ')' && depth == 0)
                {
                    break;
                }
                let _next = chars.next();
                if c == '(' {
                    depth = depth.saturating_add(1);
                }
                if c == ')' {
                    depth = depth.saturating_sub(1);
                }
                end = at.saturating_add(c.len_utf8());
            }
            while input
                .get(body..end)
                .is_some_and(|s| s.ends_with(['.', '!', '?', ':']))
            {
                end = end.saturating_sub(1);
            }
        } else {
            while let Some(&(at, c)) = chars.peek() {
                if !name_char(c) {
                    break;
                }
                let _next = chars.next();
                end = at.saturating_add(c.len_utf8());
            }
            // Shell variables and currency are prose, not directives; absolute paths are not commands.
            if kind == CompletionKind::Directive
                && chars.peek().is_some_and(|(_, c)| c.is_ascii_uppercase())
            {
                continue;
            }
            if kind == CompletionKind::Directive
                && input
                    .get(body..end)
                    .and_then(|s| s.chars().next())
                    .is_some_and(|c| c.is_ascii_digit())
            {
                continue;
            }
            if kind == CompletionKind::Command && matches!(chars.peek(), Some((_, '/' | '\\'))) {
                continue;
            }
        }
        result.push(LexToken {
            kind,
            start,
            body,
            end,
            quoted,
        });
    }
    result
}

/// Find the token containing `cursor`; invalid byte offsets and inert prose yield `None`.
#[must_use]
pub fn detect_token(input: &str, cursor: usize) -> Option<CompletionToken<'_>> {
    if !input.is_char_boundary(cursor) {
        return None;
    }
    for token in tokens(input) {
        if token.start < cursor && cursor <= token.end {
            let query_end = if token.quoted
                && input
                    .get(token.body..cursor)
                    .is_some_and(|s| s.ends_with('"'))
            {
                cursor.saturating_sub(1)
            } else {
                cursor
            };
            let query = input.get(token.body..query_end)?;
            return Some(CompletionToken {
                kind: token.kind,
                start: token.start,
                end: token.end,
                query,
            });
        }
    }
    None
}

/// Complete commands, directives or paths at the cursor, with stable ordering and a hard result cap.
#[must_use]
pub fn complete(input: &str, cursor: usize, paths: &impl PathIndex) -> Vec<Suggestion> {
    let Some(token) = detect_token(input, cursor) else {
        return Vec::new();
    };
    let mut ranked = Vec::new();
    if token.kind == CompletionKind::Mention {
        for hit in paths
            .query(token.query, INDEX_FETCH_LIMIT)
            .into_iter()
            .take(INDEX_FETCH_LIMIT)
        {
            if let Some(tier) = match_tier(token.query, &hit.path) {
                let insert = if hit
                    .path
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '`'))
                {
                    format!(
                        "@\"{}\"",
                        hit.path.replace('\\', "\\\\").replace('"', "\\\"")
                    )
                } else {
                    format!("@{}", hit.path)
                };
                ranked.push((
                    tier,
                    Suggestion {
                        label: hit.path,
                        insert,
                        description: if hit.is_dir { "Directory" } else { "File" },
                        kind: token.kind,
                        range: token.start..token.end,
                    },
                ));
            }
        }
    } else {
        let (table, sigil) = match token.kind {
            CompletionKind::Command => (SLASH_HELP, '/'),
            CompletionKind::Directive => (DIRECTIVE_HELP, '$'),
            CompletionKind::Mention => return Vec::new(),
        };
        for entry in table {
            if let Some(tier) = match_tier(token.query, entry.name) {
                let label = format!("{sigil}{}", entry.name);
                ranked.push((
                    tier,
                    Suggestion {
                        insert: label.clone(),
                        label,
                        description: entry.description,
                        kind: token.kind,
                        range: token.start..token.end,
                    },
                ));
            }
        }
    }
    ranked
        .sort_by(|(a_tier, a), (b_tier, b)| a_tier.cmp(b_tier).then_with(|| a.label.cmp(&b.label)));
    ranked.dedup_by(|(_, a), (_, b)| a.label == b.label);
    ranked
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, suggestion)| suggestion)
        .collect()
}
