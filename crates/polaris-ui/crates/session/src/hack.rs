//! `/hack` (P7-X-2): an adversarial static pass over the workspace. Race candidates, blocking
//! calls in async code, unsafe constructs and always-true assertions come from
//! [`polaris_hacker`]; secrets and destructive shell commands from the automata pack; CUDA
//! barrier misuse and other likely CUDA bugs from the CUDA advisor. Every finding is a located
//! heuristic or observation for a person to confirm, never proof.

use std::path::Path;

use polaris_code_intel::Lang;
use polaris_hacker::{FindingClass, SourceFile, static_scan};
use polaris_nvidia_docs::{Severity, advise_cuda};
use polaris_security::automata::{Category, DfaScanner};

use crate::SessionHostError;
use crate::walk::{line_of, relative, source_files};

/// Sources the hacker accepts per run (its own budget).
const MAX_SOURCES: usize = 128;
/// Largest source the hacker accepts.
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
/// Total source bytes the hacker accepts per run.
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
/// CUDA sources checked per run.
const MAX_CUDA_SOURCES: usize = 500;
/// What `/hack` does not look at.
const COVERAGE: &str = "Not analyzed: races other than split lock scopes and atomic load/store pairs in Rust, deadlocks, quadratic hot loops, unchecked panic paths and path traversal; CUDA is checked by the CUDA advisor's lexical rules. No finding is not a safety verdict.";

/// One located finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HackFinding {
    /// Workspace-relative path with `/` separators.
    pub path: String,
    /// One-based line.
    pub line: usize,
    /// Category: `race`, `blocking-in-async`, `unsafe`, `assertion`, `secret-or-shell`, `cuda`.
    pub category: &'static str,
    /// Stable rule id.
    pub rule: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

/// What `/hack` found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HackReport {
    /// Findings by path and line.
    pub findings: Vec<HackFinding>,
    /// Sources analyzed, by language name.
    pub analyzed: Vec<(&'static str, usize)>,
    /// Sources left out by the size and count budgets.
    pub skipped: usize,
    /// What the analysis does not cover.
    pub notes: Vec<String>,
}

fn language(name: &str) -> Option<Lang> {
    let lowered = name.to_ascii_lowercase();
    let extension = lowered.rsplit_once('.').map(|(_, ext)| ext)?;
    match extension {
        "rs" => Some(Lang::Rust),
        "py" => Some(Lang::Python),
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "hh" => Some(Lang::Cpp),
        "ts" => Some(Lang::TypeScript),
        _ => None,
    }
}

const fn language_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "Rust",
        Lang::Python => "Python",
        Lang::Cpp => "C/C++",
        Lang::Cuda => "CUDA",
        Lang::TypeScript => "TypeScript",
    }
}

fn is_cuda(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".cu") || lowered.ends_with(".cuh")
}

/// Run `/hack` over `workspace` (blocking).
/// # Errors
/// The workspace cannot be listed, or the analyzers fail.
pub fn hack(workspace: &Path) -> Result<HackReport, SessionHostError> {
    let mut report = HackReport::default();
    let (paths, truncated) = source_files(workspace, |name| language(name).is_some(), usize::MAX)?;
    if truncated {
        report
            .notes
            .push("The walk stopped at its bound; some sources were not analyzed.".to_owned());
    }
    let mut files = Vec::new();
    let mut total = 0_u64;
    for path in paths {
        let Some(lang) = path
            .file_name()
            .and_then(|name| language(&name.to_string_lossy()))
        else {
            continue;
        };
        let size = std::fs::metadata(&path).map_or(u64::MAX, |metadata| metadata.len());
        if files.len() >= MAX_SOURCES
            || size > MAX_SOURCE_BYTES
            || total.saturating_add(size) > MAX_TOTAL_BYTES
        {
            report.skipped = report.skipped.saturating_add(1);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        total = total.saturating_add(size);
        files.push(SourceFile {
            path: relative(workspace, &path),
            lang,
            bytes,
        });
    }
    for lang in Lang::ALL {
        let count = files.iter().filter(|file| file.lang == lang).count();
        if count > 0 {
            report.analyzed.push((language_name(lang), count));
        }
    }
    let rules = source_rules()
        .map_err(|_| SessionHostError::Unavailable("pattern rules failed to compile"))?;
    let scanned = static_scan(&files, &rules)
        .map_err(|error| SessionHostError::Ui(format!("static analysis failed: {error}")))?;
    for finding in &scanned.findings {
        let Some(file) = files.iter().find(|file| file.path == finding.path) else {
            continue;
        };
        let (category, message) = describe(finding);
        report.findings.push(HackFinding {
            path: finding.path.clone(),
            line: line_of(&file.bytes, finding.start),
            category,
            rule: finding.rule.clone(),
            message,
        });
    }
    // Per-file notes (syntax errors, partial coverage); the general limits are stated below,
    // because this composition also checks CUDA, which the hacker alone does not.
    report.notes.extend(
        scanned
            .coverage_notes
            .iter()
            .filter(|note| !note.starts_with("not analyzed"))
            .cloned(),
    );

    let (cuda, cuda_truncated) = source_files(workspace, is_cuda, MAX_CUDA_SOURCES)?;
    if cuda_truncated {
        report
            .notes
            .push("Some CUDA sources were not checked (walk bound).".to_owned());
    }
    let mut cuda_count = 0_usize;
    for path in cuda {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        cuda_count = cuda_count.saturating_add(1);
        let source = String::from_utf8_lossy(&bytes);
        for finding in advise_cuda(&source) {
            // Performance suggestions belong to /nv-verify; /hack reports likely bugs.
            if finding.severity == Severity::Warning {
                report.findings.push(HackFinding {
                    path: relative(workspace, &path),
                    line: finding.line,
                    category: "cuda",
                    rule: finding.rule.to_owned(),
                    message: finding.message,
                });
            }
        }
    }
    if cuda_count > 0 {
        report.analyzed.push(("CUDA", cuda_count));
    }
    report.notes.push(COVERAGE.to_owned());
    report.findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.rule.cmp(&b.rule))
    });
    Ok(report)
}

