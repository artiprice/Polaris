//! Conservative command classification and workspace blast accounting.

use crate::error::{GuardError, ViewError};
use globset::{GlobBuilder, GlobMatcher};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Shell dialect used to interpret the bounded command subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// POSIX-like shell.
    Posix,
    /// PowerShell.
    PowerShell,
    /// Windows command prompt.
    Cmd,
}

/// A normalized path relative to the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspacePath(String);

impl WorkspacePath {
    /// Validates and normalizes a workspace-relative path.
    ///
    /// Both slash forms are accepted. Absolute paths, drive prefixes, UNC paths and parent
    /// components are rejected. `.` and repeated separators are collapsed.
    pub fn new(path: &str) -> Result<Self, GuardError> {
        normalize_relative(path)
    }

    /// The workspace root, represented by an empty relative path.
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    /// The normalized slash-separated path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated glob evaluated only against injected workspace paths.
#[derive(Debug, Clone)]
pub struct GlobPattern {
    pattern: String,
    matchers: Vec<PathMatcher>,
}

#[derive(Debug, Clone)]
enum PathMatcher {
    Exact(String),
    Descendants(String),
    Glob(GlobMatcher),
}

impl GlobPattern {
    /// Validates and compiles a slash-separated glob pattern.
    pub fn new(pattern: &str) -> Result<Self, GuardError> {
        Self::with_patterns(vec![pattern.to_owned()])
    }

    /// Original validated pattern or a display form for a union pattern.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    fn with_patterns(patterns: Vec<String>) -> Result<Self, GuardError> {
        let mut matchers = Vec::with_capacity(patterns.len());
        for pattern in &patterns {
            if pattern.starts_with('/') || pattern.starts_with("//") || has_drive_prefix(pattern) {
                return Err(GuardError::InvalidPath);
            }
            if pattern.split('/').any(|part| part == "..") {
                return Err(GuardError::InvalidPath);
            }
            if pattern == "**" {
                matchers.push(PathMatcher::Descendants(String::new()));
            } else if let Some(prefix) = pattern.strip_suffix("/**")
                && !contains_glob_syntax(prefix)
            {
                matchers.push(PathMatcher::Exact(prefix.to_owned()));
                matchers.push(PathMatcher::Descendants(prefix.to_owned()));
            } else if !contains_glob_syntax(pattern) {
                matchers.push(PathMatcher::Exact(pattern.to_owned()));
            } else {
                matchers.push(PathMatcher::Glob(
                    GlobBuilder::new(pattern)
                        .literal_separator(true)
                        .case_insensitive(cfg!(windows))
                        .build()
                        .map_err(|_| GuardError::InvalidGlob)?
                        .compile_matcher(),
                ));
            }
        }
        Ok(Self {
            pattern: patterns.join(" | "),
            matchers,
        })
    }

    /// Whether the pattern matches a normalized workspace path.
    #[must_use]
    pub fn matches(&self, path: &WorkspacePath) -> bool {
        self.matchers.iter().any(|matcher| match matcher {
            PathMatcher::Exact(expected) => {
                if cfg!(windows) {
                    path.as_str().eq_ignore_ascii_case(expected)
                } else {
                    path.as_str() == expected
                }
            }
            PathMatcher::Descendants(prefix) if prefix.is_empty() => true,
            PathMatcher::Descendants(prefix) => {
                let path_value = path.as_str();
                let starts_with_prefix = if cfg!(windows) {
                    path_value
                        .get(..prefix.len())
                        .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
                } else {
                    path_value.starts_with(prefix)
                };
                starts_with_prefix
                    && path_value
                        .get(prefix.len()..)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            PathMatcher::Glob(glob) => glob.is_match(path.as_str()),
        })
    }
}

fn contains_glob_syntax(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | '{'))
}

/// A file fact supplied by the workspace adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFact {
    /// Workspace-relative path.
    pub path: WorkspacePath,
    /// File length in bytes at the observation point.
    pub bytes: u64,
    /// Whether the file has uncommitted work in the observed workspace.
    pub uncommitted: bool,
}

/// Injected view used for path expansion and dirty-work detection.
pub trait WorkspaceView {
    /// Lists files matching a precompiled pattern. Each path must be unique and workspace-relative.
    fn list_matching(&self, pattern: &GlobPattern) -> Result<Vec<FileFact>, ViewError>;

