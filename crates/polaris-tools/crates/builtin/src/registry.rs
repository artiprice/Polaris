//! Stable schema catalog and enum-dispatched builtin invocation.

use std::{
    io::{Read, Write},
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
use polaris_storage_cas::CasError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ToolLimits, ToolOutput,
    patch::apply_unified_patch,
    path::{WorkspacePath, resolve_in_workspace},
    proposal::{EditFileRequest, NoProposalBackend, ProposalToolBackend},
    query::{JsonQueryInput, RegexQueryInput, json_query, regex_query},
    read::ReadFileInput,
    search::{GlobInput, GrepInput, ListDirInput},
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

/// Workspace resource presented to a trusted authorization policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessResource<'a> {
    /// Admission marker for a query rooted at the workspace itself.
    WorkspaceRoot,
    /// One exact normalized workspace-relative path being accessed or disclosed.
    WorkspacePath(&'a WorkspacePath),
}

/// Authorizes every workspace resource accessed by an authorized builtin invocation.
pub trait ToolAccessPolicy {
    /// Grant or deny access to one workspace resource.
    ///
    /// # Errors
    /// Returns a policy error when the resource is not authorized.
    fn authorize(&self, resource: AccessResource<'_>) -> Result<(), ToolError>;
}

#[derive(Debug, Clone, Copy)]
struct AllowAllAccessPolicy;

impl ToolAccessPolicy for AllowAllAccessPolicy {
    fn authorize(&self, _resource: AccessResource<'_>) -> Result<(), ToolError> {
        Ok(())
    }
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
        let bytes = match self.cas.get_bounded(&cid, max_bytes) {
            Ok(bytes) => bytes,
            Err(CasError::TooLarge { .. }) => {
                return Err(ToolError::InvalidInput(
                    "referenced content exceeds configured read bound".to_owned(),
                ));
            }
            Err(_) => return Err(ToolError::NotFound("content reference".to_owned())),
        };
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
        self.invoke_blocking_with_limits(
            name,
            arguments,
            context,
            self.limits,
            &AllowAllAccessPolicy,
            false,
        )
    }

    /// Validate and invoke one pure or read-only builtin under a caller policy and output cap.
    ///
    /// The caller cap intersects the registry cap. Recursive workspace reads preauthorize the
    /// traversed workspace paths and local ignore files before the ignore-aware walker consumes
    /// ignore contents or produces rows. Preflight observes directory names and filesystem
    /// metadata to find those resources; denied paths abort the call, including paths that ignore
    /// rules would otherwise hide. External parent/global Git ignore files are disabled here.
    /// The trusted blocking entry point retains the walker's default ignore behavior.
    /// Stable-workspace assumptions still apply between preflight and later filesystem access.
    ///
    /// # Errors
    /// Unknown tools, writes, invalid arguments, policy denials, backend failures, and results
    /// exceeding either configured output limit are rejected without returning partial output.
    pub fn invoke_authorized_bounded<A: ToolAccessPolicy>(
        &self,
        name: &str,
        arguments: &Value,
        context: &ToolContext<'_>,
        caller_max_output: u64,
        policy: &A,
    ) -> Result<ToolOutput, ToolError> {
        if caller_max_output == 0 {
            return Err(ToolError::InvalidInput(
                "caller output bound must be positive".to_owned(),
            ));
        }
        let spec = self
            .get(name)
            .ok_or_else(|| ToolError::NotFound("builtin tool".to_owned()))?;
        if spec.effects.writes_workspace || spec.effects.network || spec.effects.executes {
            return Err(ToolError::Forbidden(
                "builtin is not available in read-only invocation".to_owned(),
            ));
        }
        let _bounded_arguments = serialize_bounded(
            arguments,
            self.limits.max_input_bytes,
            "tool arguments exceed configured bound",
        )?;
        validate_readonly_arguments(name, arguments)?;
        let mut limits = self.limits;
        limits.max_output_bytes = limits.max_output_bytes.min(caller_max_output);
        limits.max_read_bytes = limits.max_read_bytes.min(limits.max_output_bytes);
        self.invoke_blocking_with_limits(name, arguments, context, limits, policy, true)
    }

    fn invoke_blocking_with_limits<A: ToolAccessPolicy>(
        &self,
        name: &str,
        arguments: &Value,
        context: &ToolContext<'_>,
        call_limits: ToolLimits,
        policy: &A,
        authorized_traversal: bool,
    ) -> Result<ToolOutput, ToolError> {
        if self.get(name).is_none() {
            return Err(ToolError::NotFound("builtin tool".to_owned()));
        }
        if matches!(name, "write_file" | "edit_file" | "apply_patch" | "shell") {
            return Err(ToolError::InvalidInput(
                "this builtin requires asynchronous invocation".to_owned(),
            ));
        }
        let _input_bytes = serialize_bounded(
            arguments,
            call_limits.max_input_bytes,
            "tool arguments exceed configured bound",
        )?;
        let limits = effective_limits(call_limits, context.deadline);
        let query_uses_workspace_root = match name {
            "list_dir" | "grep" => matches!(arguments.get("path"), None | Some(Value::Null)),
            "glob" | "repo_map" => true,
            _ => false,
        };
        if query_uses_workspace_root {
            policy.authorize(AccessResource::WorkspaceRoot)?;
        }
        let parser = ParserPool::with_limits(ParseLimits {
            max_source_bytes: usize::try_from(limits.max_read_bytes).map_err(|_| {
                ToolError::InvalidInput("read limit exceeds platform size".to_owned())
            })?,
            timeout: limits.timeout,
        });
        let (value, truncated) = match name {
            "read_file" => {
                let input: ReadFileInput = parse_args(arguments)?;
                let output = crate::read::read_file_authorized(
                    context.workspace_root,
                    &input,
                    &limits,
                    |path| authorize_workspace_path(policy, path),
                )?;
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
                let output = if authorized_traversal {
                    crate::search::list_dir_authorized(
                        context.workspace_root,
                        &input,
                        &limits,
                        |path| authorize_workspace_path(policy, path),
                    )?
                } else {
                    crate::search::list_dir(context.workspace_root, &input, &limits)?
                };
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
                let output = if authorized_traversal {
                    crate::search::glob_authorized(
                        context.workspace_root,
                        &input,
                        &limits,
                        |path| authorize_workspace_path(policy, path),
                    )?
                } else {
                    crate::search::glob(context.workspace_root, &input, &limits)?
                };
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
                let output = if authorized_traversal {
                    crate::search::grep_authorized(
                        context.workspace_root,
                        &input,
                        &limits,
                        |path| authorize_workspace_path(policy, path),
                    )?
                } else {
                    crate::search::grep(context.workspace_root, &input, &limits)?
                };
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
                let output = self.skeleton(
                    context.workspace_root,
                    &input.path,
                    &limits,
                    &parser,
                    policy,
                )?;
                (
                    serde_json::to_value(output).map_err(|_| {
                        ToolError::InvalidInput("skeleton result encoding failed".to_owned())
                    })?,
                    false,
                )
            }
            "repo_map" => {
                let input: RepoMapInput = parse_args(arguments)?;
                let (output, truncated) = self.repo_map(
                    context.workspace_root,
                    &input,
                    &limits,
                    &parser,
                    policy,
                    authorized_traversal,
                )?;
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
        policy: &impl ToolAccessPolicy,
    ) -> Result<SkeletonOutput, ToolError> {
        let input = ReadFileInput {
            path: path.to_owned(),
            offset: 0,
            limit: None,
        };
        let read = crate::read::read_file_authorized(root, &input, limits, |resource_path| {
            authorize_workspace_path(policy, resource_path)
        })?;
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
        policy: &impl ToolAccessPolicy,
        authorized_traversal: bool,
    ) -> Result<(RepoMapOutput, bool), ToolError> {
        if input.max_tokens == 0 {
            return Err(ToolError::InvalidInput(
                "max_tokens must be positive".to_owned(),
            ));
        }
        for path in &input.focus_files {
            authorize_workspace_path(policy, path)?;
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
        if authorized_traversal {
            crate::search::preauthorize_ignore_files(
                &canonical_root,
                &canonical_root,
                limits,
                start,
                &mut |path| authorize_workspace_path(policy, path),
            )?;
        }
        let mut builder = WalkBuilder::new(&canonical_root);
        builder
            .hidden(false)
            .follow_links(false)
            .add_custom_ignore_filename(".polarisignore");
        if authorized_traversal {
            crate::search::configure_authorized_walker(&mut builder, &canonical_root);
        }
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
            if entry.depth() == 0 || entry.path_is_symlink() {
                continue;
            }
            let relative = match entry.path().strip_prefix(&canonical_root) {
                Ok(relative) => relative,
                Err(_) => continue,
            };
            let relative_text = relative.to_string_lossy().replace('\\', "/");
            if WorkspacePath::parse(&relative_text).is_err() {
                continue;
            }
            authorize_workspace_path(policy, &relative_text)?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
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
#[serde(deny_unknown_fields)]
struct PathInput {
    path: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoMapInput {
    #[serde(default)]
    focus_files: Vec<String>,
    max_tokens: u32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandInput {
    reference: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecallInput {
    query: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Parse and reject unknown fields for a pure or read-only builtin without performing I/O.
///
/// # Errors
/// Returns the same input error class used by invocation for malformed or unsupported tools.
pub fn validate_readonly_arguments(name: &str, arguments: &Value) -> Result<(), ToolError> {
    match name {
        "read_file" => parse_args::<ReadFileInput>(arguments).map(|_| ()),
        "list_dir" => parse_args::<ListDirInput>(arguments).map(|_| ()),
        "glob" => parse_args::<GlobInput>(arguments).map(|_| ()),
        "grep" => parse_args::<GrepInput>(arguments).map(|_| ()),
        "skeleton" => parse_args::<PathInput>(arguments).map(|_| ()),
        "repo_map" => parse_args::<RepoMapInput>(arguments).map(|_| ()),
        "json_query" => parse_args::<JsonQueryInput>(arguments).map(|_| ()),
        "regex_query" => parse_args::<RegexQueryInput>(arguments).map(|_| ()),
        "diff_apply" => parse_args::<DiffApplyInput>(arguments).map(|_| ()),
        "polaris_expand" => parse_args::<ExpandInput>(arguments).map(|_| ()),
        "polaris_recall" => parse_args::<RecallInput>(arguments).map(|_| ()),
        "classify_command" => parse_args::<ClassifyInput>(arguments).map(|_| ()),
        "write_file" | "edit_file" | "apply_patch" | "shell" => Err(ToolError::Forbidden(
            "builtin is not available in read-only invocation".to_owned(),
        )),
        _ => Err(ToolError::NotFound("builtin tool".to_owned())),
    }
}

fn authorize_workspace_path(policy: &impl ToolAccessPolicy, value: &str) -> Result<(), ToolError> {
    let path = WorkspacePath::parse(value).map_err(crate::read::path_tool_error)?;
    policy.authorize(AccessResource::WorkspacePath(&path))
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
    let encoded = serialize_bounded(
        &value,
        limits.max_output_bytes,
        "tool output exceeds configured bound",
    )?;
    let bytes = u64::try_from(encoded.len())
        .map_err(|_| ToolError::InvalidInput("tool output size overflow".to_owned()))?;
    Ok(ToolOutput {
        value,
        truncated,
        bytes,
    })
}

fn serialize_bounded(value: &Value, limit: u64, message: &str) -> Result<Vec<u8>, ToolError> {
    let mut writer = BoundedJsonWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| ToolError::InvalidInput(message.to_owned()))?;
    Ok(writer.bytes)
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: u64,
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let current = u64::try_from(self.bytes.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "output size overflow")
        })?;
        let incoming = u64::try_from(buffer.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "output size overflow")
        })?;
        let Some(next) = current.checked_add(incoming) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "output size overflow",
            ));
        };
        if next > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "output limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
