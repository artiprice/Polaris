//! Stable schema catalog and enum-dispatched builtin invocation.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

use ignore::WalkBuilder;
use polaris_code_intel::{ParseLimits, ParserPool, RepoMapBudget, SymbolGraph, repo_map};
use polaris_sdk::{
    ids::{Cid, ShortRef, TokenCount},
    registry::{Effects, ToolContext, ToolError, ToolSpec},
};
use polaris_storage_cas::Cas;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ToolLimits, ToolOutput,
    patch::apply_unified_patch,
    path::{WorkspacePath, resolve_in_workspace},
    proposal::{EditFileRequest, NoProposalBackend, ProposalToolBackend},
    query::{JsonQueryInput, RegexQueryInput, json_query, regex_query},
    read::{ReadFileInput, read_file},
    search::{GlobInput, GrepInput, ListDirInput, glob, grep, list_dir},
    shell::{NoShellBackend, ShellInput, ShellToolBackend, output_value},
};

/// Resolves content references for `polaris_expand` using the application's real reference store.
pub trait RefResolver: Send + Sync {
    /// Resolve a full CID or unambiguous short reference to text, bounded by `max_bytes`.
    fn expand(&self, reference: &str, max_bytes: u64) -> Result<Value, ToolError>;
}

/// Supplies actual persisted user memories for `polaris_recall`.
pub trait RecallBackend: Send + Sync {
    /// Retrieve relevant records; no in-memory or synthetic fallback is permitted.
    fn recall(&self, query: &str, max_results: usize) -> Result<Value, ToolError>;
}

/// CAS-backed resolver for full and collision-checked short content references.
#[derive(Debug, Clone)]
pub struct CasRefResolver {
    cas: Cas,
}

impl CasRefResolver {
    /// Use the application's CAS instance.
    #[must_use]
    pub fn new(cas: Cas) -> Self {
        Self { cas }
    }
}

impl RefResolver for CasRefResolver {
    fn expand(&self, reference: &str, max_bytes: u64) -> Result<Value, ToolError> {
        let cid = if reference.len() == 64 {
            Cid::from_hex(reference)
                .map_err(|_| ToolError::InvalidInput("invalid content id".to_owned()))?
        } else {
            let short: ShortRef = reference.parse().map_err(|_| {
                ToolError::InvalidInput("invalid short content reference".to_owned())
            })?;
            self.cas
                .resolve(&short)
                .map_err(|_| ToolError::NotFound("content reference".to_owned()))?
        };
        let bytes = self
            .cas
            .get(&cid)
            .map_err(|_| ToolError::NotFound("content reference".to_owned()))?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > max_bytes) {
            return Err(ToolError::InvalidInput(
                "referenced content exceeds configured read bound".to_owned(),
            ));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ToolError::InvalidInput("referenced content is not UTF-8 text".to_owned())
        })?;
        Ok(json!({"cid": cid.to_hex(), "text": text, "bytes": bytes.len()}))
    }
}

/// Stable, sorted builtin tool registry. Read-only filesystem access remains bounded by limits.
#[derive(Debug)]
pub struct ToolRegistry<R, B, P = NoProposalBackend, S = NoShellBackend> {
    limits: ToolLimits,
    specs: Vec<ToolSpec>,
    refs: R,
    recall: B,
    proposals: P,
    shell: S,
}

impl<R, B> ToolRegistry<R, B, NoProposalBackend, NoShellBackend>
where
    R: RefResolver,
    B: RecallBackend,
{
    /// Create the P1 registry with actual resolver and recall services from the composition root.
    pub fn new(limits: ToolLimits, refs: R, recall: B) -> Result<Self, ToolError> {
        limits.validate()?;
        usize::try_from(limits.max_read_bytes)
            .map_err(|_| ToolError::InvalidInput("read limit exceeds platform size".to_owned()))?;
        let specs = builtin_specs();
        Ok(Self {
            limits,
            specs,
            refs,
            recall,
            proposals: NoProposalBackend,
            shell: NoShellBackend,
        })
    }
}

