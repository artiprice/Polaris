use proptest::prelude::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
    time::Duration,
};

use polaris_sdk::{
    ids::{FrameId, SessionId},
    registry::ToolContext,
};
use polaris_tools_builtin::{
    GlobInput, GrepInput, JsonQueryInput, NoProposalBackend, NoShellBackend, PathError,
    ReadFileInput, RecallBackend, RefResolver, RegexQueryInput, ToolLimits, ToolRegistry,
    apply_unified_patch, glob, grep, json_query, p2_builtin_specs, read_file, regex_query,
    resolve_in_workspace,
};

struct NoRefs;
impl RefResolver for NoRefs {
    fn expand(
        &self,
        _reference: &str,
        _max_bytes: u64,
    ) -> Result<serde_json::Value, polaris_sdk::registry::ToolError> {
        Err(polaris_sdk::registry::ToolError::NotFound(
            "content reference".to_owned(),
        ))
    }
}

struct NoRecall;
impl RecallBackend for NoRecall {
    fn recall(
        &self,
        _query: &str,
        _max_results: usize,
    ) -> Result<serde_json::Value, polaris_sdk::registry::ToolError> {
        Err(polaris_sdk::registry::ToolError::NotFound(
            "recall backend".to_owned(),
        ))
    }
}

fn test_limits() -> ToolLimits {
    ToolLimits {
        max_read_bytes: 16_384,
        max_results: 32,
        max_output_bytes: 32_768,
        max_input_bytes: 32_768,
        regex_size_limit: 1_048_576,
        timeout: Duration::from_secs(2),
    }
}

#[test]
fn rejects_dotdot_and_absolute_escapes() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    assert!(resolve_in_workspace(root.path(), "../outside").is_err());
    assert!(resolve_in_workspace(root.path(), "C:/Windows/System32").is_err());
    assert!(resolve_in_workspace(root.path(), "//server/share/file").is_err());
    Ok(())
}

#[test]
fn rejects_windows_ads_and_reserved_names() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    assert!(matches!(
        resolve_in_workspace(root.path(), "file.txt:secret"),
        Err(PathError::Invalid(_))
    ));
    assert!(matches!(
        resolve_in_workspace(root.path(), "NUL.txt"),
        Err(PathError::Invalid(_))
    ));
    assert!(matches!(
        resolve_in_workspace(root.path(), "folder./file"),
        Err(PathError::Invalid(_))
    ));
    Ok(())
}

#[test]
fn secret_directories_and_credential_files_are_denied() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    for path in [
        ".aws/credentials",
        ".ssh/id_ed25519",
        ".gnupg/private-keys-v1.d/key",
        ".polaris/approvals.json",
        "config/.npmrc",
        "certs/client.p12",
    ] {
        assert!(matches!(
            resolve_in_workspace(root.path(), path),
            Err(PathError::SecretDenied)
        ));
    }
    Ok(())
}

#[test]
fn applies_context_add_delete_without_touching_base() -> Result<(), Box<dyn std::error::Error>> {
    let base = b"one\r\ntwo\r\n";
    let patch = "@@ -1,2 +1,2 @@\n one\n-two\n+three\n";
    let output = apply_unified_patch(base, patch)?;
    assert_eq!(output, b"one\r\nthree\r\n");
    assert_eq!(base, b"one\r\ntwo\r\n");
    Ok(())
}

#[test]
fn rejects_stale_patch_context() {
    let base = b"one\ntwo\n";
    let patch = "@@ -1,2 +1,2 @@\n one\n-wrong\n+three\n";
    assert!(apply_unified_patch(base, patch).is_err());
}

#[test]
fn path_resolver_requires_existing_targets() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let path = match resolve_in_workspace(root.path(), "child.txt") {
        Ok(_) => return Err("missing path unexpectedly resolved".into()),
        Err(error) => error,
    };
    assert!(matches!(path, PathError::NotFound));
    Ok(())
}

#[test]
fn reads_utf16_bom_and_preserves_newline_facts() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("notes.txt");
    std::fs::write(
        &path,
        [b"\xFF\xFE".as_slice(), b"a\0\r\0\n\0b\0".as_slice()].concat(),
    )?;
    let output = read_file(
        root.path(),
        &ReadFileInput {
            path: "notes.txt".to_owned(),
            offset: 0,
            limit: None,
        },
        &test_limits(),
    )?;
    assert_eq!(output.encoding, "UTF-16LE");
    assert_eq!(output.content.as_deref(), Some("a\r\nb"));
    assert_eq!(output.crlf_lines, 1);
    assert!(!output.binary);
    Ok(())
}

#[test]
fn read_and_search_deny_secrets_and_skip_binary() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join(".env"), "TOKEN=hidden\n")?;
    std::fs::write(root.path().join("visible.txt"), b"needle here\n\0binary")?;
    assert!(
        read_file(
            root.path(),
            &ReadFileInput {
                path: ".env".to_owned(),
                offset: 0,
                limit: None
            },
            &test_limits()
        )
        .is_err()
    );
    let matches = grep(
        root.path(),
        &GrepInput {
            pattern: "needle".to_owned(),
            path: None,
            case_insensitive: false,
        },
        &test_limits(),
    )?;
    assert!(matches.rows.is_empty());
    Ok(())
}

