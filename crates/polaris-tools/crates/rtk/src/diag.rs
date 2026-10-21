//! Diagnostic model shared by the JSON and text paths.

use serde::Serialize;

/// Severity of a diagnostic. Only errors become ledger issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// A compile or lint error.
    Error,
    /// A warning (dropped from the view whenever any error exists).
    Warning,
}

/// A source position (`file` is whatever the compiler printed, Windows separators included).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Location {
    /// File path as printed by the compiler.
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
}

/// One compiler or linter diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Error or warning.
    pub level: Level,
    /// Error code such as `E0425`, when the compiler gave one.
    pub code: Option<String>,
    /// The message without the `error[CODE]:` prefix.
    pub message: String,
    /// Primary location; for macro-expanded code, the outer call site.
    pub primary: Option<Location>,
    /// Child notes and helps, each as `note: ...` or `help: ...`.
    pub details: Vec<String>,
    /// `false` when the primary location is inside the cargo registry or a git checkout.
    pub first_party: bool,
}

impl Diagnostic {
    /// A first-party diagnostic with no location and no details.
    #[must_use]
    pub fn new(level: Level, code: Option<&str>, message: &str) -> Self {
        Self {
            level,
            code: code.map(str::to_owned),
            message: message.to_owned(),
            primary: None,
            details: Vec::new(),
            first_party: true,
        }
    }

    /// The first line as rustc prints it: `error[E0425]: message` or `warning: message`.
    #[must_use]
    pub fn headline(&self) -> String {
        let label = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
        };
        match &self.code {
            Some(code) => format!("{label}[{code}]: {}", self.message),
            None => format!("{label}: {}", self.message),
        }
    }

    /// `true` when this diagnostic belongs in the ledger: an error that is not a build summary
    /// (`aborting due to ...`, `could not compile ...`). Errors in third-party code count too,
    /// because a broken dependency still breaks the build.
    #[must_use]
    pub fn is_issue(&self) -> bool {
        self.level == Level::Error && !is_summary_message(&self.message)
    }

    /// Renders the diagnostic as a compact text block (headline, location, details).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = self.headline();
        out.push('\n');
        if let Some(loc) = &self.primary {
            out.push_str(&format!(" --> {}:{}:{}\n", loc.file, loc.line, loc.column));
        }
        for detail in &self.details {
            out.push_str(&format!("  = {detail}\n"));
        }
        out
    }
}

/// `true` for messages that summarise a build instead of describing a problem:
/// `aborting due to N previous errors`, `could not compile ...`, `build failed, ...`, and cargo's
/// `` `crate` (bin "x") generated N warnings``.
#[must_use]
pub fn is_summary_message(message: &str) -> bool {
    let m = message.trim_start();
    m.starts_with("aborting due to")
        || m.starts_with("could not compile")
        || m.starts_with("build failed")
        || (m.starts_with('`') && m.contains(" generated "))
}

/// Decides whether a path (or a whole output line) points into third-party code: the cargo
/// registry or git checkouts, under `.cargo` or under a custom `CARGO_HOME`.
///
/// Matching is case-insensitive and treats `\` and `/` alike, so
/// `C:\Users\x\.cargo\registry\src\...` is recognised as well as the Unix form. A project path that
/// merely contains the word (`my-registry/src`) is not third party.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThirdPartyMatcher {
    custom: Vec<String>,
}

fn normalise_path(text: &str) -> String {
    text.replace('\\', "/").to_ascii_lowercase()
}

impl ThirdPartyMatcher {
    /// A matcher for `.cargo/registry`, `.cargo/git` and, when given, `<cargo_home>/registry` and
    /// `<cargo_home>/git`.
    #[must_use]
    pub fn new(cargo_home: Option<&str>) -> Self {
        let custom = cargo_home
            .map(normalise_path)
            .map(|home| home.trim_end_matches('/').to_owned())
            .filter(|home| !home.is_empty())
            .map(|home| vec![format!("{home}/registry"), format!("{home}/git")])
            .unwrap_or_default();
        Self { custom }
    }

    /// A matcher that honours the `CARGO_HOME` environment variable.
    #[must_use]
    pub fn from_env() -> Self {
        let home = std::env::var("CARGO_HOME").ok();
        Self::new(home.as_deref())
    }

    /// `true` if `text` mentions a registry or git path.
    #[must_use]
    pub fn is_third_party(&self, text: &str) -> bool {
        let n = normalise_path(text);
        n.contains(".cargo/registry")
            || n.contains(".cargo/git")
            || self.custom.iter().any(|prefix| n.contains(prefix.as_str()))
    }
}
