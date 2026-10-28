//! Workspace-local file indexing and completion token detection.
//!
//! The worker indexes paths only and reads only ignore-rule files, never source contents.

use globset::{GlobBuilder, GlobMatcher};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};
use walkdir::{DirEntry, WalkDir};

const MAX_RESULTS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionKind {
    Mention,
    Slash,
    Setting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionToken {
    /// UTF-8 byte range of the complete token, including its marker.
    pub range: Range<usize>,
    pub kind: CompletionKind,
    /// Search text before the cursor, excluding the marker and optional quote.
    pub query: String,
}

#[derive(Debug)]
struct ScanResult {
    generation: u64,
    complete: bool,
    paths: Vec<IndexedPath>,
    errors: Vec<String>,
}

#[derive(Debug)]
struct IndexedPath {
    display: String,
    lowercase: String,
    basename_lowercase: String,
}

impl IndexedPath {
    fn new(display: String) -> Self {
        let lowercase = display.to_lowercase();
        let basename_lowercase = lowercase
            .rsplit('/')
            .next()
            .unwrap_or(&lowercase)
            .to_owned();
        Self {
            display,
            lowercase,
            basename_lowercase,
        }
    }
}

/// An asynchronously refreshed, path-only index for a single workspace.
#[derive(Debug)]
pub struct FileIndex {
    root: PathBuf,
    generation: u64,
    tx: Sender<ScanResult>,
    rx: Receiver<ScanResult>,
    paths: Vec<IndexedPath>,
    errors: Vec<String>,
    ready: bool,
    query_cache: RefCell<Option<(String, Vec<String>)>>,
    query_cache_generation: Cell<u64>,
}

impl FileIndex {
    /// Start the first background scan. The index is empty until poll receives it.
    pub fn start(root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut index = Self {
            root,
            generation: 0,
            tx,
            rx,
            paths: Vec::new(),
            errors: Vec::new(),
            ready: false,
            query_cache: RefCell::new(None),
            query_cache_generation: Cell::new(0),
        };
        index.refresh();
        index
    }

    /// Start a new scan. Results from prior scans are ignored when they arrive.
    pub fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.ready = false;
        let root = self.root.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let (paths, errors) = scan_workspace_depth(root.clone(), 2);
            if tx
                .send(ScanResult {
                    generation,
                    complete: false,
                    paths: paths.into_iter().map(IndexedPath::new).collect(),
                    errors,
                })
                .is_err()
            {
                return;
            }
            let (paths, errors) = scan_workspace(root);
            let paths = paths.into_iter().map(IndexedPath::new).collect();
            let _ = tx.send(ScanResult {
                generation,
                complete: true,
                paths,
                errors,
            });
        });
    }

    /// Apply any finished scan for the current generation without blocking.
    /// Returns true when a current-generation snapshot or error list was replaced.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(result) if result.generation == self.generation => {
                    self.paths = result.paths;
                    self.errors = result.errors;
                    self.ready = result.complete;
                    *self.query_cache.borrow_mut() = None;
                    changed = true;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    /// Whether the current scan has produced a snapshot.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Current scan errors, if the walker reported any.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Search indexed workspace-relative paths, returning at most 100 matches.
    pub fn query(&self, needle: &str) -> Vec<String> {
        let needle_lower = needle.to_lowercase();
        if self.query_cache_generation.get() == self.generation
            && let Some((cached_needle, results)) = self.query_cache.borrow().as_ref()
            && cached_needle == &needle_lower
        {
            return results.clone();
        }

        let results: Vec<String> = if needle_lower.is_empty() {
            self.paths
                .iter()
                .take(MAX_RESULTS)
                .map(|path| path.display.clone())
                .collect()
        } else {
            let mut matches: Vec<(u8, &IndexedPath)> = self
                .paths
                .iter()
                .filter_map(|path| {
                    match_indexed_path(&path.lowercase, &path.basename_lowercase, &needle_lower)
                        .map(|rank| (rank, path))
                })
                .collect();
            matches.sort_by(|(rank_a, path_a), (rank_b, path_b)| {
                rank_a
                    .cmp(rank_b)
                    .then_with(|| path_a.lowercase.cmp(&path_b.lowercase))
                    .then_with(|| path_a.display.cmp(&path_b.display))
            });
            matches
                .into_iter()
                .take(MAX_RESULTS)
                .map(|(_, path)| path.display.clone())
                .collect()
        };
        *self.query_cache.borrow_mut() = Some((needle_lower, results.clone()));
        self.query_cache_generation.set(self.generation);
        results
    }
}