    /// Returns the current dirty path set. This is separate from file listing so dirty paths that
    /// have been deleted from the worktree can still be reported.
    fn dirty_paths(&self) -> Result<BTreeSet<WorkspacePath>, ViewError>;
}

/// Warning and critical thresholds for explicit blast reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlastLimits {
    /// Affected file count at which an approval is required.
    pub approval_files: u64,
    /// Affected file count at which impact is critical.
    pub critical_files: u64,
    /// Affected byte count at which an approval is required.
    pub approval_bytes: u64,
    /// Affected byte count at which impact is critical.
    pub critical_bytes: u64,
    /// Whether touching any uncommitted path makes impact critical.
    pub dirty_is_critical: bool,
}

impl BlastLimits {
    /// Creates consistent, positive file and byte thresholds.
    pub fn new(
        approval_files: u64,
        critical_files: u64,
        approval_bytes: u64,
        critical_bytes: u64,
        dirty_is_critical: bool,
    ) -> Result<Self, GuardError> {
        if approval_files == 0
            || critical_files < approval_files
            || approval_bytes == 0
            || critical_bytes < approval_bytes
        {
            return Err(GuardError::InvalidLimits);
        }
        Ok(Self {
            approval_files,
            critical_files,
            approval_bytes,
            critical_bytes,
            dirty_is_critical,
        })
    }
}

/// Inputs for one blast-radius estimate.
#[derive(Debug, Clone, Copy)]
pub struct BlastInput<'a> {
    /// The command as submitted to the shell.
    pub command: &'a str,
    /// Workspace-relative current directory.
    pub cwd: &'a WorkspacePath,
    /// Shell dialect.
    pub shell: ShellKind,
    /// Explicit limits supplied by the caller's policy.
    pub limits: &'a BlastLimits,
    /// Effective Cargo target directory resolved from workspace configuration, if known.
    /// Cargo commands are Unknown/Critical when this fact is unavailable and the command does not
    /// supply an explicit `--target-dir`.
    pub cargo_target_dir: Option<&'a WorkspacePath>,
}

/// Confidence in command classification and path enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A supported command form and all target paths were enumerated.
    Exact,
    /// The command has known broader effects, such as Cargo output paths or git state changes.
    Estimated,
    /// Command syntax or effects were not classified.
    Unknown,
}

/// Kind of file effect predicted by the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// A file may be removed.
    Deleted,
    /// A file may be created or replaced.
    Overwritten,
}

/// Severity after matching the classified effects against workspace facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// No supported workspace effect was found.
    None,
    /// Small, clean impact below approval limits.
    Low,
    /// Approval limit reached or uncommitted work is affected.
    High,
    /// Unknown command or critical size/dirty-work threshold reached.
    Critical,
}

/// Approval result derived from a [`BlastReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardVerdict {
    /// Impact is below the caller's approval threshold.
    Allow,
    /// The caller must obtain approval using this report.
    NeedsApproval(BlastReport),
    /// The caller's policy forbids this impact.
    Deny(String),
}

/// Enumerated effect of a command over workspace paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlastReport {
    /// Files predicted to be deleted.
    pub deleted: Vec<WorkspacePath>,
    /// Files predicted to be created or replaced.
    pub overwritten: Vec<WorkspacePath>,
    /// Sum of distinct affected file sizes, using checked arithmetic.
    pub total_bytes: u64,
    /// Affected paths with uncommitted work, including deleted dirty paths absent from the tree.
    pub uncommitted_hit: Vec<WorkspacePath>,
    /// Confidence level. Unknown command forms must have Critical severity.
    pub confidence: Confidence,
    /// Derived severity.
    pub severity: Severity,
    /// Human-readable reasons without echoing command contents.
    pub reasons: Vec<String>,
}

impl BlastReport {
    /// Returns an approval verdict for this report.
    #[must_use]
    pub fn verdict(self) -> GuardVerdict {
        match self.severity {
            Severity::None | Severity::Low => GuardVerdict::Allow,
            Severity::High | Severity::Critical => GuardVerdict::NeedsApproval(self),
        }
    }
}

