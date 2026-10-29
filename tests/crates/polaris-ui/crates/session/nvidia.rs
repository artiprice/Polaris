//! P5 NVIDIA knowledge at the composition root: `$nvidia(...)` from the kernel's directive
//! state, `/nv-profile` and `/nv-verify` on the sample repositories in
//! `tests/fixtures/nvidia`, and the context a `$nvidia(auto)` turn carries.

use std::path::{Path, PathBuf};

use polaris_kernel::directives::directive::NvidiaMode;
use polaris_nvidia_docs::{ComputeCapability, Gpu, auto_profile};
use polaris_sdk::ids::SessionId;
use polaris_ui_session::OfflineSession;
use polaris_ui_session::commands::LocalCommand;
use polaris_ui_session::nvidia::{
    context_for_turn, nv_verify, nvidia_mode, profile_lines, verify_lines,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tests/fixtures/nvidia")
}

/// Copy a sample repository into a temporary workspace, dropping the `.fixture` suffix that
/// keeps this repository itself from looking like a CUDA project.
fn materialize(name: &str) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    let mut stack = vec![fixtures().join(name)];
    let root = fixtures().join(name);
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
            let relative = relative.strip_suffix(".fixture").unwrap_or(&relative);
            let destination = target.path().join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, destination)?;
        }
    }
    Ok(target)
}

#[test]
fn nv_verify_reports_each_planted_mistake_and_skips_vendored_code() -> TestResult {
    let workspace = materialize("cuda-vector-add")?;
    let report = nv_verify(workspace.path())?;
    assert_eq!(
        report.checked, 2,
        "vector_add.cu and checked.cu; third_party/ is skipped"
    );
    let found: Vec<(&str, usize, &str)> = report
        .findings()
        .map(|(path, finding)| (path, finding.line, finding.rule))
        .collect();
    assert_eq!(
        found,
        [
            ("src/vector_add.cu", 7, "missing-grid-bounds"),
            ("src/vector_add.cu", 14, "strided-global-access"),
            ("src/vector_add.cu", 19, "shared-bank-conflict"),
            ("src/vector_add.cu", 31, "barrier-in-divergent-branch"),
            ("src/vector_add.cu", 40, "unchecked-api-call"),
            ("src/vector_add.cu", 41, "unchecked-api-call"),
            ("src/vector_add.cu", 42, "unchecked-api-call"),
            ("src/vector_add.cu", 43, "unchecked-launch"),
            ("src/vector_add.cu", 44, "unchecked-api-call"),
            ("src/vector_add.cu", 45, "unchecked-api-call"),
            ("src/vector_add.cu", 46, "unchecked-api-call"),
            ("src/vector_add.cu", 47, "unchecked-api-call"),
        ],
        "{report:#?}"
    );
    assert_eq!(report.warnings(), 10);
    let lines = verify_lines(&report);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Checked 2 CUDA sources: 10 warnings, 2 suggestions.")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  src/vector_add.cu:43 warning [unchecked-launch]"))
    );
    assert!(
        !lines.iter().any(|line| line.contains("checked.cu:")),
        "the careful file is clean"
    );
    assert!(!lines.iter().any(|line| line.contains("vendored")));
    Ok(())
}

#[test]
fn profiles_of_the_sample_repositories() -> TestResult {
    let cuda = materialize("cuda-vector-add")?;
    let profile = auto_profile(cuda.path(), "")?;
    let topics: Vec<&str> = profile.topics.iter().map(|topic| topic.as_str()).collect();
    assert_eq!(topics, ["cuda"]);
    let archs: Vec<String> = profile
        .target_archs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(archs, ["sm_75", "sm_90"]);
    let local = [Gpu {
        name: "NVIDIA GeForce GTX 1650".to_owned(),
        capability: ComputeCapability { major: 7, minor: 5 },
    }];
    let lines = profile_lines(NvidiaMode::Auto, &profile, &local);
    assert!(lines.contains(&"Stack: cuda".to_owned()), "{lines:#?}");
    assert!(lines.contains(&"Build targets: sm_75, sm_90".to_owned()));
    assert!(lines.contains(&"Local GPU: NVIDIA GeForce GTX 1650 (sm_75)".to_owned()));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Note: the build targets sm_90"))
    );

    let serving = materialize("trtllm-serving")?;
    let profile = auto_profile(serving.path(), "")?;
    let topics: Vec<&str> = profile.topics.iter().map(|topic| topic.as_str()).collect();
    assert_eq!(topics, ["tensorrt-llm", "triton"]);
    let archs: Vec<String> = profile
        .target_archs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(archs, ["sm_90"]);

    let plain = materialize("plain-rust")?;
    let lines = profile_lines(NvidiaMode::Auto, &auto_profile(plain.path(), "")?, &[]);
    assert!(lines.contains(&"No NVIDIA stack found in this workspace.".to_owned()));
    assert!(lines.contains(&"Local GPU: none reported by nvidia-smi.".to_owned()));
    Ok(())
}