#[derive(Clone)]
struct IgnoreRule {
    base: PathBuf,
    pattern: GlobMatcher,
    negated: bool,
    dir_only: bool,
    has_slash: bool,
}

fn scan_workspace(root: PathBuf) -> (Vec<String>, Vec<String>) {
    scan_workspace_depth(root, usize::MAX)
}
fn scan_workspace_depth(root: PathBuf, max_depth: usize) -> (Vec<String>, Vec<String>) {
    let mut paths = Vec::new();
    let mut errors = Vec::new();
    let mut rules_by_dir: std::collections::HashMap<PathBuf, Vec<IgnoreRule>> =
        std::collections::HashMap::new();
    let mut walker = WalkDir::new(&root)
        .max_depth(max_depth)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();

    while let Some(result) = walker.next() {
        match result {
            Err(error) => errors.push(error.to_string()),
            Ok(entry) => {
                if !visible_for_walk(&entry) {
                    if entry.file_type().is_dir() {
                        walker.skip_current_dir();
                    }
                    continue;
                }
                let Some(relative) = entry.path().strip_prefix(&root).ok().map(PathBuf::from)
                else {
                    continue;
                };
                if relative.as_os_str().is_empty() {
                    let mut rules = Vec::new();
                    if let Some(global) = global_ignore_path() {
                        load_ignore_path(&global, Path::new(""), &mut rules, &mut errors);
                    }
                    load_ignore_file(&root, Path::new(""), ".gitignore", &mut rules, &mut errors);
                    load_ignore_file(&root, Path::new(""), ".ignore", &mut rules, &mut errors);
                    rules_by_dir.insert(relative, rules);
                    continue;
                }

                let parent = relative.parent().unwrap_or_else(|| Path::new(""));
                let inherited = rules_by_dir.get(parent).cloned().unwrap_or_default();
                if entry.file_type().is_dir() {
                    if is_ignored(&relative, true, &inherited) {
                        walker.skip_current_dir();
                        continue;
                    }
                    let mut rules = inherited;
                    load_ignore_file(
                        &root.join(&relative),
                        &relative,
                        ".gitignore",
                        &mut rules,
                        &mut errors,
                    );
                    load_ignore_file(
                        &root.join(&relative),
                        &relative,
                        ".ignore",
                        &mut rules,
                        &mut errors,
                    );
                    rules_by_dir.insert(relative.clone(), rules.clone());
                    continue;
                }
                if !entry.file_type().is_file() || is_ignore_file(&relative) {
                    continue;
                }
                if !entry.file_name().to_string_lossy().starts_with('.')
                    && !is_ignored(&relative, false, &inherited)
                {
                    paths.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    paths.sort();
    (paths, errors)
}

fn visible_for_walk(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !name.starts_with('.') || name == ".gitignore" || name == ".ignore"
}

fn is_ignore_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".gitignore" | ".ignore")
    )
}

fn load_ignore_file(
    dir: &Path,
    relative_dir: &Path,
    name: &str,
    rules: &mut Vec<IgnoreRule>,
    errors: &mut Vec<String>,
) {
    load_ignore_path(&dir.join(name), relative_dir, rules, errors);
}

fn load_ignore_path(
    path: &Path,
    relative_dir: &Path,
    rules: &mut Vec<IgnoreRule>,
    errors: &mut Vec<String>,
) {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            return;
        }
    };
    for line in contents.lines() {
        if let Some(rule) = parse_ignore_rule(line, relative_dir) {
            rules.push(rule);
        }
    }
}

fn global_ignore_path() -> Option<PathBuf> {
    if let Ok(output) = Command::new("git")
        .args(["config", "--global", "--path", "--get", "core.excludesFile"])
        .output()
        && output.status.success()
    {
        let configured = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !configured.is_empty() {
            return Some(PathBuf::from(configured));
        }
    }
    let default = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("git/ignore")
    } else {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        PathBuf::from(home).join(".config/git/ignore")
    };
    Some(default)
}