#[derive(Debug, Clone)]
struct Effect {
    kind: EffectKind,
    pattern: GlobPattern,
    only_uncommitted: bool,
}

#[derive(Debug, Clone)]
struct Classification {
    effects: Vec<Effect>,
    confidence: Confidence,
    reason: String,
}

/// Estimates a command's file impact using only the injected workspace view.
///
/// Only a deliberately small command subset is classified. Unknown syntax, shell composition,
/// expansions, aliases and unsupported flags are Critical. This function never executes a
/// command and never reads the filesystem directly.
///
/// # Errors
/// Returns [`GuardError`] for inconsistent limits, invalid provider facts or checked overflow.
pub fn blast_radius<V: WorkspaceView>(
    view: &V,
    input: &BlastInput<'_>,
) -> Result<BlastReport, GuardError> {
    if input.limits.approval_files == 0
        || input.limits.critical_files < input.limits.approval_files
        || input.limits.approval_bytes == 0
        || input.limits.critical_bytes < input.limits.approval_bytes
    {
        return Err(GuardError::InvalidLimits);
    }
    let classification = classify(
        input.command,
        input.cwd,
        input.shell,
        input.cargo_target_dir,
    );
    let dirty = view.dirty_paths().map_err(GuardError::Workspace)?;
    let mut deleted = HashSet::new();
    let mut overwritten = HashSet::new();
    let mut dirty_hit = HashSet::new();
    let mut byte_sizes: HashMap<WorkspacePath, u64> = HashMap::new();
    for effect in &classification.effects {
        for fact in view
            .list_matching(&effect.pattern)
            .map_err(GuardError::Workspace)?
        {
            validate_fact(&fact)?;
            if effect.only_uncommitted && !fact.uncommitted {
                continue;
            }
            match effect.kind {
                EffectKind::Deleted => {
                    deleted.insert(fact.path.clone());
                }
                EffectKind::Overwritten => {
                    overwritten.insert(fact.path.clone());
                }
            }
            if fact.uncommitted || dirty.contains(&fact.path) {
                dirty_hit.insert(fact.path.clone());
            }
            byte_sizes
                .entry(fact.path)
                .and_modify(|bytes| *bytes = (*bytes).max(fact.bytes))
                .or_insert(fact.bytes);
        }
        for path in &dirty {
            if effect.pattern.matches(path) {
                dirty_hit.insert(path.clone());
                match effect.kind {
                    EffectKind::Deleted => {
                        deleted.insert(path.clone());
                    }
                    EffectKind::Overwritten => {
                        overwritten.insert(path.clone());
                    }
                }
            }
        }
    }
    let total_bytes = byte_sizes.values().try_fold(0_u64, |sum, bytes| {
        sum.checked_add(*bytes)
            .ok_or(GuardError::ArithmeticOverflow)
    })?;
    let affected_count =
        u64::try_from(byte_sizes.len()).map_err(|_| GuardError::ArithmeticOverflow)?;
    let confidence = classification.confidence;
    let dirty_count = u64::try_from(dirty_hit.len()).map_err(|_| GuardError::ArithmeticOverflow)?;
    let severity = severity_for(
        confidence,
        affected_count,
        total_bytes,
        dirty_count,
        input.limits,
    );
    let mut reasons = vec![classification.reason];
    if dirty_count > 0 {
        reasons.push(format!("{dirty_count} uncommitted path(s) are affected"));
    }
    let mut deleted: Vec<_> = deleted.into_iter().collect();
    let mut overwritten: Vec<_> = overwritten.into_iter().collect();
    let mut uncommitted_hit: Vec<_> = dirty_hit.into_iter().collect();
    deleted.sort_unstable();
    overwritten.sort_unstable();
    uncommitted_hit.sort_unstable();
    Ok(BlastReport {
        deleted,
        overwritten,
        total_bytes,
        uncommitted_hit,
        confidence,
        severity,
        reasons,
    })
}

fn validate_fact(fact: &FileFact) -> Result<(), GuardError> {
    if fact.path.as_str().is_empty() {
        return Err(GuardError::Workspace(ViewError::InvalidFacts));
    }
    if fact.path.as_str().starts_with('/') || has_drive_prefix(fact.path.as_str()) {
        return Err(GuardError::Workspace(ViewError::InvalidFacts));
    }
    Ok(())
}

