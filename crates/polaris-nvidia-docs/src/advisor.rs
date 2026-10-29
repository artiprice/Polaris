//! CUDA advisor rules (`/nv-verify`): lexical checks over CUDA C++ source for the mistakes that
//! cost the most time on a GPU — unchecked launches and API calls, missing grid bounds,
//! barriers in divergent code, strided global access and shared-memory bank conflicts.
//!
//! The rules work on source with comments and literals blanked, so they are fast and need no
//! compiler; they are advice, not proof, and each finding names the line and the fix.

use std::sync::LazyLock;

use regex::Regex;

/// How strongly a finding should be acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Likely a bug.
    Warning,
    /// A performance suggestion.
    Info,
}

/// One rule violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorFinding {
    /// Stable rule id.
    pub rule: &'static str,
    /// Severity.
    pub severity: Severity,
    /// One-based line.
    pub line: usize,
    /// What is wrong and how to fix it.
    pub message: String,
}

/// Largest source examined, in bytes.
pub const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
/// Characters after a launch searched for its error check.
const LAUNCH_CHECK_WINDOW: usize = 400;

fn pattern(source: &str) -> Option<Regex> {
    Regex::new(source).ok()
}

static LAUNCH: LazyLock<Option<Regex>> =
    LazyLock::new(|| pattern(r"\b([A-Za-z_]\w*)\s*<<<[^;]*?>>>\s*\([^;]*\)\s*;"));
static LAUNCH_CHECK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    pattern(r"cudaGetLastError|cudaPeekAtLastError|CUDA_CHECK|CHECK_CUDA|checkCuda|cudaCheck|CUCHK")
});
static API_CALL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    pattern(
        r"(?m)(?:^|[;{}])\s*(cuda(?:Malloc\w*|Memcpy\w*|Memset\w*|Free\w*|DeviceSynchronize|StreamSynchronize|EventSynchronize|MallocManaged))\s*\(",
    )
});
static INDEX_VARIABLE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    pattern(
        r"\b(?:int|unsigned|size_t|long|auto|uint32_t|int64_t)?\s*([A-Za-z_]\w*)\s*=\s*[^;=]*\b(?:threadIdx|blockIdx)\b[^;]*;",
    )
});
static DIVERGENT_IF: LazyLock<Option<Regex>> =
    LazyLock::new(|| pattern(r"\bif\s*\(([^)]*\bthreadIdx\b[^)]*)\)"));
static STRIDED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    pattern(r"\[\s*(threadIdx\.x|tid|idx|i)\s*\*\s*([A-Za-z_]\w*|[2-9]\d*|1\d+)\s*[\]+]")
});
static SHARED_2D: LazyLock<Option<Regex>> = LazyLock::new(|| {
    pattern(r"__shared__\s+[\w:<>\s]+?\b([A-Za-z_]\w*)\s*\[\s*(\d+)\s*\]\s*\[\s*(\d+)\s*\]")
});

/// Check one CUDA source file.
#[must_use]
pub fn advise_cuda(source: &str) -> Vec<AdvisorFinding> {
    if source.len() > MAX_SOURCE_BYTES {
        return Vec::new();
    }
    let code = blank_comments_and_literals(source);
    let mut findings = Vec::new();
    unchecked_launches(&code, &mut findings);
    unchecked_api_calls(&code, &mut findings);
    for kernel in kernels(&code) {
        missing_grid_bounds(&code, &kernel, &mut findings);
        barrier_in_divergent_branch(&code, &kernel, &mut findings);
    }
    strided_access(&code, &mut findings);
    shared_bank_conflicts(&code, &mut findings);
    findings.sort_by(|a, b| a.line.cmp(&b.line).then(a.rule.cmp(b.rule)));
    findings
}

