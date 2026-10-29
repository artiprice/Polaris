//! NVIDIA knowledge at the composition root (P5, PLAN §6.14): the session's `$nvidia(...)`
//! directive decides whether a chat turn carries the workspace's NVIDIA profile and the CUDA
//! invariants that apply to it; `/nv-profile` and `/nv-verify` report the profile and the CUDA
//! advisor findings ([`polaris_nvidia_docs`]) to every frontend.
//!
//! `$nvidia` / `$nvidia(auto)` (also the mode when no `$nvidia` directive was applied) profiles
//! the workspace and the prompt each turn and adds context only when an NVIDIA stack is found;
//! `$nvidia(list)` keeps the profile out of the model's context (the commands still show it);
//! `$nvidia(off)` disables both.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use polaris_kernel::KernelHandle;
use polaris_kernel::directives::directive::{NvidiaMode, SessionDirective};
use polaris_nvidia_docs::advisor::MAX_SOURCE_BYTES;
use polaris_nvidia_docs::{
    AdvisorFinding, DocTopic, Gpu, NvidiaProfile, Severity, advise_cuda, auto_profile,
    parse_nvidia_smi,
};

use crate::SessionHostError;
use crate::walk::{relative, source_files};

/// CUDA sources `/nv-verify` checks.
const MAX_SOURCES: usize = 500;
/// Findings quoted in a turn's context.
const CONTEXT_FINDINGS: usize = 8;
/// How long `nvidia-smi` may take.
const SMI_TIMEOUT: Duration = Duration::from_secs(3);

/// The `$nvidia` mode in effect: the session's directive, [`NvidiaMode::Auto`] when none was
/// applied.
#[must_use]
pub fn nvidia_mode(kernel: &KernelHandle) -> NvidiaMode {
    match kernel.directives().get("nvidia") {
        Some(SessionDirective::Nvidia(mode)) => *mode,
        _ => NvidiaMode::Auto,
    }
}

/// The CUDA advisor findings of one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CudaFindings {
    /// Workspace-relative path with `/` separators.
    pub path: String,
    /// Findings, in line order.
    pub findings: Vec<AdvisorFinding>,
}

/// What `/nv-verify` found in a workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvVerifyReport {
    /// CUDA sources checked.
    pub checked: usize,
    /// Sources with at least one finding, by path.
    pub files: Vec<CudaFindings>,
    /// Sources skipped because they exceed the advisor's size bound.
    pub oversized: Vec<String>,
    /// The walk stopped at a bound before seeing the whole workspace.
    pub truncated: bool,
}

impl NvVerifyReport {
    /// Findings that are likely bugs.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.findings()
            .filter(|(_, finding)| finding.severity == Severity::Warning)
            .count()
    }

    /// Every finding with its path.
    pub fn findings(&self) -> impl Iterator<Item = (&str, &AdvisorFinding)> {
        self.files.iter().flat_map(|file| {
            file.findings
                .iter()
                .map(move |finding| (file.path.as_str(), finding))
        })
    }
}

/// Run the CUDA advisor over every `.cu`/`.cuh` source of `workspace` (bounded walk; vendored
/// and build directories are skipped).
/// # Errors
/// The workspace cannot be listed.
pub fn nv_verify(workspace: &Path) -> Result<NvVerifyReport, SessionHostError> {
    let (sources, truncated) = source_files(workspace, is_cuda_source, MAX_SOURCES)?;
    let mut report = NvVerifyReport {
        truncated,
        ..NvVerifyReport::default()
    };
    for path in sources {
        let relative = relative(workspace, &path);
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if usize::try_from(metadata.len()).map_or(true, |len| len > MAX_SOURCE_BYTES) {
            report.oversized.push(relative);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        report.checked = report.checked.saturating_add(1);
        let findings = advise_cuda(&String::from_utf8_lossy(&bytes));
        if !findings.is_empty() {
            report.files.push(CudaFindings {
                path: relative,
                findings,
            });
        }
    }
    Ok(report)
}

fn is_cuda_source(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".cu") || lowered.ends_with(".cuh")
}