fn severity_for(
    confidence: Confidence,
    files: u64,
    bytes: u64,
    dirty: u64,
    limits: &BlastLimits,
) -> Severity {
    if confidence == Confidence::Unknown
        || files >= limits.critical_files
        || bytes >= limits.critical_bytes
        || limits.dirty_is_critical && dirty > 0
    {
        Severity::Critical
    } else if files >= limits.approval_files || bytes >= limits.approval_bytes || dirty > 0 {
        Severity::High
    } else if files == 0 && bytes == 0 {
        Severity::None
    } else {
        Severity::Low
    }
}

fn classify(
    command: &str,
    cwd: &WorkspacePath,
    shell: ShellKind,
    cargo_target_dir: Option<&WorkspacePath>,
) -> Classification {
    let tokens = match tokenize(command, shell) {
        Some(tokens) if !tokens.is_empty() => tokens,
        _ => return unknown("command is empty or has unsupported shell syntax"),
    };
    if tokens.iter().any(|token| {
        token.contains('$') || token.contains('%') || token.contains('`') || token.contains('!')
    }) {
        return unknown("command contains a variable or history expansion");
    }
    let mut effects = Vec::new();
    let (confidence, reason) = match tokens.first().map(String::as_str) {
        Some("rm") if shell == ShellKind::Posix => {
            classify_remove(&tokens, cwd, &mut effects, shell, false)
        }
        Some("del") if shell == ShellKind::Cmd => {
            classify_remove(&tokens, cwd, &mut effects, shell, true)
        }
        Some("rmdir") if matches!(shell, ShellKind::Posix | ShellKind::Cmd) => {
            classify_rmdir(&tokens, cwd, &mut effects, shell)
        }
        Some("rd") if shell == ShellKind::Cmd => classify_rmdir(&tokens, cwd, &mut effects, shell),
        Some("Remove-Item") if shell == ShellKind::PowerShell => {
            classify_remove_item(&tokens, cwd, &mut effects)
        }
        Some("cargo") => classify_cargo(&tokens, cwd, cargo_target_dir, &mut effects),
        Some("git") => classify_git(&tokens, &mut effects),
        Some("mv") => classify_move(&tokens, cwd, &mut effects),
        Some("move") if shell == ShellKind::Cmd => classify_move(&tokens, cwd, &mut effects),
        Some("find") => classify_find(&tokens, cwd, &mut effects),
        _ => return classify_redirection(&tokens, cwd, effects),
    };
    match confidence {
        Confidence::Unknown => unknown(&reason),
        _ => Classification {
            effects,
            confidence,
            reason,
        },
    }
}

fn unknown(reason: &str) -> Classification {
    Classification {
        effects: Vec::new(),
        confidence: Confidence::Unknown,
        reason: reason.to_owned(),
    }
}

fn classify_remove(
    tokens: &[String],
    cwd: &WorkspacePath,
    effects: &mut Vec<Effect>,
    shell: ShellKind,
    windows_del: bool,
) -> (Confidence, String) {
    let mut recursive = false;
    let mut after_delimiter = false;
    let mut operands = Vec::new();
    for token in tokens.iter().skip(1) {
        if token == "--" {
            after_delimiter = true;
        } else if windows_del
            && shell != ShellKind::Posix
            && matches!(token.to_ascii_lowercase().as_str(), "/f" | "/q")
        {
        } else if windows_del && shell != ShellKind::Posix && token.eq_ignore_ascii_case("/s") {
            recursive = true;
        } else if !after_delimiter && token.starts_with('-') {
            let flags = token.trim_start_matches('-');
            if flags
                .chars()
                .all(|flag| matches!(flag, 'r' | 'R' | 'f' | 'v'))
            {
                recursive |= flags.contains('r') || flags.contains('R');
            } else {
                return (
                    Confidence::Unknown,
                    "remove command has unsupported flags".to_owned(),
                );
            }
        } else if token.starts_with('/') && !token.contains('*') && !token.contains('?') {
            return (
                Confidence::Unknown,
                "absolute paths are not classified".to_owned(),
            );
        } else {
            operands.push(token.as_str());
        }
    }
    if operands.is_empty() {
        return (
            Confidence::Unknown,
            "remove command has no target".to_owned(),
        );
    }
    for operand in operands {
        match add_path_effect(effects, cwd, operand, EffectKind::Deleted, recursive, false) {
            Ok(()) => {}
            Err(_) => {
                return (
                    Confidence::Unknown,
                    "remove target escapes workspace".to_owned(),
                );
            }
        }
    }
    (Confidence::Exact, "recognized remove command".to_owned())
}