fn unchecked_launches(code: &str, findings: &mut Vec<AdvisorFinding>) {
    let (Some(launch), Some(check)) = (LAUNCH.as_ref(), LAUNCH_CHECK.as_ref()) else {
        return;
    };
    for found in launch.captures_iter(code) {
        let (Some(whole), Some(name)) = (found.get(0), found.get(1)) else {
            continue;
        };
        let window_end = floor_char(code, whole.end().saturating_add(LAUNCH_CHECK_WINDOW));
        let after = code.get(whole.end()..window_end).unwrap_or_default();
        if !check.is_match(after) {
            findings.push(AdvisorFinding {
                rule: "unchecked-launch",
                severity: Severity::Warning,
                line: line_of(code, whole.start()),
                message: format!(
                    "the launch of `{}` is not followed by cudaGetLastError(); launch errors (bad grid size, too many registers) stay silent until a later call fails",
                    name.as_str()
                ),
            });
        }
    }
}

fn unchecked_api_calls(code: &str, findings: &mut Vec<AdvisorFinding>) {
    let Some(api) = API_CALL.as_ref() else { return };
    for found in api.captures_iter(code) {
        let Some(name) = found.get(1) else { continue };
        findings.push(AdvisorFinding {
            rule: "unchecked-api-call",
            severity: Severity::Warning,
            line: line_of(code, name.start()),
            message: format!(
                "the cudaError_t returned by `{}` is ignored; wrap it in an error-checking macro",
                name.as_str()
            ),
        });
    }
}

/// A `__global__` function's name and body range.
struct Kernel {
    name: String,
    body: std::ops::Range<usize>,
}