/// The local GPUs `nvidia-smi` reports; empty when it is missing, fails or takes too long.
pub async fn local_gpus() -> Vec<Gpu> {
    let mut command = tokio::process::Command::new("nvidia-smi");
    command
        .args(["--query-gpu=name,compute_cap", "--format=csv,noheader"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(SMI_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

fn mode_name(mode: NvidiaMode) -> &'static str {
    match mode {
        NvidiaMode::Auto => "auto",
        NvidiaMode::List => "list",
        NvidiaMode::Off => "off",
    }
}

fn topic_names(profile: &NvidiaProfile) -> String {
    profile
        .topics
        .iter()
        .map(|topic| topic.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn arch_names(profile: &NvidiaProfile) -> String {
    profile
        .target_archs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `/nv-profile`: the mode, the detected stack with its evidence, the local GPUs and what the
/// build targets that they cannot run.
#[must_use]
pub fn profile_lines(mode: NvidiaMode, profile: &NvidiaProfile, local: &[Gpu]) -> Vec<String> {
    let mut lines = vec![format!(
        "$nvidia({}) — {}",
        mode_name(mode),
        match mode {
            NvidiaMode::Auto => "replies get this profile and its invariants when a stack is found",
            NvidiaMode::List => "shown here only; replies do not get it",
            NvidiaMode::Off => "disabled; replies do not get it",
        }
    )];
    if profile.topics.is_empty() && profile.target_archs.is_empty() {
        lines.push("No NVIDIA stack found in this workspace.".to_owned());
    } else {
        lines.push(format!("Stack: {}", or_none(&topic_names(profile))));
        lines.push(format!("Build targets: {}", or_none(&arch_names(profile))));
        lines.push("Evidence:".to_owned());
        lines.extend(profile.evidence.iter().map(|line| format!("  {line}")));
    }
    if local.is_empty() {
        lines.push("Local GPU: none reported by nvidia-smi.".to_owned());
    } else {
        for gpu in local {
            lines.push(format!("Local GPU: {} ({})", gpu.name, gpu.capability));
        }
    }
    lines.extend(
        profile
            .hardware_notes(local)
            .into_iter()
            .map(|note| format!("Note: {note}")),
    );
    lines
}

fn or_none(text: &str) -> &str {
    if text.is_empty() { "none" } else { text }
}

/// `/nv-verify`: one line per finding, grouped by file, with a summary first.
#[must_use]
pub fn verify_lines(report: &NvVerifyReport) -> Vec<String> {
    let total = report.findings().count();
    let mut lines = vec![format!(
        "Checked {} CUDA source{}: {} warning{}, {} suggestion{}.",
        report.checked,
        plural(report.checked),
        report.warnings(),
        plural(report.warnings()),
        total.saturating_sub(report.warnings()),
        plural(total.saturating_sub(report.warnings())),
    )];
    if report.checked == 0 && report.oversized.is_empty() {
        lines.push("No .cu or .cuh sources in this workspace.".to_owned());
    }
    for file in &report.files {
        lines.push(file.path.clone());
        for finding in &file.findings {
            lines.push(format!(
                "  {}:{} {} [{}] {}",
                file.path,
                finding.line,
                severity_name(finding.severity),
                finding.rule,
                finding.message
            ));
        }
    }
    for path in &report.oversized {
        lines.push(format!("{path}: skipped (larger than 2 MiB)"));
    }
    if report.truncated {
        lines.push("The walk stopped at its bound; some sources were not checked.".to_owned());
    }
    lines.push("Lexical rules: advice, not proof — confirm each finding in the code.".to_owned());
    lines
}

const fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// Short invariants for code on `topic`, cited by rule rather than by document.
#[must_use]
pub const fn invariants(topic: DocTopic) -> &'static [&'static str] {
    match topic {
        DocTopic::Cuda => &[
            "check every kernel launch (cudaGetLastError right after it) and every CUDA runtime call's return code",
            "guard each global index against the problem size; use a grid-stride loop when the grid may not cover it",
            "never call __syncthreads() inside a branch that depends on threadIdx",
            "keep global accesses coalesced (consecutive threads touch consecutive addresses); pad 2-D shared arrays by one column against bank conflicts",
        ],
        DocTopic::Ptx => &[
            "inline PTX must match the target architecture; guard newer instructions with __CUDA_ARCH__ checks",
        ],
        DocTopic::TensorRtLlm => &[
            "engines are built for one GPU architecture and TensorRT-LLM version; rebuild them for the target GPU",
        ],
        DocTopic::Triton => &[
            "keep the model repository layout and config.pbtxt in sync with the model's inputs, outputs and max_batch_size",
        ],
        DocTopic::Nccl => &[
            "every rank must issue the same collectives in the same order; check ncclResult_t of each call",
        ],
        DocTopic::OpenShell | DocTopic::NemoClaw | DocTopic::Nemotron => &[],
    }
}

/// Context for one chat turn under `$nvidia(auto)`: the profile, its invariants and the advisor's
/// top findings; `None` when the workspace and prompt show no NVIDIA stack.
#[must_use]
pub fn turn_context(profile: &NvidiaProfile, report: &NvVerifyReport) -> Option<String> {
    if profile.topics.is_empty() && profile.target_archs.is_empty() {
        return None;
    }
    let mut text = String::from("NVIDIA profile of this workspace ($nvidia(auto)):\n");
    let _stack = writeln!(text, "- stack: {}", or_none(&topic_names(profile)));
    let _archs = writeln!(text, "- build targets: {}", or_none(&arch_names(profile)));
    let invariants: Vec<&str> = profile
        .topics
        .iter()
        .flat_map(|topic| invariants(*topic).iter().copied())
        .collect();
    if !invariants.is_empty() {
        text.push_str("Respect these invariants in answers and changes:\n");
        for invariant in invariants {
            let _line = writeln!(text, "- {invariant}");
        }
    }
    let total = report.findings().count();
    if total > 0 {
        let _summary = writeln!(
            text,
            "The CUDA advisor (/nv-verify) reports {total} finding{} in this workspace; the first ones:",
            plural(total)
        );
        for (path, finding) in report.findings().take(CONTEXT_FINDINGS) {
            let _line = writeln!(
                text,
                "- {path}:{} [{}] {}",
                finding.line, finding.rule, finding.message
            );
        }
    }
    text.push_str(
        "These are lexical heuristics, not proof: confirm them in the code before relying on them.",
    );
    Some(text)
}

/// The turn context for `prompt` when `mode` is [`NvidiaMode::Auto`] (blocking: walks the
/// workspace).
/// # Errors
/// The workspace cannot be listed.
pub fn context_for_turn(
    mode: NvidiaMode,
    workspace: &Path,
    prompt: &str,
) -> Result<Option<String>, SessionHostError> {
    if mode != NvidiaMode::Auto {
        return Ok(None);
    }
    let profile = auto_profile(workspace, prompt)
        .map_err(|_| SessionHostError::Unavailable("workspace is unavailable"))?;
    if profile.topics.is_empty() && profile.target_archs.is_empty() {
        return Ok(None);
    }
    let report = if profile.topics.contains(&DocTopic::Cuda) {
        nv_verify(workspace)?
    } else {
        NvVerifyReport::default()
    };
    Ok(turn_context(&profile, &report))
}

/// `/nv-profile` (`verify` false) or `/nv-verify` (`verify` true) on `workspace` under `mode`.
/// # Errors
/// The workspace cannot be listed.
pub(crate) async fn report_lines(
    verify: bool,
    mode: NvidiaMode,
    workspace: &Path,
) -> Result<Vec<String>, SessionHostError> {
    if mode == NvidiaMode::Off {
        return Ok(vec![
            if verify {
                "$nvidia(off) — the CUDA advisor is disabled; apply $nvidia(auto) to enable it."
            } else {
                "$nvidia(off) — NVIDIA profiling is disabled; apply $nvidia(auto) to enable it."
            }
            .to_owned(),
        ]);
    }
    let root = workspace.to_owned();
    if verify {
        let report = tokio::task::spawn_blocking(move || nv_verify(&root))
            .await
            .map_err(|_| SessionHostError::Unavailable("advisor task failed"))??;
        Ok(verify_lines(&report))
    } else {
        let profile = tokio::task::spawn_blocking(move || auto_profile(&root, ""))
            .await
            .map_err(|_| SessionHostError::Unavailable("profile task failed"))?
            .map_err(|_| SessionHostError::Unavailable("workspace is unavailable"))?;
        Ok(profile_lines(mode, &profile, &local_gpus().await))
    }
}