fn classify_rmdir(
    tokens: &[String],
    cwd: &WorkspacePath,
    effects: &mut Vec<Effect>,
    shell: ShellKind,
) -> (Confidence, String) {
    let mut recursive = false;
    let mut operands = Vec::new();
    for token in tokens.iter().skip(1) {
        let lower = token.to_ascii_lowercase();
        if matches!(lower.as_str(), "/s" | "/q" | "-r" | "-recurse") {
            recursive |= matches!(lower.as_str(), "/s" | "-r" | "-recurse");
        } else if token.starts_with('-') || token.starts_with('/') && shell == ShellKind::Posix {
            return (
                Confidence::Unknown,
                "rmdir has unsupported flags".to_owned(),
            );
        } else {
            operands.push(token.as_str());
        }
    }
    if operands.is_empty() {
        return (Confidence::Unknown, "rmdir has no target".to_owned());
    }
    for operand in operands {
        if add_path_effect(effects, cwd, operand, EffectKind::Deleted, recursive, false).is_err() {
            return (
                Confidence::Unknown,
                "rmdir target escapes workspace".to_owned(),
            );
        }
    }
    (Confidence::Exact, "recognized rmdir command".to_owned())
}

fn classify_remove_item(
    tokens: &[String],
    cwd: &WorkspacePath,
    effects: &mut Vec<Effect>,
) -> (Confidence, String) {
    let mut recursive = false;
    let mut operands = Vec::new();
    let mut expect_path = false;
    for token in tokens.iter().skip(1) {
        let lower = token.to_ascii_lowercase();
        if expect_path {
            operands.push(token.as_str());
            expect_path = false;
        } else if lower == "-recurse" || lower == "-r" {
            recursive = true;
        } else if lower == "-force" || lower == "-f" {
        } else if lower == "-path" || lower == "-literalpath" {
            expect_path = true;
        } else if token.starts_with('-') {
            return (
                Confidence::Unknown,
                "Remove-Item has unsupported parameters".to_owned(),
            );
        } else {
            operands.push(token.as_str());
        }
    }
    if expect_path || operands.is_empty() {
        return (
            Confidence::Unknown,
            "Remove-Item has no complete path argument".to_owned(),
        );
    }
    for operand in operands {
        if add_path_effect(effects, cwd, operand, EffectKind::Deleted, recursive, false).is_err() {
            return (
                Confidence::Unknown,
                "Remove-Item target escapes workspace".to_owned(),
            );
        }
    }
    (
        Confidence::Exact,
        "recognized Remove-Item command".to_owned(),
    )
}

