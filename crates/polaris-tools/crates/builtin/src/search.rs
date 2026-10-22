//! Workspace listing, globbing and ignore-aware regular-expression search.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

use globset::GlobBuilder;
use ignore::WalkBuilder;
use polaris_security_automata::{UserRegexLimits, validate_user_regex};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::{
    ToolLimits,
    path::{WorkspacePath, resolve_in_workspace},
    read::path_tool_error,
};

/// Input for listing one workspace directory; `None` lists the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListDirInput {
    /// Directory path, or `None` for the workspace root.
    #[serde(default)]
    pub path: Option<String>,
}

/// Input for a repository-relative glob pattern.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GlobInput {
    /// Glob pattern using `/`-separated workspace paths.
    pub pattern: String,
}

/// Input for regular-expression search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GrepInput {
    /// Regex pattern, validated by the shared automata guard before compilation.
    pub pattern: String,
    /// Optional workspace-relative file or directory. `None` searches the workspace.
    #[serde(default)]
    pub path: Option<String>,
    /// Case-insensitive matching.
    #[serde(default)]
    pub case_insensitive: bool,
}

/// One directory entry or grep match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRow {
    /// Workspace-relative path with `/` separators.
    pub path: String,
    /// Matched line number, one-based; absent for directory/glob rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// Matching line text, absent for directory/glob rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Whether this entry is a directory, absent for grep matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dir: Option<bool>,
    /// File byte length, absent when not available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Bounded result rows and an explicit truncation flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchOutput {
    /// Ordered rows.
    pub rows: Vec<SearchRow>,
    /// True when count, byte or time limits omitted results.
    pub truncated: bool,
}

/// List directory entries in deterministic name order. Symlinks and secret paths are omitted.
pub fn list_dir(
    root: &Path,
    input: &ListDirInput,
    limits: &ToolLimits,
) -> Result<SearchOutput, polaris_sdk::registry::ToolError> {
    let directory = resolve_optional_directory(root, input.path.as_deref())?;
    if !directory.is_dir() {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "path is not a directory".to_owned(),
        ));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| polaris_sdk::registry::ToolError::Io("workspace root failed".to_owned()))?;
    let start = Instant::now();
    let mut selected = BTreeMap::new();
    let mut truncated = false;
    let mut builder = WalkBuilder::new(&directory);
    builder
        .hidden(false)
        .follow_links(false)
        .max_depth(Some(1))
        .add_custom_ignore_filename(".polarisignore");
    for entry in builder.build() {
        if start.elapsed() >= limits.timeout {
            truncated = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                truncated = true;
                continue;
            }
        };
        if entry.depth() == 0 || entry.path_is_symlink() {
            continue;
        }
        let path = entry.path().to_path_buf();
        let name = match path.file_name() {
            Some(name) => name.to_os_string(),
            None => continue,
        };
        let key = (name.to_string_lossy().to_ascii_lowercase(), name);
        selected.insert(key, path);
        if selected.len() > limits.max_results {
            truncated = true;
            if let Some(last) = selected.keys().next_back().cloned() {
                selected.remove(&last);
            }
        }
    }
    let mut rows = Vec::with_capacity(selected.len());
    for path in selected.into_values() {
        let Some(relative) = path.strip_prefix(&canonical_root).ok() else {
            continue;
        };
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if WorkspacePath::parse(&relative_text).is_err() {
            continue;
        }
        let resolved = match resolve_in_workspace(root, &relative_text) {
            Ok(resolved) => resolved,
            Err(_) => continue,
        };
        let metadata = fs::symlink_metadata(&resolved).map_err(|_| {
            polaris_sdk::registry::ToolError::Io("entry metadata failed".to_owned())
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        rows.push(SearchRow {
            path: relative_text,
            line: None,
            text: None,
            is_dir: Some(metadata.is_dir()),
            size_bytes: Some(metadata.len()),
        });
    }
    Ok(SearchOutput { rows, truncated })
}

/// Expand a glob under the workspace root while honoring ignore files.
pub fn glob(
    root: &Path,
    input: &GlobInput,
    limits: &ToolLimits,
) -> Result<SearchOutput, polaris_sdk::registry::ToolError> {
    let pattern = input.pattern.replace('\\', "/");
    if pattern.is_empty()
        || pattern.starts_with('/')
        || pattern
            .split('/')
            .any(|part| part == ".." || part.contains(':'))
    {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "invalid workspace glob".to_owned(),
        ));
    }
    let matcher = GlobBuilder::new(&pattern)
        .literal_separator(true)
        .build()
        .map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput("invalid glob pattern".to_owned())
        })?;
    let set = globset::GlobSetBuilder::new()
        .add(matcher)
        .build()
        .map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput("invalid glob pattern".to_owned())
        })?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| polaris_sdk::registry::ToolError::Io("workspace root failed".to_owned()))?;
    let start = Instant::now();
    let mut output = SearchOutput {
        rows: Vec::new(),
        truncated: false,
    };
    for entry in walker(&canonical_root) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                output.truncated = true;
                continue;
            }
        };
        if start.elapsed() >= limits.timeout {
            output.truncated = true;
            break;
        }
        if entry.path_is_symlink() || entry.depth() == 0 {
            continue;
        }
        let relative = match entry.path().strip_prefix(&canonical_root) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if !set.is_match(&relative_text) || WorkspacePath::parse(&relative_text).is_err() {
            continue;
        }
        let resolved = match resolve_in_workspace(&canonical_root, &relative_text) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let metadata = match fs::symlink_metadata(resolved) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            continue;
        }
        if output.rows.len() >= limits.max_results {
            output.truncated = true;
            break;
        }
        output.rows.push(SearchRow {
            path: relative_text,
            line: None,
            text: None,
            is_dir: Some(metadata.is_dir()),
            size_bytes: Some(metadata.len()),
        });
    }
    Ok(output)
}