fn parse_ignore_rule(line: &str, base: &Path) -> Option<IgnoreRule> {
    let mut line = line.trim_end_matches(' ');
    if line.is_empty() {
        return None;
    }
    if line.starts_with('#') {
        return None;
    }
    let negated = line.starts_with('!') && !line.starts_with("\\!");
    if negated || line.starts_with("\\#") || line.starts_with("\\!") {
        line = &line[1..];
    }
    if line.is_empty() {
        return None;
    }
    let anchored = line.starts_with('/');
    if anchored {
        line = &line[1..];
    }
    let dir_only = line.ends_with('/');
    if dir_only {
        line = line.trim_end_matches('/');
    }
    if line.is_empty() {
        return None;
    }
    let has_slash = anchored || line.contains('/');
    let pattern = GlobBuilder::new(line)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .ok()?
        .compile_matcher();
    Some(IgnoreRule {
        base: base.to_path_buf(),
        pattern,
        negated,
        dir_only,
        has_slash,
    })
}

fn is_ignored(path: &Path, is_dir: bool, rules: &[IgnoreRule]) -> bool {
    let components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let mut prefix = PathBuf::new();
    let mut parent_ignored = false;
    for (index, component) in components.iter().enumerate() {
        prefix.push(component);
        let current_is_dir = index + 1 < components.len() || is_dir;
        let mut last_match = None;
        for rule in rules {
            let Ok(relative_to_rule) = prefix.strip_prefix(&rule.base) else {
                continue;
            };
            if relative_to_rule.as_os_str().is_empty() {
                continue;
            }
            let relative_text = relative_to_rule.to_string_lossy().replace('\\', "/");
            let matched = if rule.has_slash {
                rule.pattern.is_match(&relative_text)
            } else {
                let name = relative_to_rule
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                rule.pattern.is_match(name.as_ref())
            };
            if matched && (!rule.dir_only || current_is_dir) {
                last_match = Some(rule.negated);
            }
        }
        parent_ignored = if parent_ignored {
            true
        } else {
            last_match.is_some_and(|negated| !negated)
        };
    }
    parent_ignored
}

#[cfg(test)]
fn match_path(path: &str, needle_lower: &str) -> Option<u8> {
    let lower = path.to_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    match_indexed_path(&lower, basename, needle_lower)
}

fn match_indexed_path(lower_path: &str, basename_lower: &str, needle_lower: &str) -> Option<u8> {
    if basename_lower.starts_with(needle_lower) || lower_path.starts_with(needle_lower) {
        Some(0)
    } else if lower_path.contains(needle_lower) {
        Some(1)
    } else if is_subsequence(needle_lower, lower_path) {
        Some(2)
    } else {
        None
    }
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|wanted| chars.by_ref().any(|got| got == wanted))
}

/// Find an active mention or slash command at a UTF-8 byte cursor.
/// Mentions may be plain (@src/main.rs) or quoted (@"src/a file.rs").
/// Slash commands start at text start or after whitespace; URL and path slashes are ignored.
pub fn active_token(text: &str, cursor: usize) -> Option<CompletionToken> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    let mention = active_mention(text, before, cursor);
    let slash = active_slash(text, before, cursor);
    let setting = active_setting(text, before, cursor);
    [mention, slash, setting]
        .into_iter()
        .flatten()
        .max_by_key(|t| t.range.start)
}

fn active_mention(text: &str, before: &str, cursor: usize) -> Option<CompletionToken> {
    let start = before.rfind('@')?;
    if start > 0 {
        let previous = before[..start].chars().next_back()?;
        if !(previous.is_whitespace() || "([{,;".contains(previous)) {
            return None;
        }
    }
    let after_marker = start + 1;
    let (query_start, quoted) = if text[after_marker..].starts_with('"') {
        (after_marker + 1, true)
    } else {
        (after_marker, false)
    };
    if quoted {
        let tail = &text[query_start..];
        let close = tail.find('"').map(|offset| query_start + offset);
        if close.is_some_and(|close| cursor > close) {
            return None;
        }
        let query_end = cursor.min(close.unwrap_or(cursor));
        if text[query_start..query_end].contains('\n') {
            return None;
        }
        let end = close.map_or(cursor, |close| close + 1);
        Some(CompletionToken {
            range: start..end,
            kind: CompletionKind::Mention,
            query: text[query_start..query_end].to_owned(),
        })
    } else {
        let token_end = text[after_marker..]
            .find(char::is_whitespace)
            .map_or(text.len(), |offset| after_marker + offset);
        if cursor > token_end {
            return None;
        }
        Some(CompletionToken {
            range: start..token_end,
            kind: CompletionKind::Mention,
            query: text[after_marker..cursor].to_owned(),
        })
    }
}