fn classify_cargo(
    tokens: &[String],
    cwd: &WorkspacePath,
    known_target_dir: Option<&WorkspacePath>,
    effects: &mut Vec<Effect>,
) -> (Confidence, String) {
    let Some(subcommand) = tokens.get(1).map(String::as_str) else {
        return (
            Confidence::Unknown,
            "cargo command has no subcommand".to_owned(),
        );
    };
    let mut explicit_target = None;
    let mut expect_target = false;
    for token in tokens.iter().skip(2) {
        if expect_target {
            explicit_target = resolve_operand(cwd, token);
            expect_target = false;
        } else if token == "--target-dir" {
            expect_target = true;
        } else if token.starts_with("--target-dir=") {
            explicit_target = token
                .split_once('=')
                .and_then(|(_, path)| resolve_operand(cwd, path));
        } else if token == "--manifest-path" || token.starts_with("--manifest-path=") {
            return (
                Confidence::Unknown,
                "Cargo manifest root is not classified".to_owned(),
            );
        }
    }
    if expect_target {
        return (
            Confidence::Unknown,
            "Cargo target directory argument is missing".to_owned(),
        );
    }
    let target_dir =
        explicit_target.or_else(|| known_target_dir.map(|path| path.as_str().to_owned()));
    let Some(target_dir) = target_dir else {
        return (
            Confidence::Unknown,
            "effective Cargo target directory is unavailable".to_owned(),
        );
    };
    match subcommand {
        "clean" => match add_path_effect(
            effects,
            &WorkspacePath::root(),
            &target_dir,
            EffectKind::Deleted,
            true,
            false,
        ) {
            Ok(()) => (
                Confidence::Estimated,
                "cargo clean may delete the configured target tree".to_owned(),
            ),
            Err(_) => (
                Confidence::Unknown,
                "cargo clean target path is invalid".to_owned(),
            ),
        },
        "test" | "check" | "build" | "clippy" => {
            match add_path_effect(
                effects,
                &WorkspacePath::root(),
                &target_dir,
                EffectKind::Overwritten,
                true,
                false,
            ) {
                Ok(()) => (
                    Confidence::Estimated,
                    "Cargo may update the target tree".to_owned(),
                ),
                Err(_) => (
                    Confidence::Unknown,
                    "Cargo target path is invalid".to_owned(),
                ),
            }
        }
        _ => (
            Confidence::Unknown,
            "cargo subcommand is not classified".to_owned(),
        ),
    }
}

fn classify_git(tokens: &[String], effects: &mut Vec<Effect>) -> (Confidence, String) {
    if tokens.get(1).is_some_and(|value| value == "reset")
        && tokens.get(2).is_some_and(|value| value == "--hard")
        && tokens.len() == 3
    {
        return match add_pattern_effect(effects, "**", EffectKind::Overwritten, true, true) {
            Ok(()) => (
                Confidence::Estimated,
                "git reset --hard rewrites uncommitted tracked paths".to_owned(),
            ),
            Err(_) => (
                Confidence::Unknown,
                "git reset target pattern is invalid".to_owned(),
            ),
        };
    }
    if tokens.get(1).is_some_and(|value| value == "clean") {
        let flags = tokens.get(2).map(String::as_str);
        if tokens.len() == 3
            && flags.is_some_and(|value| {
                value.starts_with('-')
                    && value.contains('f')
                    && value.contains('d')
                    && value.chars().skip(1).all(|c| matches!(c, 'f' | 'd' | 'q'))
            })
        {
            return match add_pattern_effect(effects, "**", EffectKind::Deleted, true, false) {
                Ok(()) => (
                    Confidence::Estimated,
                    "git clean may remove uncommitted paths".to_owned(),
                ),
                Err(_) => (
                    Confidence::Unknown,
                    "git clean target pattern is invalid".to_owned(),
                ),
            };
        }
    }
    (
        Confidence::Unknown,
        "git command is not classified".to_owned(),
    )
}

fn classify_move(
    tokens: &[String],
    cwd: &WorkspacePath,
    effects: &mut Vec<Effect>,
) -> (Confidence, String) {
    if tokens.len() != 3 {
        return (
            Confidence::Unknown,
            "mv requires exactly one source and destination".to_owned(),
        );
    }
    let source = match tokens.get(1).and_then(|value| resolve_operand(cwd, value)) {
        Some(path) => path,
        None => {
            return (
                Confidence::Unknown,
                "mv source escapes workspace".to_owned(),
            );
        }
    };
    let destination = match tokens.get(2).and_then(|value| resolve_operand(cwd, value)) {
        Some(path) => path,
        None => {
            return (
                Confidence::Unknown,
                "mv destination escapes workspace".to_owned(),
            );
        }
    };
    for (path, kind) in [
        (&source, EffectKind::Deleted),
        (&destination, EffectKind::Overwritten),
    ] {
        if add_pattern_effect(effects, path.as_str(), kind, true, false).is_err() {
            return (Confidence::Unknown, "mv target glob is invalid".to_owned());
        }
    }
    (
        Confidence::Estimated,
        "mv may remove its source and replace its destination".to_owned(),
    )
}

