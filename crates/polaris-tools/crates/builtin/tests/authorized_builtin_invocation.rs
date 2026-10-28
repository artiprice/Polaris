use std::{collections::BTreeSet, time::Duration};

use polaris_sdk::{
    ids::{FrameId, SessionId},
    registry::{ToolContext, ToolError},
};
use polaris_storage_cas::{Cas, CasConfig};
use polaris_tools_builtin::{
    AccessResource, CasRefResolver, NoProposalBackend, NoShellBackend, RecallBackend,
    ToolAccessPolicy, ToolLimits, ToolRegistry, WorkspacePath,
};

#[derive(Debug, Clone)]
struct DenyPaths {
    denied: BTreeSet<String>,
    deny_root: bool,
}

impl DenyPaths {
    fn allowing_all() -> Self {
        Self {
            denied: BTreeSet::new(),
            deny_root: false,
        }
    }

    fn denying(path: &str) -> Self {
        Self {
            denied: BTreeSet::from([path.to_owned()]),
            deny_root: false,
        }
    }

    fn denying_root() -> Self {
        Self {
            denied: BTreeSet::new(),
            deny_root: true,
        }
    }
}

impl ToolAccessPolicy for DenyPaths {
    fn authorize(&self, resource: AccessResource<'_>) -> Result<(), ToolError> {
        match resource {
            AccessResource::WorkspaceRoot if self.deny_root => {
                Err(ToolError::Forbidden("workspace access denied".to_owned()))
            }
            AccessResource::WorkspaceRoot => Ok(()),
            AccessResource::WorkspacePath(path) if self.denied.contains(path.as_str()) => {
                Err(ToolError::Forbidden("workspace path denied".to_owned()))
            }
            AccessResource::WorkspacePath(_) => Ok(()),
        }
    }
}

#[derive(Debug, Clone)]
struct AllowOnlyPath(String);

impl ToolAccessPolicy for AllowOnlyPath {
    fn authorize(&self, resource: AccessResource<'_>) -> Result<(), ToolError> {
        match resource {
            AccessResource::WorkspaceRoot => Err(ToolError::Forbidden(
                "workspace root access denied".to_owned(),
            )),
            AccessResource::WorkspacePath(path) if path.as_str() == self.0 => Ok(()),
            AccessResource::WorkspacePath(_) => {
                Err(ToolError::Forbidden("workspace path denied".to_owned()))
            }
        }
    }
}

#[derive(Debug)]
struct NoRecall;

impl RecallBackend for NoRecall {
    fn recall(&self, _query: &str, _max_results: usize) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::NotFound("recall backend".to_owned()))
    }
}

fn limits(max_output_bytes: u64, max_read_bytes: u64) -> ToolLimits {
    ToolLimits {
        max_read_bytes,
        max_results: 32,
        max_output_bytes,
        max_input_bytes: 32_768,
        regex_size_limit: 1_048_576,
        timeout: Duration::from_secs(2),
    }
}

fn registry(
    cas: Cas,
    max_output_bytes: u64,
    max_read_bytes: u64,
) -> Result<ToolRegistry<CasRefResolver, NoRecall>, ToolError> {
    ToolRegistry::new(
        limits(max_output_bytes, max_read_bytes),
        CasRefResolver::new(cas),
        NoRecall,
    )
}

fn context(root: &std::path::Path) -> ToolContext<'_> {
    ToolContext {
        session: SessionId::from_u128(1),
        frame: FrameId::from_u128(2),
        workspace_root: root,
        deadline: None,
    }
}

fn empty_cas() -> Result<(tempfile::TempDir, Cas), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let config = CasConfig::new(directory.path().join("cas"));
    Ok((directory, Cas::open(config)?))
}

#[test]
fn recursive_reads_check_each_resource() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join("nested"))?;
    std::fs::write(workspace.path().join("first.txt"), "needle first\n")?;
    std::fs::write(
        workspace.path().join("nested/second.txt"),
        "needle second\n",
    )?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "grep",
        &serde_json::json!({"pattern":"needle"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::denying("nested/second.txt"),
    );
    assert!(matches!(result, Err(ToolError::Forbidden(_))));
    Ok(())
}