fn active_setting(text: &str, before: &str, cursor: usize) -> Option<CompletionToken> {
    let start = before.rfind(char::from(36))?;
    if start > 0 && !before[..start].chars().next_back()?.is_whitespace() {
        return None;
    }
    let token_end = text[start..]
        .find(|ch: char| ch.is_whitespace() || ch == '=')
        .map_or(text.len(), |n| start + n);
    if cursor > token_end {
        return None;
    }
    if !text[start + 1..token_end]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    Some(CompletionToken {
        range: start..token_end,
        kind: CompletionKind::Setting,
        query: text[start + 1..cursor].into(),
    })
}

fn active_slash(text: &str, before: &str, cursor: usize) -> Option<CompletionToken> {
    let start = before.rfind('/')?;
    if start > 0 && !before[..start].chars().next_back()?.is_whitespace() {
        return None;
    }
    let token_end = text[start..]
        .find(char::is_whitespace)
        .map_or(text.len(), |offset| start + offset);
    if cursor > token_end {
        return None;
    }
    let body = &text[start + 1..token_end];
    if !body
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    let query_end = cursor.min(token_end);
    Some(CompletionToken {
        range: start..token_end,
        kind: CompletionKind::Slash,
        query: text[start + 1..query_end].to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_is_case_insensitive_and_prefix_first() {
        let paths = ["src/Alpha.rs", "docs/alpha-guide.md", "x/a-l-p-h-a"];
        let got: Vec<_> = paths
            .iter()
            .filter_map(|path| match_path(path, "alpha").map(|rank| (rank, path)))
            .collect();
        assert_eq!(got[0].1, &"src/Alpha.rs");
        assert!(
            got.iter()
                .any(|(rank, path)| *rank == 2 && path.ends_with("a-l-p-h-a"))
        );
    }

    #[test]
    fn recognizes_plain_and_quoted_mentions() {
        let plain = active_token("see @src/main.rs", 16).unwrap();
        assert_eq!(plain.kind, CompletionKind::Mention);
        assert_eq!(plain.query, "src/main.rs");
        assert_eq!(plain.range, 4..16);
        let quoted = active_token("see @\"src/a file.rs\"", 16).unwrap();
        assert_eq!(quoted.kind, CompletionKind::Mention);
        assert_eq!(quoted.query, "src/a file");
        assert_eq!(quoted.range, 4..20);
    }

    #[test]
    fn file_index_lists_visible_files_and_obeys_ignore_files() {
        use std::{
            fs as stdfs,
            time::{Duration, Instant, SystemTime, UNIX_EPOCH},
        };

        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = stdfs::remove_dir_all(&self.0);
            }
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("completion-fixture-{}-{nonce}", std::process::id()));
        stdfs::create_dir_all(root.join("src")).unwrap();
        stdfs::create_dir_all(root.join("nested")).unwrap();
        stdfs::create_dir_all(root.join("target")).unwrap();
        let _fixture = Fixture(root.clone());
        stdfs::write(root.join("src/main.rs"), "source content").unwrap();
        stdfs::write(root.join("ignored.txt"), "ignored content").unwrap();
        stdfs::write(root.join("keep.tmp"), "visible via negation").unwrap();
        stdfs::write(root.join("nested/drop.tmp"), "ignored by parent rule").unwrap();
        stdfs::write(root.join("nested/keep.tmp"), "visible via nested override").unwrap();
        stdfs::write(
            root.join("target/generated.rs"),
            "ignored without a Git root",
        )
        .unwrap();
        stdfs::write(root.join(".hidden"), "hidden content").unwrap();
        stdfs::write(
            root.join(".gitignore"),
            "ignored.txt\n*.tmp\n!keep.tmp\n/target/\n",
        )
        .unwrap();
        stdfs::write(root.join("nested/.gitignore"), "!keep.tmp\n").unwrap();

        let mut index = FileIndex::start(root);
        assert!(!index.is_ready());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !index.is_ready() && Instant::now() < deadline {
            index.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(index.is_ready());
        let paths = index.query("");
        assert!(
            paths.iter().any(|path| path == "src/main.rs"),
            "paths: {paths:?}, errors: {:?}",
            index.errors()
        );
        assert!(
            !paths.iter().any(|path| path == "ignored.txt"),
            "paths: {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path == "keep.tmp"),
            "paths: {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path == "nested/keep.tmp"),
            "paths: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path == "nested/drop.tmp"),
            "paths: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path.starts_with("target/")),
            "paths: {paths:?}"
        );
        assert!(
            !paths.iter().any(|path| path == ".hidden"),
            "paths: {paths:?}"
        );

        index.refresh();
        assert!(!index.is_ready());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !index.is_ready() && Instant::now() < deadline {
            index.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(index.is_ready());
    }

    #[test]
    fn query_cache_is_generation_scoped_and_cleared_on_poll() {
        let (tx, rx) = mpsc::channel();
        let mut index = FileIndex {
            root: PathBuf::new(),
            generation: 7,
            tx,
            rx,
            paths: vec![IndexedPath::new("src/old-file.rs".into())],
            errors: Vec::new(),
            ready: true,
            query_cache: RefCell::new(None),
            query_cache_generation: Cell::new(0),
        };

        assert_eq!(index.query("OLD"), vec!["src/old-file.rs"]);
        assert_eq!(index.query_cache_generation.get(), 7);
        assert_eq!(index.query_cache.borrow().as_ref().unwrap().0, "old");
        index
            .tx
            .send(ScanResult {
                generation: 7,
                complete: true,
                paths: vec![IndexedPath::new("src/new-file.rs".into())],
                errors: Vec::new(),
            })
            .unwrap();
        assert!(index.poll());
        assert!(index.query_cache.borrow().is_none());
        assert_eq!(index.query("new"), vec!["src/new-file.rs"]);
        index
            .tx
            .send(ScanResult {
                generation: 7,
                complete: false,
                paths: vec![IndexedPath::new("src/nearby.rs".into())],
                errors: vec![],
            })
            .unwrap();
        assert!(index.poll());
        assert!(!index.is_ready());
        assert_eq!(index.query("near"), vec!["src/nearby.rs"]);
        index
            .tx
            .send(ScanResult {
                generation: 6,
                complete: true,
                paths: vec![],
                errors: vec![],
            })
            .unwrap();
        assert!(!index.poll());
        assert!(!index.is_ready());
        assert_eq!(index.query("near"), vec!["src/nearby.rs"]);
        index
            .tx
            .send(ScanResult {
                generation: 7,
                complete: true,
                paths: vec![IndexedPath::new("src/deep/complete.rs".into())],
                errors: vec![],
            })
            .unwrap();
        assert!(index.poll());
        assert!(index.is_ready());
        assert_eq!(index.query("complete"), vec!["src/deep/complete.rs"]);
    }

    #[test]
    fn query_ties_are_case_insensitive_then_lexical() {
        let (tx, rx) = mpsc::channel();
        let index = FileIndex {
            root: PathBuf::new(),
            generation: 1,
            tx,
            rx,
            paths: ["src/foo.rs", "app/foo.rs", "app/Foo.rs"]
                .into_iter()
                .map(|path| IndexedPath::new(path.into()))
                .collect(),
            errors: Vec::new(),
            ready: true,
            query_cache: RefCell::new(None),
            query_cache_generation: Cell::new(0),
        };
        assert_eq!(
            index.query("FOO"),
            vec!["app/Foo.rs", "app/foo.rs", "src/foo.rs"]
        );
    }

    #[test]
    fn slash_commands_ignore_urls_and_paths() {
        let command = active_token("/help", 5).unwrap();
        assert_eq!(command.kind, CompletionKind::Slash);
        assert_eq!(command.range, 0..5);
        let after_text = active_token("say /help now", 9).unwrap();
        assert_eq!(after_text.range, 4..9);
        assert!(active_token("https://host/path", 17).is_none());
        assert!(active_token("/src/foo", 8).is_none());
        assert!(active_token("text /src/foo", 13).is_none());
    }
}

#[cfg(test)]
mod traversal_regression {
    use super::*;
    #[test]
    fn ignored_directory_is_pruned_before_loading_its_rules() {
        let workspace = std::env::current_dir().unwrap();
        let fixture = workspace
            .join("target")
            .join(format!("index-pruning-{}", std::process::id()));
        assert!(fixture.starts_with(workspace.join("target")));
        std::fs::create_dir_all(fixture.join("skip")).unwrap();
        std::fs::write(fixture.join(".gitignore"), "/skip/\n").unwrap();
        std::fs::write(fixture.join("skip/.gitignore"), [255, 254, 255]).unwrap();
        std::fs::write(fixture.join("visible.txt"), "visible").unwrap();
        let (paths, errors) = scan_workspace(fixture.clone());
        assert_eq!(paths, ["visible.txt"]);
        assert!(
            errors.is_empty(),
            "ignored subtree was traversed: {errors:?}"
        );
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
