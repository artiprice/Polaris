use crate::{Confidence, Finding, FindingClass, HackerError, SourceFile, StaticReport};
use polaris_code_intel::{AnalysisCoverage, ParserPool, analyze_safety};
use polaris_security_automata::{DfaScanner, ScanBuf};

const MAX_FILES: usize = 128;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MATCHES_PER_FILE: usize = 4096;
const MAX_FINDINGS: usize = 8192;
const MAX_PATH_BYTES: usize = 4096;
const MAX_RULE_BYTES: usize = 256;
const COVERAGE_LIMITS: &str = "not analyzed: quadratic hot loops, thread races, CUDA barrier/bank behavior, unchecked panic paths, or path traversal; absence of findings is not a safety verdict";

/// Run code-intel observations and the caller-selected DFA rules over caller-owned source bytes.
///
/// The async blocking rule is intentionally advisory: code-intel provides source-located AST calls,
/// while this crate conservatively approximates the enclosing async region from source keywords.
/// Other syntax findings come directly from the code-intel analyzer. Evidence contains only rule
/// and node-kind metadata; source bytes remain caller-owned and are never copied into reports.
///
/// # Errors
/// Returns parser/automata errors or an explicit request-budget error. No partial report is
/// returned when file, byte, DFA-match, or finding budgets are exceeded.
pub fn static_scan(files: &[SourceFile], rules: &DfaScanner) -> Result<StaticReport, HackerError> {
    validate_budgets(files)?;
    let parser = ParserPool::new();
    let mut report = StaticReport::default();
    let mut matches = ScanBuf::with_capacity(MAX_MATCHES_PER_FILE);
    report.coverage_notes.push(COVERAGE_LIMITS.to_owned());

    for file in files {
        let parsed = parser.parse(file.lang, &file.bytes, None)?;
        let analysis = analyze_safety(&parsed)?;
        if analysis.syntax_error {
            report.coverage_notes.push(format!(
                "{}: syntax errors present; findings may be incomplete",
                file.path
            ));
        }
        if analysis.unsafe_coverage != AnalysisCoverage::Complete {
            report.coverage_notes.push(format!(
                "{}: unsafe-construct coverage is {:?}",
                file.path, analysis.unsafe_coverage
            ));
        }
        if analysis.call_coverage != AnalysisCoverage::Complete {
            report.coverage_notes.push(format!(
                "{}: call-pattern coverage is {:?}",
                file.path, analysis.call_coverage
            ));
        }
        for observation in &analysis.unsafe_constructs {
            if observation.span.text(&file.bytes).is_some() {
                push_finding(
                    &mut report,
                    Finding {
                        path: file.path.clone(),
                        class: FindingClass::UnsafeConstruct,
                        rule: format!("ast-unsafe-{}", observation.node_kind),
                        start: observation.span.start,
                        end: observation.span.end,
                        evidence: format!("ast-unsafe:{}", observation.node_kind),
                        confidence: Confidence::DeterministicObservation,
                    },
                )?;
            }
        }
        for observation in &analysis.tautologies {
            if observation.span.text(&file.bytes).is_some() {
                push_finding(
                    &mut report,
                    Finding {
                        path: file.path.clone(),
                        class: FindingClass::TautologicalAssertion,
                        rule: format!("ast-tautology-{:?}", observation.kind),
                        start: observation.span.start,
                        end: observation.span.end,
                        evidence: format!("ast-tautology:{:?}", observation.kind),
                        confidence: Confidence::DeterministicObservation,
                    },
                )?;
            }
        }
        for call in &analysis.calls {
            if call.identity.as_deref() == Some("std::thread::sleep")
                && in_async_region(&file.bytes, call.span.start)
                && call.span.text(&file.bytes).is_some()
            {
                push_finding(
                    &mut report,
                    Finding {
                        path: file.path.clone(),
                        class: FindingClass::BlockingInAsync,
                        rule: "async-blocking-thread-sleep".to_owned(),
                        start: call.span.start,
                        end: call.span.end,
                        evidence: "heuristic:async-region+std::thread::sleep".to_owned(),
                        confidence: Confidence::AdvisoryHeuristic,
                    },
                )?;
            }
        }
        rules.scan(&file.bytes, &mut matches)?;
        for matched in matches.matches() {
            let start = usize::try_from(matched.start)
                .map_err(|_| HackerError::InvalidRequest("DFA span does not fit platform size"))?;
            let end = usize::try_from(matched.end)
                .map_err(|_| HackerError::InvalidRequest("DFA span does not fit platform size"))?;
            if end <= file.bytes.len()
                && start < end
                && let Some(rule) = rules.rule_name(matched.rule)
            {
                if rule.len() > MAX_RULE_BYTES {
                    return Err(HackerError::InvalidRequest(
                        "rule label exceeds audit limit",
                    ));
                }
                push_finding(
                    &mut report,
                    Finding {
                        path: file.path.clone(),
                        class: FindingClass::RuleMatch,
                        rule: rule.to_owned(),
                        start,
                        end,
                        evidence: format!("dfa-rule:{rule}"),
                        confidence: Confidence::DeterministicObservation,
                    },
                )?;
            }
        }
    }
    report.findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.start.cmp(&right.start))
            .then(left.rule.cmp(&right.rule))
    });
    Ok(report)
}

fn validate_budgets(files: &[SourceFile]) -> Result<(), HackerError> {
    if files.len() > MAX_FILES {
        return Err(HackerError::InvalidRequest(
            "source file count exceeds audit limit",
        ));
    }
    let mut total_bytes = 0_usize;
    for file in files {
        if file.path.is_empty() || file.path.len() > MAX_PATH_BYTES {
            return Err(HackerError::InvalidRequest(
                "source path exceeds audit limit",
            ));
        }
        if file.bytes.len() > MAX_FILE_BYTES {
            return Err(HackerError::InvalidRequest(
                "source file exceeds audit limit",
            ));
        }
        total_bytes =
            total_bytes
                .checked_add(file.bytes.len())
                .ok_or(HackerError::InvalidRequest(
                    "aggregate source size overflow",
                ))?;
        if total_bytes > MAX_TOTAL_SOURCE_BYTES {
            return Err(HackerError::InvalidRequest(
                "aggregate source size exceeds audit limit",
            ));
        }
    }
    Ok(())
}

fn push_finding(report: &mut StaticReport, finding: Finding) -> Result<(), HackerError> {
    if report.findings.len() >= MAX_FINDINGS {
        return Err(HackerError::InvalidRequest(
            "finding count exceeds audit limit",
        ));
    }
    report.findings.push(finding);
    Ok(())
}

fn in_async_region(source: &[u8], call_start: usize) -> bool {
    let Some(prefix) = source.get(..call_start) else {
        return false;
    };
    let Ok(prefix) = std::str::from_utf8(prefix) else {
        return false;
    };
    let Some(function_position) = prefix.rfind("fn ") else {
        return false;
    };
    let before_function = match prefix.get(..function_position) {
        Some(value) => value,
        None => return false,
    };
    if !before_function.ends_with("async ") {
        return false;
    }
    let async_position = function_position.saturating_sub("async ".len());
    prefix
        .get(async_position..)
        .is_some_and(|region| region.matches('{').count() > region.matches('}').count())
}