/// Rules that are precise in source code: concrete credential formats and destructive shell
/// commands. The generic `name: value` secret rule (built for redacting logs) and the PII rules
/// match ordinary identifiers such as `token: CancellationToken`, so they are left out.
fn source_rules() -> Result<DfaScanner, polaris_security::automata::AutomataError> {
    let mut builder = DfaScanner::builder();
    for (name, pattern, category) in polaris_security::automata::DEFAULT_RULES {
        let precise = match category {
            Category::Secret => !matches!(*name, "secret_assignment" | "bearer_header"),
            Category::DangerousShell => true,
            Category::Pii => false,
        };
        if precise {
            builder.add(name, pattern, *category)?;
        }
    }
    builder.build(&polaris_security::automata::CompileLimits::default())
}

/// The receiver named in a race finding's evidence (`heuristic:<receiver>:...`).
fn receiver(evidence: &str) -> &str {
    evidence.split(':').nth(1).unwrap_or("the value")
}

fn describe(finding: &polaris_hacker::Finding) -> (&'static str, String) {
    match finding.class {
        FindingClass::RaceCandidate if finding.rule == polaris_hacker::ATOMIC_LOAD_STORE => (
            "race",
            format!(
                "`{}` is read with load and written back with a separate store; concurrent callers lose updates — use fetch_add/fetch_update or compare_exchange",
                receiver(&finding.evidence)
            ),
        ),
        FindingClass::RaceCandidate => (
            "race",
            format!(
                "`{}` is locked again after the earlier guard was released; another thread can change it between the check and the write — hold one guard for both",
                receiver(&finding.evidence)
            ),
        ),
        FindingClass::BlockingInAsync => (
            "blocking-in-async",
            match finding.rule.as_str() {
                "async-blocking-thread-sleep" => "std::thread::sleep parks the executor thread inside an async fn and stalls every task on it; use tokio::time::sleep".to_owned(),
                "async-blocking-fs" => "blocking file I/O inside an async fn stalls the executor thread; use tokio::fs or spawn_blocking".to_owned(),
                _ => "a blocking call inside an async fn stalls the executor thread; use the async equivalent or spawn_blocking".to_owned(),
            },
        ),
        FindingClass::UnsafeConstruct => (
            "unsafe",
            format!(
                "unsafe construct ({}): its safety invariant needs a SAFETY comment and review",
                finding.rule.trim_start_matches("ast-unsafe-")
            ),
        ),
        FindingClass::TautologicalAssertion => (
            "assertion",
            "this assertion is always true and checks nothing".to_owned(),
        ),
        FindingClass::RuleMatch => (
            "secret-or-shell",
            format!(
                "matches the `{}` pattern (a credential or a destructive shell command in source); confirm and remove it",
                finding.rule
            ),
        ),
    }
}

/// `/hack`: a summary, one line per finding, then what was not analyzed.
#[must_use]
pub fn hack_lines(report: &HackReport) -> Vec<String> {
    let analyzed = if report.analyzed.is_empty() {
        "no supported sources".to_owned()
    } else {
        report
            .analyzed
            .iter()
            .map(|(name, count)| format!("{count} {name}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut categories: Vec<(&str, usize)> = Vec::new();
    for finding in &report.findings {
        match categories
            .iter_mut()
            .find(|(name, _)| *name == finding.category)
        {
            Some((_, count)) => *count = count.saturating_add(1),
            None => categories.push((finding.category, 1)),
        }
    }
    categories.sort_by(|a, b| a.0.cmp(b.0));
    let breakdown = if categories.is_empty() {
        String::new()
    } else {
        format!(
            " ({})",
            categories
                .iter()
                .map(|(name, count)| format!("{count} {name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let total = report.findings.len();
    let mut lines = vec![format!(
        "Analyzed {analyzed}: {total} finding{}{breakdown}.",
        if total == 1 { "" } else { "s" }
    )];
    for finding in &report.findings {
        lines.push(format!(
            "{}:{} {} [{}] {}",
            finding.path, finding.line, finding.category, finding.rule, finding.message
        ));
    }
    if report.skipped > 0 {
        lines.push(format!(
            "{} source{} left out by the size and count budgets.",
            report.skipped,
            if report.skipped == 1 {
                " was"
            } else {
                "s were"
            }
        ));
    }
    lines.extend(report.notes.iter().cloned());
    lines.push(
        "Heuristics and pattern matches: advice, not proof — confirm each finding in the code."
            .to_owned(),
    );
    lines
}