fn kernels(code: &str) -> Vec<Kernel> {
    let mut found = Vec::new();
    for (at, _) in code.match_indices("__global__") {
        let rest = code.get(at..).unwrap_or_default();
        let Some(open_paren) = rest.find('(') else {
            continue;
        };
        let name = rest
            .get(..open_paren)
            .unwrap_or_default()
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .rfind(|word| !word.is_empty())
            .unwrap_or_default()
            .to_owned();
        let Some(open_brace) = rest.find('{') else {
            continue;
        };
        if rest
            .get(..open_brace)
            .is_some_and(|head| head.contains(';'))
        {
            continue; // a declaration
        }
        let start = at.saturating_add(open_brace);
        let mut depth = 0_usize;
        let mut end = None;
        for (offset, ch) in code.get(start..).unwrap_or_default().char_indices() {
            match ch {
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(start.saturating_add(offset).saturating_add(1));
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            found.push(Kernel {
                name,
                body: start..end,
            });
        }
    }
    found
}

fn missing_grid_bounds(code: &str, kernel: &Kernel, findings: &mut Vec<AdvisorFinding>) {
    let Some(index) = INDEX_VARIABLE.as_ref() else {
        return;
    };
    let body = code.get(kernel.body.clone()).unwrap_or_default();
    for found in index.captures_iter(body) {
        let Some(variable) = found.get(1) else {
            continue;
        };
        let name = regex::escape(variable.as_str());
        let used_as_index =
            pattern(&format!(r"\[\s*{name}\s*\]")).is_some_and(|re| re.is_match(body));
        let compared = pattern(&format!(r"\b{name}\s*(<|<=|>|>=)|(<|<=|>|>=)\s*{name}\b"))
            .is_some_and(|re| re.is_match(body));
        if used_as_index && !compared {
            findings.push(AdvisorFinding {
                rule: "missing-grid-bounds",
                severity: Severity::Warning,
                line: line_of(code, kernel.body.start.saturating_add(variable.start())),
                message: format!(
                    "kernel `{}` indexes with `{}` without comparing it to the data size; the last block reads and writes past the end unless the grid divides the size exactly (add `if ({} >= n) return;` or a grid-stride loop)",
                    kernel.name,
                    variable.as_str(),
                    variable.as_str()
                ),
            });
        }
    }
}

fn barrier_in_divergent_branch(code: &str, kernel: &Kernel, findings: &mut Vec<AdvisorFinding>) {
    let Some(divergent) = DIVERGENT_IF.as_ref() else {
        return;
    };
    let body = code.get(kernel.body.clone()).unwrap_or_default();
    for found in divergent.captures_iter(body) {
        let Some(whole) = found.get(0) else { continue };
        let after = body.get(whole.end()..).unwrap_or_default();
        let branch = match after.trim_start().strip_prefix('{') {
            Some(inner) => {
                let mut depth = 1_usize;
                let mut end = inner.len();
                for (offset, ch) in inner.char_indices() {
                    match ch {
                        '{' => depth = depth.saturating_add(1),
                        '}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                end = offset;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                inner.get(..end).unwrap_or_default()
            }
            None => after.split(';').next().unwrap_or_default(),
        };
        if branch.contains("__syncthreads") {
            findings.push(AdvisorFinding {
                rule: "barrier-in-divergent-branch",
                severity: Severity::Warning,
                line: line_of(code, kernel.body.start.saturating_add(whole.start())),
                message: format!(
                    "kernel `{}` calls __syncthreads() inside a branch that depends on threadIdx; threads that skip the branch never reach the barrier and the block can hang",
                    kernel.name
                ),
            });
        }
    }
}

fn strided_access(code: &str, findings: &mut Vec<AdvisorFinding>) {
    let Some(strided) = STRIDED.as_ref() else {
        return;
    };
    for found in strided.captures_iter(code) {
        let (Some(whole), Some(index), Some(stride)) = (found.get(0), found.get(1), found.get(2))
        else {
            continue;
        };
        findings.push(AdvisorFinding {
            rule: "strided-global-access",
            severity: Severity::Info,
            line: line_of(code, whole.start()),
            message: format!(
                "`[{} * {}]` makes neighbouring threads touch addresses {} elements apart, so a warp's loads are not coalesced; index by thread first (`[row * width + threadIdx.x]`) or stage the tile through shared memory",
                index.as_str(),
                stride.as_str(),
                stride.as_str()
            ),
        });
    }
}

fn shared_bank_conflicts(code: &str, findings: &mut Vec<AdvisorFinding>) {
    let Some(shared) = SHARED_2D.as_ref() else {
        return;
    };
    for found in shared.captures_iter(code) {
        let (Some(whole), Some(name), Some(rows), Some(columns)) =
            (found.get(0), found.get(1), found.get(2), found.get(3))
        else {
            continue;
        };
        let Ok(width) = columns.as_str().parse::<u32>() else {
            continue;
        };
        if width == 0 || width % 32 != 0 {
            continue;
        }
        findings.push(AdvisorFinding {
            rule: "shared-bank-conflict",
            severity: Severity::Info,
            line: line_of(code, whole.start()),
            message: format!(
                "`__shared__ {}[{}][{width}]`: a row width that is a multiple of 32 puts a column in one bank, so column-wise access (transposes) serialises 32 ways; pad it to [{}][{}]",
                name.as_str(),
                rows.as_str(),
                rows.as_str(),
                width.saturating_add(1)
            ),
        });
    }
}

/// Comments and string/char literals replaced by spaces; newlines and byte offsets kept.
fn blank_comments_and_literals(source: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        Line,
        Block,
        Text(char),
    }
    let mut out = String::with_capacity(source.len());
    let mut state = State::Code;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        let blank = |ch: char| if ch == '\n' { '\n' } else { ' ' };
        match state {
            State::Code => match (ch, chars.peek().copied()) {
                ('/', Some('/')) => {
                    state = State::Line;
                    out.push(' ');
                }
                ('/', Some('*')) => {
                    state = State::Block;
                    out.push(' ');
                }
                ('"' | '\'', _) => {
                    state = State::Text(ch);
                    out.push(ch);
                }
                _ => out.push(ch),
            },
            State::Line => {
                if ch == '\n' {
                    state = State::Code;
                }
                out.push(blank(ch));
            }
            State::Block => {
                if ch == '*' && chars.peek() == Some(&'/') {
                    out.push(' ');
                    if let Some(slash) = chars.next() {
                        out.push(blank(slash));
                    }
                    state = State::Code;
                } else {
                    out.push(blank(ch));
                }
            }
            State::Text(quote) => {
                if ch == '\\' {
                    out.push(' ');
                    if let Some(escaped) = chars.next() {
                        out.push(blank(escaped));
                    }
                } else if ch == quote {
                    state = State::Code;
                    out.push(ch);
                } else {
                    out.push(blank(ch));
                }
            }
        }
    }
    out
}

fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(0, |prefix| prefix.matches('\n').count())
        .saturating_add(1)
}

fn floor_char(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}