#[test]
fn ignorefile_denial_precedes_rule_use() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join(".polarisignore"), "visible.txt\n")?;
    std::fs::write(workspace.path().join("visible.txt"), "must not be hidden\n")?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "glob",
        &serde_json::json!({"pattern":"**/*.txt"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::denying(".polarisignore"),
    );
    assert!(matches!(result, Err(ToolError::Forbidden(_))));
    Ok(())
}

#[test]
fn ignorefile_preflight_rejects_oversized_case_insensitive_name()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join(".POLARISIGNORE"), "x".repeat(32))?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 8)?;
    let result = tools.invoke_authorized_bounded(
        "glob",
        &serde_json::json!({"pattern":"**/*.txt"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn ignorefile_preflight_rejects_directory_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join(".GiTiGnOrE"))?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "glob",
        &serde_json::json!({"pattern":"**/*.txt"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[cfg(unix)]
#[test]
fn ignorefile_preflight_rejects_fifo_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let fifo_path = workspace.path().join(".polarisignore");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()?;
    if !status.success() {
        return Err("mkfifo could not create the ignore fixture".into());
    }
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "glob",
        &serde_json::json!({"pattern":"**/*.txt"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[cfg(unix)]
#[test]
fn ignorefile_preflight_rejects_symlink_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("rules"), "visible.txt\n")?;
    std::os::unix::fs::symlink(
        workspace.path().join("rules"),
        workspace.path().join(".GiTiGnOrE"),
    )?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "glob",
        &serde_json::json!({"pattern":"**/*.txt"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn caller_output_cap_intersects_registry_limit() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas.clone(), 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "json_query",
        &serde_json::json!({"json":"{\"value\":\"a long value\"}","pointer":"/value"}),
        &context(workspace.path()),
        8,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    let registry_limited = registry(cas, 8, 4_096)?;
    let result = registry_limited.invoke_authorized_bounded(
        "json_query",
        &serde_json::json!({"json":"{\"value\":\"a long value\"}","pointer":"/value"}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn oversized_cas_reference_is_rejected_before_decode() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let cas_root = tempfile::tempdir()?;
    let mut config = CasConfig::new(cas_root.path());
    config.min_compress_bytes = usize::MAX;
    config.max_blob_bytes = 1_024;
    let cas = Cas::open(config)?;
    let cid = cas.put(b"known-valid-content-that-is-larger-than-the-read-cap")?;
    std::fs::write(
        cas.blob_path(&cid),
        b"corrupt raw blob payload longer than cap",
    )?;
    let tools = registry(cas, 16_384, 8)?;
    let result = tools.invoke_authorized_bounded(
        "polaris_expand",
        &serde_json::json!({"reference":cid.to_hex()}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(
        result,
        Err(ToolError::InvalidInput(message))
            if message == "referenced content exceeds configured read bound"
    ));
    Ok(())
}

#[test]
fn within_bound_cas_corruption_remains_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let cas_root = tempfile::tempdir()?;
    let mut config = CasConfig::new(cas_root.path());
    config.min_compress_bytes = usize::MAX;
    config.max_blob_bytes = 1_024;
    let cas = Cas::open(config)?;
    let cid = cas.put(b"valid stored content")?;
    std::fs::write(cas.blob_path(&cid), b"x")?;
    let tools = registry(cas, 16_384, 8)?;
    let result = tools.invoke_authorized_bounded(
        "polaris_expand",
        &serde_json::json!({"reference":cid.to_hex()}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(
        result,
        Err(ToolError::NotFound(message)) if message == "content reference"
    ));
    Ok(())
}

#[test]
fn zero_output_cap_rejects_before_cas_access() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "polaris_expand",
        &serde_json::json!({"reference":"0000000000000000000000000000000000000000000000000000000000000000"}),
        &context(workspace.path()),
        0,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn authorization_errors_never_return_partial_success() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("allowed.txt"), "present\n")?;
    std::fs::write(workspace.path().join("blocked.txt"), "secret name\n")?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "list_dir",
        &serde_json::json!({}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::denying("blocked.txt"),
    );
    assert!(matches!(result, Err(ToolError::Forbidden(_))));
    Ok(())
}

#[test]
fn exact_directory_token_does_not_authorize_descendants() -> Result<(), Box<dyn std::error::Error>>
{
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join("parent"))?;
    std::fs::write(workspace.path().join("parent/file.txt"), "private\n")?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "list_dir",
        &serde_json::json!({"path":"parent"}),
        &context(workspace.path()),
        16_384,
        &AllowOnlyPath("parent".to_owned()),
    );
    assert!(matches!(result, Err(ToolError::Forbidden(_))));
    Ok(())
}

#[test]
fn exact_file_token_reads_without_parent_grants() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join("parent"))?;
    std::fs::write(workspace.path().join("parent/file.txt"), "private\n")?;
    let path = WorkspacePath::parse("parent/file.txt")?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "read_file",
        &serde_json::json!({"path":path.as_str()}),
        &context(workspace.path()),
        16_384,
        &AllowOnlyPath("parent/file.txt".to_owned()),
    )?;
    assert_eq!(
        result.value.get("content"),
        Some(&serde_json::json!("private\n"))
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn read_file_rejects_fifo_before_open() -> Result<(), Box<dyn std::error::Error>> {
    use std::{sync::mpsc, thread};

    use polaris_tools_builtin::{ReadFileInput, read_file};

    let workspace = tempfile::tempdir()?;
    let fifo_path = workspace.path().join("stream.pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()?;
    if !status.success() {
        return Err("mkfifo could not create the FIFO fixture".into());
    }
    let root = workspace.path().to_path_buf();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = read_file(
            &root,
            &ReadFileInput {
                path: "stream.pipe".to_owned(),
                offset: 0,
                limit: None,
            },
            &limits(16_384, 4_096),
        );
        match sender.send(result) {
            Ok(()) => {}
            Err(error) => drop(error),
        }
    });
    let result = receiver.recv_timeout(Duration::from_secs(1))?;
    assert!(matches!(result, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn workspace_root_denial_precedes_reads() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let result = tools.invoke_authorized_bounded(
        "list_dir",
        &serde_json::json!({}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::denying_root(),
    );
    assert!(matches!(result, Err(ToolError::Forbidden(_))));
    Ok(())
}

#[test]
fn readonly_entry_rejects_writes_and_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = ToolRegistry::new_with_services(
        limits(16_384, 4_096),
        CasRefResolver::new(cas),
        NoRecall,
        NoProposalBackend,
        NoShellBackend,
    )?;
    let write = tools.invoke_authorized_bounded(
        "write_file",
        &serde_json::json!({}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(write, Err(ToolError::Forbidden(_))));
    let unknown = tools.invoke_authorized_bounded(
        "read_file",
        &serde_json::json!({"path":"missing.txt","extra":true}),
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    );
    assert!(matches!(unknown, Err(ToolError::InvalidInput(_))));
    Ok(())
}

#[test]
fn exact_output_boundary_is_accepted_and_json_is_escaped() -> Result<(), Box<dyn std::error::Error>>
{
    let workspace = tempfile::tempdir()?;
    let (_cas_root, cas) = empty_cas()?;
    let tools = registry(cas, 16_384, 4_096)?;
    let arguments =
        serde_json::json!({"json":"{\"value\":\"line\\nquote\\\"\"}","pointer":"/value"});
    let output = tools.invoke_authorized_bounded(
        "json_query",
        &arguments,
        &context(workspace.path()),
        16_384,
        &DenyPaths::allowing_all(),
    )?;
    let encoded_len = u64::try_from(serde_json::to_vec(&output.value)?.len())?;
    assert_eq!(output.bytes, encoded_len);
    let exact = tools.invoke_authorized_bounded(
        "json_query",
        &arguments,
        &context(workspace.path()),
        output.bytes,
        &DenyPaths::allowing_all(),
    )?;
    assert_eq!(exact.bytes, output.bytes);
    Ok(())
}