fn classify_find(
    tokens: &[String],
    cwd: &WorkspacePath,
    effects: &mut Vec<Effect>,
) -> (Confidence, String) {
    if tokens.len() < 3 || tokens.last().is_none_or(|token| token != "-delete") {
        return (
            Confidence::Unknown,
            "find expression is not classified".to_owned(),
        );
    }
    let root = match tokens.get(1).and_then(|value| resolve_operand(cwd, value)) {
        Some(path) => path,
        None => {
            return (
                Confidence::Unknown,
                "find root escapes workspace".to_owned(),
            );
        }
    };
    if add_pattern_effect(effects, root.as_str(), EffectKind::Deleted, true, false).is_err() {
        return (Confidence::Unknown, "find root glob is invalid".to_owned());
    }
    (
        Confidence::Estimated,
        "find -delete may remove matching descendants".to_owned(),
    )
}

fn classify_redirection(
    tokens: &[String],
    cwd: &WorkspacePath,
    mut effects: Vec<Effect>,
) -> Classification {
    let redirects: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| (token == ">").then_some(index))
        .collect();
    if redirects.len() != 1 {
        return unknown("command shape is not classified");
    }
    let Some(index) = redirects.first().copied() else {
        return unknown("redirection target is missing");
    };
    if index == 0 || tokens.len() != index.saturating_add(2) {
        return unknown("redirection command shape is not classified");
    }
    let executable = tokens.first().map(String::as_str);
    if !matches!(executable, Some("echo" | "printf" | "cat")) {
        return unknown("redirection source command is not classified");
    }
    if add_path_effect(
        &mut effects,
        cwd,
        match tokens.get(index.saturating_add(1)) {
            Some(path) => path.as_str(),
            None => return unknown("redirection target is missing"),
        },
        EffectKind::Overwritten,
        false,
        false,
    )
    .is_err()
    {
        return unknown("redirection target escapes workspace");
    }
    Classification {
        effects,
        confidence: Confidence::Exact,
        reason: "recognized truncating redirection".to_owned(),
    }
}

fn add_path_effect(
    effects: &mut Vec<Effect>,
    cwd: &WorkspacePath,
    operand: &str,
    kind: EffectKind,
    recursive: bool,
    only_uncommitted: bool,
) -> Result<(), GuardError> {
    let path = resolve_operand(cwd, operand).ok_or(GuardError::InvalidPath)?;
    add_pattern_effect(effects, &path, kind, recursive, only_uncommitted)
}

fn add_pattern_effect(
    effects: &mut Vec<Effect>,
    path: &str,
    kind: EffectKind,
    recursive: bool,
    only_uncommitted: bool,
) -> Result<(), GuardError> {
    let mut patterns = vec![path.to_owned()];
    if recursive {
        patterns.push(if path.is_empty() {
            "**".to_owned()
        } else {
            format!("{path}/**")
        });
    }
    effects.push(Effect {
        kind,
        pattern: GlobPattern::with_patterns(patterns)?,
        only_uncommitted,
    });
    Ok(())
}

fn resolve_operand(cwd: &WorkspacePath, operand: &str) -> Option<String> {
    if operand.starts_with('/') || operand.starts_with("\\\\") || has_drive_prefix(operand) {
        return None;
    }
    let normalized = operand.replace('\\', "/");
    let mut parts: Vec<String> = cwd
        .as_str()
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part.to_owned()),
        }
    }
    Some(parts.join("/"))
}

fn normalize_relative(path: &str) -> Result<WorkspacePath, GuardError> {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.starts_with("//") || has_drive_prefix(&normalized)
    {
        return Err(GuardError::InvalidPath);
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(GuardError::InvalidPath),
            _ => parts.push(part.to_owned()),
        }
    }
    Ok(WorkspacePath(parts.join("/")))
}

fn has_drive_prefix(path: &str) -> bool {
    let mut chars = path.chars();
    matches!((chars.next(), chars.next()), (Some(letter), Some(':')) if letter.is_ascii_alphabetic())
}

fn tokenize(command: &str, shell: ShellKind) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if quote.is_some_and(|value| value == '"') && character == '\\' && shell == ShellKind::Posix
        {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            ';' | '|' | '&' | '<' | '\n' | '\r' => return None,
            '>' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(">".to_owned());
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if escaped || quote.is_some() {
        return None;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Some(tokens)
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-runtime/crates/guards/src/blast.rs"]
mod workload_tests;