impl<R, B, P, S> ToolRegistry<R, B, P, S>
where
    R: RefResolver,
    B: RecallBackend,
    P: ProposalToolBackend,
    S: ShellToolBackend,
{
    /// Create a P2 registry with explicit proposal and runtime-owned sandbox execution services.
    pub fn new_with_services(
        limits: ToolLimits,
        refs: R,
        recall: B,
        proposals: P,
        shell: S,
    ) -> Result<Self, ToolError> {
        limits.validate()?;
        usize::try_from(limits.max_read_bytes)
            .map_err(|_| ToolError::InvalidInput("read limit exceeds platform size".to_owned()))?;
        let mut specs = builtin_specs();
        specs.extend(p2_builtin_specs());
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self {
            limits,
            specs,
            refs,
            recall,
            proposals,
            shell,
        })
    }
}

impl<R, B, P, S> ToolRegistry<R, B, P, S>
where
    R: RefResolver,
    B: RecallBackend,
    P: ProposalToolBackend,
    S: ShellToolBackend,
{
    /// Stable tool descriptors sorted lexicographically by name.
    #[must_use]
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Find a tool descriptor by exact name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.specs
            .binary_search_by(|spec| spec.name.as_ref().cmp(name))
            .ok()
            .and_then(|index| self.specs.get(index))
    }

    /// Invoke one exact registered tool. Unknown or unavailable tools return errors, never fake data.
    pub async fn invoke(
        &self,
        name: &str,
        arguments: &Value,
        context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        if matches!(name, "write_file" | "edit_file" | "apply_patch" | "shell") {
            if self.get(name).is_none() {
                return Err(ToolError::NotFound("builtin tool".to_owned()));
            }
            let input_bytes = serde_json::to_vec(arguments)
                .map_err(|_| ToolError::InvalidInput("invalid tool arguments".to_owned()))?;
            if u64::try_from(input_bytes.len())
                .map_or(true, |length| length > self.limits.max_input_bytes)
            {
                return Err(ToolError::InvalidInput(
                    "tool arguments exceed configured bound".to_owned(),
                ));
            }
            let limits = effective_limits(self.limits, context.deadline);
            let value = match name {
                "write_file" => {
                    let input: WriteFileInput = parse_args(arguments)?;
                    let expected_base = parse_cid(&input.expected_base)?;
                    self.proposals
                        .write_file(
                            context,
                            &input.path,
                            &input.contents,
                            expected_base,
                            &limits,
                        )
                        .await?
                }
                "edit_file" => {
                    let input: EditFileInput = parse_args(arguments)?;
                    let expected_base = parse_cid(&input.expected_base)?;
                    self.proposals
                        .edit_file(
                            context,
                            EditFileRequest {
                                path: &input.path,
                                old_text: &input.old_text,
                                new_text: &input.new_text,
                                replace_all: input.replace_all,
                                expected_base,
                            },
                            &limits,
                        )
                        .await?
                }
                "apply_patch" => {
                    let input: ApplyPatchInput = parse_args(arguments)?;
                    let expected_base = parse_cid(&input.expected_base)?;
                    self.proposals
                        .apply_patch(context, &input.path, &input.patch, expected_base, &limits)
                        .await?
                }
                "shell" => {
                    let input: ShellInput = parse_args(arguments)?;
                    let output = self.shell.execute(&input, limits, context.deadline).await?;
                    output_value(&output)
                }
                _ => return Err(ToolError::NotFound("builtin tool".to_owned())),
            };
            return finish_output(value, false, &limits);
        }
        self.invoke_blocking(name, arguments, context)
    }

    /// Invoke on an explicitly blocking owner. Filesystem, CAS and recall operations are sync.
    /// # Errors
    /// Unknown tool, invalid input, denied path, resource bound, or backend failure.
    pub fn invoke_blocking(
        &self,
        name: &str,
        arguments: &Value,
        context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        if self.get(name).is_none() {
            return Err(ToolError::NotFound("builtin tool".to_owned()));
        }
        if matches!(name, "write_file" | "edit_file" | "apply_patch" | "shell") {
            return Err(ToolError::InvalidInput(
                "this builtin requires asynchronous invocation".to_owned(),
            ));
        }
        let input_bytes = serde_json::to_vec(arguments)
            .map_err(|_| ToolError::InvalidInput("invalid tool arguments".to_owned()))?;
        if u64::try_from(input_bytes.len())
            .map_or(true, |length| length > self.limits.max_input_bytes)
        {
            return Err(ToolError::InvalidInput(
                "tool arguments exceed configured bound".to_owned(),
            ));
        }
        let limits = effective_limits(self.limits, context.deadline);
        let parser = ParserPool::with_limits(ParseLimits {
            max_source_bytes: usize::try_from(limits.max_read_bytes).map_err(|_| {
                ToolError::InvalidInput("read limit exceeds platform size".to_owned())
            })?,
            timeout: limits.timeout,
        });
        let (value, truncated) = match name {
            "read_file" => {
                let input: ReadFileInput = parse_args(arguments)?;
                let output = read_file(context.workspace_root, &input, &limits)?;
                let truncated = output.truncated;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("read result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "list_dir" => {
                let input: ListDirInput = parse_args(arguments)?;
                let output = list_dir(context.workspace_root, &input, &limits)?;
                let truncated = output.truncated;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("directory result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "glob" => {
                let input: GlobInput = parse_args(arguments)?;
                let output = glob(context.workspace_root, &input, &limits)?;
                let truncated = output.truncated;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("glob result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "grep" => {
                let input: GrepInput = parse_args(arguments)?;
                let output = grep(context.workspace_root, &input, &limits)?;
                let truncated = output.truncated;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("grep result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "skeleton" => {
                let input: PathInput = parse_args(arguments)?;
                let output =
                    self.skeleton(context.workspace_root, &input.path, &limits, &parser)?;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("skeleton result encoding failed".to_owned())
                    })?,
                    false,
                )
            }
            "repo_map" => {
                let input: RepoMapInput = parse_args(arguments)?;
                let (output, truncated) =
                    self.repo_map(context.workspace_root, &input, &limits, &parser)?;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("repo map result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "json_query" => {
                let input: JsonQueryInput = parse_args(arguments)?;
                (
                    serde_json::to_value(json_query(&input)?).map_err(|_| {
                        ToolError::InvalidInput("JSON query result encoding failed".to_owned())
                    })?,
                    false,
                )
            }
            "regex_query" => {
                let input: RegexQueryInput = parse_args(arguments)?;
                let output = regex_query(&input, limits.max_results, limits.regex_size_limit)?;
                let truncated = output.truncated;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("regex query result encoding failed".to_owned())
                    })?,
                    truncated,
                )
            }
            "diff_apply" => {
                let input: DiffApplyInput = parse_args(arguments)?;
                let result =
                    apply_unified_patch(input.base.as_bytes(), &input.patch).map_err(|_| {
                        ToolError::InvalidInput(
                            "patch is malformed or does not match the supplied base".to_owned(),
                        )
                    })?;
                let text = String::from_utf8(result).map_err(|_| {
                    ToolError::InvalidInput("patch result is not valid UTF-8 text".to_owned())
                })?;
                (json!({"result": text}), false)
            }
            "classify_command" => {
                let input: ClassifyInput = parse_args(arguments)?;
                let classification = crate::shell::classify_command(&input.argv, input.shell);
                (json!({"classification": classification}), false)
            }
            "polaris_expand" => {
                let input: ExpandInput = parse_args(arguments)?;
                (
                    self.refs.expand(&input.reference, limits.max_read_bytes)?,
                    false,
                )
            }
            "polaris_recall" => {
                let input: RecallInput = parse_args(arguments)?;
                (self.recall.recall(&input.query, limits.max_results)?, false)
            }
            _ => return Err(ToolError::NotFound("builtin tool".to_owned())),
        };
        finish_output(value, truncated, &limits)
    }

    fn skeleton(
        &self,
        root: &Path,
        path: &str,
        limits: &ToolLimits,
        parser: &ParserPool,
    ) -> Result<SkeletonOutput, ToolError> {
        let input = ReadFileInput {
            path: path.to_owned(),
            offset: 0,
            limit: None,
        };
        let read = read_file(root, &input, limits)?;
        if read.binary {
            return Err(ToolError::InvalidInput(
                "binary files have no source skeleton".to_owned(),
            ));
        }
        if read.truncated {
            return Err(ToolError::InvalidInput(
                "file exceeds the source limit".to_owned(),
            ));
        }
        let text = read
            .content
            .ok_or_else(|| ToolError::InvalidInput("file content unavailable".to_owned()))?;
        let resolved = resolve_in_workspace(root, path)
            .map_err(|_| ToolError::Forbidden("workspace path denied".to_owned()))?;
        let parsed = parser.parse_file(&resolved, text.as_bytes()).map_err(|_| {
            ToolError::InvalidInput("source language is unsupported or parse failed".to_owned())
        })?;
        let skeleton = polaris_code_intel::skeleton(&parsed)
            .map_err(|_| ToolError::InvalidInput("source skeleton exceeded limits".to_owned()))?;
        Ok(SkeletonOutput {
            path: path.replace('\\', "/"),
            items: skeleton
                .items
                .into_iter()
                .map(|item| SkeletonRow {
                    kind: format!("{:?}", item.kind),
                    name: item.name,
                    signature: item.signature,
                    depth: item.depth,
                })
                .collect(),
        })
    }

    fn repo_map(
        &self,
        root: &Path,
        input: &RepoMapInput,
        limits: &ToolLimits,
        parser: &ParserPool,
    ) -> Result<(RepoMapOutput, bool), ToolError> {
        if input.max_tokens == 0 {
            return Err(ToolError::InvalidInput(
                "max_tokens must be positive".to_owned(),
            ));
        }
        let canonical_root = root
            .canonicalize()
            .map_err(|_| ToolError::Io("workspace root failed".to_owned()))?;
        let start = Instant::now();
        let mut graph = SymbolGraph::new();
        let mut files = 0_u64;
        let max_files = u64::try_from(limits.max_results).map_err(|_| {
            ToolError::InvalidInput("file result bound exceeds platform size".to_owned())
        })?;
        let mut truncated = false;
        let mut builder = WalkBuilder::new(&canonical_root);
        builder
            .hidden(false)
            .follow_links(false)
            .add_custom_ignore_filename(".polarisignore");
        for entry in builder.build() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    truncated = true;
                    continue;
                }
            };
            if start.elapsed() >= limits.timeout || files >= max_files {
                truncated = true;
                break;
            }
            if entry.depth() == 0
                || entry.path_is_symlink()
                || !entry.file_type().is_some_and(|kind| kind.is_file())
            {
                continue;
            }
            let relative = match entry.path().strip_prefix(&canonical_root) {
                Ok(relative) => relative,
                Err(_) => continue,
            };
            let relative_text = relative.to_string_lossy().replace('\\', "/");
            let resolved = match resolve_in_workspace(&canonical_root, &relative_text) {
                Ok(path) => path,
                Err(_) => continue,
            };
            let metadata = match std::fs::metadata(&resolved) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() > limits.max_read_bytes {
                truncated = true;
                continue;
            }
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&resolved).and_then(|file| {
                file.take(limits.max_read_bytes.saturating_add(1))
                    .read_to_end(&mut bytes)
            });
            match read {
                Ok(_)
                    if u64::try_from(bytes.len())
                        .is_ok_and(|size| size <= limits.max_read_bytes) => {}
                Ok(_) => {
                    truncated = true;
                    continue;
                }
                Err(_) => {
                    truncated = true;
                    continue;
                }
            }
            if bytes
                .get(..bytes.len().min(8_192))
                .is_some_and(|head| head.contains(&0))
            {
                continue;
            }
            if let Ok(parsed) = parser.parse_file(&resolved, &bytes)
                && graph.add_file(relative, &parsed).is_ok()
            {
                files = files.saturating_add(1);
            }
        }
        let max_tokens = TokenCount::new(input.max_tokens);
        let focus_files = input
            .focus_files
            .iter()
            .filter_map(|path| {
                WorkspacePath::parse(path)
                    .ok()
                    .map(|path| PathBuf::from(path.as_str()))
            })
            .collect();
        let map = repo_map(
            &graph,
            &RepoMapBudget {
                max_tokens,
                focus_files,
            },
        )
        .map_err(|_| ToolError::InvalidInput("repository map failed".to_owned()))?;
        Ok((
            RepoMapOutput {
                text: map.render(),
                token_estimate: map.token_estimate.get(),
                entries: map
                    .entries
                    .into_iter()
                    .map(|entry| RepoMapRow {
                        path: entry.path.to_string_lossy().replace('\\', "/"),
                        rank: entry.rank,
                        outline: entry.outline,
                    })
                    .collect(),
                files_indexed: files,
            },
            truncated,
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PathInput {
    path: String,
}
#[derive(Debug, Clone, Deserialize)]
struct RepoMapInput {
    #[serde(default)]
    focus_files: Vec<String>,
    max_tokens: u32,
}
#[derive(Debug, Clone, Deserialize)]
struct ExpandInput {
    reference: String,
}
#[derive(Debug, Clone, Deserialize)]
struct RecallInput {
    query: String,
}
#[derive(Debug, Clone, Deserialize)]
struct DiffApplyInput {
    base: String,
    patch: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    path: String,
    contents: String,
    expected_base: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditFileInput {
    path: String,
    old_text: String,
    new_text: String,
    #[serde(default)]
    replace_all: bool,
    expected_base: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyPatchInput {
    path: String,
    patch: String,
    expected_base: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifyInput {
    argv: Vec<String>,
    shell: crate::shell::ShellFlavor,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SkeletonOutput {
    path: String,
    items: Vec<SkeletonRow>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct SkeletonRow {
    kind: String,
    name: String,
    signature: String,
    depth: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RepoMapOutput {
    text: String,
    token_estimate: u32,
    entries: Vec<RepoMapRow>,
    files_indexed: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RepoMapRow {
    path: String,
    rank: f64,
    outline: Vec<String>,
}

fn parse_args<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, ToolError> {
    serde_json::from_value(value.clone())
        .map_err(|_| ToolError::InvalidInput("tool arguments do not match schema".to_owned()))
}

fn parse_cid(value: &str) -> Result<Cid, ToolError> {
    Cid::from_hex(value)
        .map_err(|_| ToolError::InvalidInput("expected_base must be a full content id".to_owned()))
}

fn effective_limits(mut limits: ToolLimits, deadline: Option<std::time::Duration>) -> ToolLimits {
    if let Some(remaining) = deadline {
        limits.timeout = limits.timeout.min(remaining);
    }
    limits
}

fn finish_output(
    value: Value,
    truncated: bool,
    limits: &ToolLimits,
) -> Result<ToolOutput, ToolError> {
    let encoded = serde_json::to_vec(&value)
        .map_err(|_| ToolError::InvalidInput("tool result encoding failed".to_owned()))?;
    let bytes = u64::try_from(encoded.len())
        .map_err(|_| ToolError::InvalidInput("tool output size overflow".to_owned()))?;
    if bytes > limits.max_output_bytes {
        return Err(ToolError::InvalidInput(
            "tool output exceeds configured bound".to_owned(),
        ));
    }
    Ok(ToolOutput {
        value,
        truncated,
        bytes,
    })
}

/// Registered P1 tool schemas in byte-stable name order. No backend is needed to inspect schemas.
#[must_use]
pub fn builtin_specs() -> Vec<ToolSpec> {
    let read = Effects::READ_ONLY;
    let pure = Effects::PURE;
    let mut specs = vec![
        spec(
            "read_file",
            "Read a bounded workspace file; secret-bearing paths are denied.",
            r#"{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1}},"required":["path"],"additionalProperties":false}"#,
            read,
        ),
        spec(
            "list_dir",
            "List entries under the workspace while excluding symlinks and secret paths.",
            r#"{"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}"#,
            read,
        ),
        spec(
            "glob",
            "Find workspace paths using a glob and repository ignore rules.",
            r#"{"type":"object","properties":{"pattern":{"type":"string","minLength":1}},"required":["pattern"],"additionalProperties":false}"#,
            read,
        ),
        spec(
            "grep",
            "Search workspace text using a size-guarded regular expression.",
            r#"{"type":"object","properties":{"pattern":{"type":"string","minLength":1},"path":{"type":"string"},"case_insensitive":{"type":"boolean"}},"required":["pattern"],"additionalProperties":false}"#,
            read,
        ),
        spec(
            "repo_map",
            "Build a bounded symbol map from supported source files.",
            r#"{"type":"object","properties":{"focus_files":{"type":"array","items":{"type":"string"}},"max_tokens":{"type":"integer","minimum":1}},"required":["max_tokens"],"additionalProperties":false}"#,
            read,
        ),
        spec(
            "skeleton",
            "Extract signatures from one supported source file.",
            r#"{"type":"object","properties":{"path":{"type":"string","minLength":1}},"required":["path"],"additionalProperties":false}"#,
            read,
        ),
        spec(
            "json_query",
            "Select a value from caller-provided JSON using JSON Pointer.",
            r#"{"type":"object","properties":{"json":{"type":"string"},"pointer":{"type":"string"}},"required":["json"],"additionalProperties":false}"#,
            pure,
        ),
        spec(
            "regex_query",
            "Find bounded regular-expression matches in caller-provided text.",
            r#"{"type":"object","properties":{"text":{"type":"string"},"pattern":{"type":"string"},"case_insensitive":{"type":"boolean"}},"required":["text","pattern"],"additionalProperties":false}"#,
            pure,
        ),
        spec(
            "diff_apply",
            "Apply a unified patch to caller-provided text as a pure operation; no files are changed.",
            r#"{"type":"object","properties":{"base":{"type":"string"},"patch":{"type":"string"}},"required":["base","patch"],"additionalProperties":false}"#,
            pure,
        ),
        spec(
            "polaris_expand",
            "Expand a full or collision-checked short content reference.",
            r#"{"type":"object","properties":{"reference":{"type":"string","minLength":1}},"required":["reference"],"additionalProperties":false}"#,
            pure,
        ),
        spec(
            "polaris_recall",
            "Retrieve persisted relevant context through the configured recall backend.",
            r#"{"type":"object","properties":{"query":{"type":"string","minLength":1}},"required":["query"],"additionalProperties":false}"#,
            pure,
        ),
    ];
    specs.sort_by(|left, right| left.name.cmp(&right.name));
    specs
}

/// P2 schemas that are advertised only when proposal and sandbox services are explicitly bound.
#[must_use]
pub fn p2_builtin_specs() -> Vec<ToolSpec> {
    let writes = Effects {
        reads_workspace: true,
        writes_workspace: true,
        network: false,
        executes: false,
    };
    let executes = Effects {
        reads_workspace: false,
        writes_workspace: false,
        network: false,
        executes: true,
    };
    let pure = Effects::PURE;
    let mut specs = vec![
        spec(
            "write_file",
            "Propose a complete replacement for an existing UTF-8 workspace file; include its current full base CID. This submits for verification and never writes the file directly.",
            r#"{"type":"object","properties":{"path":{"type":"string","minLength":1},"contents":{"type":"string"},"expected_base":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","contents","expected_base"],"additionalProperties":false}"#,
            writes,
        ),
        spec(
            "edit_file",
            "Propose an exact text replacement in a workspace file. One match is required unless replace_all is true; include the current full base CID.",
            r#"{"type":"object","properties":{"path":{"type":"string","minLength":1},"old_text":{"type":"string","minLength":1},"new_text":{"type":"string"},"replace_all":{"type":"boolean","default":false},"expected_base":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","old_text","new_text","expected_base"],"additionalProperties":false}"#,
            writes,
        ),
        spec(
            "apply_patch",
            "Validate and propose a single-file unified patch against the current workspace file; include its full base CID. No host file is written.",
            r#"{"type":"object","properties":{"path":{"type":"string","minLength":1},"patch":{"type":"string","minLength":1},"expected_base":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","patch","expected_base"],"additionalProperties":false}"#,
            writes,
        ),
        spec(
            "classify_command",
            "Classify an explicit argv command as heavy, destructive, or unknown without executing it.",
            r#"{"type":"object","properties":{"argv":{"type":"array","minItems":1,"items":{"type":"string"}},"shell":{"type":"string","enum":["posix","power_shell","cmd"]}},"required":["argv","shell"],"additionalProperties":false}"#,
            pure,
        ),
        spec(
            "shell",
            "Run explicit argv in the runtime-owned sandbox only. Output is returned in full within the configured capture bound; it is not run on the host.",
            r#"{"type":"object","properties":{"argv":{"type":"array","minItems":1,"items":{"type":"string"}},"workdir":{"type":["string","null"]},"shell":{"type":"string","enum":["posix","power_shell","cmd"]},"timeout_ms":{"type":["integer","null"],"minimum":1,"maximum":900000}},"required":["argv","shell"],"additionalProperties":false}"#,
            executes,
        ),
    ];
    specs.sort_by(|left, right| left.name.cmp(&right.name));
    specs
}

fn spec(name: &str, description: &str, schema: &str, effects: Effects) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema_json: schema.into(),
        effects,
    }
}