/// Search text files under the workspace, respecting `.gitignore` and `.polarisignore`.
pub fn grep(
    root: &Path,
    input: &GrepInput,
    limits: &ToolLimits,
) -> Result<SearchOutput, polaris_sdk::registry::ToolError> {
    let validated = validate_user_regex(
        &input.pattern,
        &UserRegexLimits {
            nfa_size_bytes: limits.regex_size_limit,
            ..UserRegexLimits::default()
        },
    )
    .map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("regex rejected by safety limits".to_owned())
    })?;
    let matcher = RegexBuilder::new(validated.as_str())
        .case_insensitive(input.case_insensitive)
        .size_limit(limits.regex_size_limit)
        .build()
        .map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput("regex could not be compiled".to_owned())
        })?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| polaris_sdk::registry::ToolError::Io("workspace root failed".to_owned()))?;
    let search_root = resolve_optional_directory(&canonical_root, input.path.as_deref())?;
    let start = Instant::now();
    let mut output = SearchOutput {
        rows: Vec::new(),
        truncated: false,
    };
    if search_root.is_file() {
        search_one_file(
            &canonical_root,
            &search_root,
            &matcher,
            limits,
            start,
            &mut output,
        )?;
        return Ok(output);
    }
    for entry in walker(&search_root) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                output.truncated = true;
                continue;
            }
        };
        if start.elapsed() >= limits.timeout {
            output.truncated = true;
            break;
        }
        if entry.path_is_symlink()
            || entry.depth() == 0
            || !entry.file_type().is_some_and(|kind| kind.is_file())
        {
            continue;
        }
        search_one_file(
            &canonical_root,
            entry.path(),
            &matcher,
            limits,
            start,
            &mut output,
        )?;
        if output.rows.len() >= limits.max_results || output.truncated {
            break;
        }
    }
    Ok(output)
}

fn search_one_file(
    root: &Path,
    path: &Path,
    matcher: &regex::Regex,
    limits: &ToolLimits,
    start: Instant,
    output: &mut SearchOutput,
) -> Result<(), polaris_sdk::registry::ToolError> {
    if start.elapsed() >= limits.timeout {
        output.truncated = true;
        return Ok(());
    }
    let relative = match path.strip_prefix(root) {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    let relative_text = relative.to_string_lossy().replace('\\', "/");
    let resolved = match resolve_in_workspace(root, &relative_text) {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    let metadata = match fs::metadata(&resolved) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };
    if !metadata.is_file() || metadata.len() > limits.max_read_bytes {
        output.truncated = true;
        return Ok(());
    }
    let maximum = usize::try_from(limits.max_read_bytes).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("read limit is too large".to_owned())
    })?;
    let read_limit = limits.max_read_bytes.saturating_add(1);
    let mut file = match File::open(resolved) {
        Ok(file) => file,
        Err(_) => {
            output.truncated = true;
            return Ok(());
        }
    };
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .is_err()
    {
        output.truncated = true;
        return Ok(());
    }
    if bytes.len() > maximum {
        output.truncated = true;
        return Ok(());
    }
    if bytes
        .get(..bytes.len().min(8_192))
        .is_some_and(|head| head.contains(&0))
    {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&bytes);
    for (line_index, line) in text.lines().enumerate() {
        if start.elapsed() >= limits.timeout {
            output.truncated = true;
            return Ok(());
        }
        if matcher.is_match(line) {
            if output.rows.len() >= limits.max_results {
                output.truncated = true;
                return Ok(());
            }
            let line_number = match u64::try_from(line_index) {
                Ok(number) => number.saturating_add(1),
                Err(_) => u64::MAX,
            };
            output.rows.push(SearchRow {
                path: relative_text.clone(),
                line: Some(line_number),
                text: Some(line.to_owned()),
                is_dir: None,
                size_bytes: None,
            });
        }
    }
    Ok(())
}

fn resolve_optional_directory(
    root: &Path,
    path: Option<&str>,
) -> Result<PathBuf, polaris_sdk::registry::ToolError> {
    match path {
        None => root
            .canonicalize()
            .map_err(|_| polaris_sdk::registry::ToolError::Io("workspace root failed".to_owned())),
        Some(path) => resolve_in_workspace(root, path).map_err(path_tool_error),
    }
}

fn walker(root: &Path) -> ignore::Walk {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .follow_links(false)
        .add_custom_ignore_filename(".polarisignore");
    builder.build()
}