#[test]
fn glob_honors_custom_ignore_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join(".polarisignore"), "ignored.txt\n")?;
    std::fs::write(root.path().join("visible.txt"), "visible")?;
    std::fs::write(root.path().join("ignored.txt"), "hidden")?;
    let output = glob(
        root.path(),
        &GlobInput {
            pattern: "*.txt".to_owned(),
        },
        &test_limits(),
    )?;
    let paths: Vec<&str> = output.rows.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(paths, vec!["visible.txt"]);
    Ok(())
}

#[test]
fn json_pointer_and_regex_query_are_bounded_and_exact() -> Result<(), Box<dyn std::error::Error>> {
    let selected = json_query(&JsonQueryInput {
        json: r#"{"items":["α",2]}"#.to_owned(),
        pointer: "/items/0".to_owned(),
    })?;
    assert_eq!(selected.value, serde_json::Value::String("α".to_owned()));
    let found = regex_query(
        &RegexQueryInput {
            text: "a α a".to_owned(),
            pattern: "a".to_owned(),
            case_insensitive: false,
        },
        1,
        1_048_576,
    )?;
    assert_eq!(found.matches.len(), 1);
    assert!(found.truncated);
    assert_eq!(
        found.matches.first().map(|item| (item.start, item.end)),
        Some((0, 1))
    );
    Ok(())
}

#[test]
fn rejects_malformed_patch_counts_and_retains_missing_final_newline() {
    assert!(apply_unified_patch(b"one\n", "@@ -1,2 +1,2 @@\n one\n").is_err());
    let candidate = apply_unified_patch(
        b"one",
        "@@ -1 +1 @@\n-one\n+two\n\\ No newline at end of file\n",
    );
    assert_eq!(candidate.as_deref(), Ok(b"two".as_slice()));
    assert!(apply_unified_patch(b"", "@@ -0,0 +1 @@\n\n").is_err());
    assert!(apply_unified_patch(b"x\n", "--- a/one\n+++ b/two\n@@ -1 +1 @@\n-x\n+y\n").is_err());
    assert!(
        apply_unified_patch(
            b"x\n",
            "--- a/one\n+++ b/one\n@@ -1 +1 @@\n-x\n+y\n--- a/two\n+++ b/two\n@@ -1 +1 @@\n-x\n+z\n"
        )
        .is_err()
    );
}

#[test]
fn registry_specs_are_stable_and_schema_backed() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ToolRegistry::new(test_limits(), NoRefs, NoRecall)?;
    let names: Vec<&str> = registry
        .specs()
        .iter()
        .map(|spec| spec.name.as_ref())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    assert_eq!(names.len(), 11);
    for spec in registry.specs() {
        let schema: serde_json::Value = serde_json::from_str(spec.input_schema_json.as_ref())?;
        assert_eq!(
            schema.get("type"),
            Some(&serde_json::Value::String("object".to_owned()))
        );
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::Value::Bool(false))
        );
    }
    assert!(registry.get("write_file").is_none());
    Ok(())
}

#[test]
fn p2_registry_advertises_only_explicitly_bound_proposal_and_sandbox_services()
-> Result<(), Box<dyn std::error::Error>> {
    let registry = ToolRegistry::new_with_services(
        test_limits(),
        NoRefs,
        NoRecall,
        NoProposalBackend,
        NoShellBackend,
    )?;
    let names: Vec<&str> = registry
        .specs()
        .iter()
        .map(|spec| spec.name.as_ref())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    for required in [
        "write_file",
        "edit_file",
        "apply_patch",
        "shell",
        "classify_command",
    ] {
        assert!(
            registry.get(required).is_some(),
            "missing P2 spec {required}"
        );
    }
    let proposals = p2_builtin_specs()
        .into_iter()
        .filter(|spec| spec.effects.writes_workspace)
        .count();
    assert_eq!(proposals, 3);
    for spec in registry.specs() {
        let schema: serde_json::Value = serde_json::from_str(spec.input_schema_json.as_ref())?;
        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false)
        );
    }
    let root = tempfile::tempdir()?;
    let context = ToolContext {
        session: SessionId::from_u128(1),
        frame: FrameId::from_u128(2),
        workspace_root: root.path(),
        deadline: None,
    };
    let shell_error = block_on(registry.invoke(
        "shell",
        &serde_json::json!({"argv":["echo","no host"],"shell":"posix"}),
        &context,
    ))
    .err()
    .ok_or("disabled shell unexpectedly returned output")?;
    assert!(matches!(
        shell_error,
        polaris_sdk::registry::ToolError::Forbidden(_)
    ));
    Ok(())
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

proptest! {
    #[test]
    fn unified_patch_parser_never_panics_on_arbitrary_inputs(base in proptest::collection::vec(any::<u8>(), 0..256), patch in any::<String>()) {
        let _ = apply_unified_patch(&base, &patch);
    }
}