#[test]
fn an_auto_turn_carries_the_profile_only_where_a_stack_is_found() -> TestResult {
    let cuda = materialize("cuda-vector-add")?;
    let context = context_for_turn(NvidiaMode::Auto, cuda.path(), "Why is my kernel slow?")?
        .ok_or("a CUDA workspace gets context")?;
    assert!(context.starts_with("NVIDIA profile of this workspace ($nvidia(auto)):"));
    assert!(context.contains("- stack: cuda\n- build targets: sm_75, sm_90\n"));
    assert!(context.contains("cudaGetLastError"), "{context}");
    assert!(
        context.contains("reports 12 findings in this workspace"),
        "{context}"
    );
    assert!(context.contains("- src/vector_add.cu:7 [missing-grid-bounds]"));
    assert_eq!(
        context.matches("\n- src/vector_add.cu:").count(),
        8,
        "at most eight findings"
    );
    // `list` and `off` keep the profile out of the model's context.
    assert_eq!(context_for_turn(NvidiaMode::List, cuda.path(), "")?, None);
    assert_eq!(context_for_turn(NvidiaMode::Off, cuda.path(), "")?, None);

    let serving = materialize("trtllm-serving")?;
    let context = context_for_turn(NvidiaMode::Auto, serving.path(), "")?
        .ok_or("a TensorRT-LLM workspace gets context")?;
    assert!(context.contains("- stack: tensorrt-llm, triton\n"));
    assert!(context.contains("rebuild them for the target GPU"));
    assert!(
        !context.contains("/nv-verify"),
        "no CUDA sources, no advisor section"
    );

    let plain = materialize("plain-rust")?;
    assert_eq!(context_for_turn(NvidiaMode::Auto, plain.path(), "")?, None);
    // The prompt alone can name the stack.
    let context = context_for_turn(
        NvidiaMode::Auto,
        plain.path(),
        "Port this to CUDA for sm_90",
    )?
    .ok_or("a CUDA prompt gets context")?;
    assert!(context.contains("- stack: cuda\n- build targets: sm_90\n"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_session_directive_selects_the_mode_and_survives_resume() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = materialize("cuda-vector-add")?;
    let id = SessionId::new_v7();
    let mut session = OfflineSession::open(state.path(), workspace.path(), id).await?;
    assert_eq!(
        nvidia_mode(session.kernel()),
        NvidiaMode::Auto,
        "auto without a directive"
    );
    let verify = LocalCommand::NvVerify
        .run(session.kernel(), session.workspace())
        .await?;
    assert_eq!(
        verify.first().map(String::as_str),
        Some("Checked 2 CUDA sources: 10 warnings, 2 suggestions.")
    );

    session.apply_directive("$nvidia(list)").await?;
    assert_eq!(nvidia_mode(session.kernel()), NvidiaMode::List);
    let profile = LocalCommand::NvProfile
        .run(session.kernel(), session.workspace())
        .await?;
    assert!(
        profile
            .first()
            .is_some_and(|line| line.starts_with("$nvidia(list) — shown here only")),
        "{profile:#?}"
    );
    assert!(profile.contains(&"Stack: cuda".to_owned()));

    session.apply_directive("$nvidia(off)").await?;
    assert_eq!(nvidia_mode(session.kernel()), NvidiaMode::Off);
    let off = LocalCommand::NvVerify
        .run(session.kernel(), session.workspace())
        .await?;
    assert!(
        off.first()
            .is_some_and(|line| line.starts_with("$nvidia(off)"))
    );
    session.close().await?;
    drop(session);

    // The kernel rebuilds its directive state from the durable log.
    let mut resumed = OfflineSession::open(state.path(), workspace.path(), id).await?;
    assert_eq!(nvidia_mode(resumed.kernel()), NvidiaMode::Off);
    resumed.apply_directive("$nvidia").await?;
    assert_eq!(nvidia_mode(resumed.kernel()), NvidiaMode::Auto);
    resumed.close().await?;
    Ok(())
}

#[test]
fn only_the_exact_commands_are_answered_locally() {
    assert_eq!(
        LocalCommand::parse("/nv-profile"),
        Some(LocalCommand::NvProfile)
    );
    assert_eq!(
        LocalCommand::parse("  /nv-verify \n"),
        Some(LocalCommand::NvVerify)
    );
    assert_eq!(LocalCommand::parse("/nv-verify src/a.cu"), None);
    assert_eq!(LocalCommand::parse("please run /nv-verify"), None);
    assert_eq!(LocalCommand::parse("/hack"), Some(LocalCommand::Hack));
    assert_eq!(LocalCommand::parse("/hack src"), None);
    assert_eq!(LocalCommand::NvVerify.title(), "CUDA advisor");
}
