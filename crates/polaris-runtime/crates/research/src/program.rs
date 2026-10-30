//! Strict parser for the immutable `program.md` contract.
//!
//! The supported format has a YAML-like front matter with one key per line, followed by exactly
//! one `metric` fence. It deliberately avoids a general YAML parser: duplicate keys and unknown
//! fields fail closed instead of being interpreted differently by separate consumers.
use crate::{Budget, Direction, MetricParser};
use polaris_sdk::ids::Cid;
use polaris_storage_cas::content_id;
use std::collections::BTreeMap;

const MAX_PROGRAM_BYTES: usize = 65_536;
const MAX_COMMAND_BYTES: usize = 4_096;
const MAX_SCOPES: usize = 128;

/// An immutable, validated research program parsed from the original Markdown bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramMd {
    source: Box<str>,
    cid: Cid,
    goal: Box<str>,
    metric: MetricSpec,
    budget: Budget,
    editable: Vec<Box<str>>,
    readonly: Vec<Box<str>>,
}

impl ProgramMd {
    /// Parse the strict front matter and fenced metric block.
    ///
    /// # Errors
    /// Returns [`ProgramError`] for malformed, ambiguous, oversized, or unsafe definitions.
    pub fn parse(source: &str) -> Result<Self, ProgramError> {
        if source.is_empty() || source.len() > MAX_PROGRAM_BYTES || !source.starts_with("---\n") {
            return Err(ProgramError::Format);
        }
        let mut front = BTreeMap::new();
        let mut metric_fields = BTreeMap::new();
        let mut in_front = true;
        let mut in_metric = false;
        let mut front_closed = false;
        let mut metric_closed = false;
        for line in source.lines().skip(1) {
            if in_front {
                if line == "---" {
                    in_front = false;
                    front_closed = true;
                    continue;
                }
                insert_field(&mut front, line)?;
            } else if line == "```metric" {
                if in_metric || metric_closed {
                    return Err(ProgramError::Format);
                }
                in_metric = true;
            } else if line == "```" && in_metric {
                in_metric = false;
                metric_closed = true;
            } else if in_metric {
                insert_field(&mut metric_fields, line)?;
            }
        }
        if in_front || in_metric || !front_closed || !metric_closed {
            return Err(ProgramError::Format);
        }
        reject_unknown(
            &front,
            &[
                "goal",
                "wall_ms",
                "tokens",
                "micro_usd",
                "max_experiments",
                "editable",
                "readonly",
            ],
        )?;
        reject_unknown(
            &metric_fields,
            &["name", "direction", "format", "command", "harness"],
        )?;
        let goal = take_required(&front, "goal")?;
        if goal.trim().is_empty() || goal.len() > 4_096 || goal.chars().any(char::is_control) {
            return Err(ProgramError::Goal);
        }
        let direction = match required(&metric_fields, "direction")? {
            "min" => Direction::Min,
            "max" => Direction::Max,
            _ => return Err(ProgramError::Metric),
        };
        let name = required(&metric_fields, "name")?;
        let format = required(&metric_fields, "format")?;
        let harness = required(&metric_fields, "harness")?;
        if !valid_path(harness) {
            return Err(ProgramError::ProtectedPath);
        }
        let parser = match format {
            "scalar" => {
                MetricParser::scalar(MAX_PROGRAM_BYTES).map_err(|_| ProgramError::Metric)?
            }
            "named" => {
                MetricParser::named(name, MAX_PROGRAM_BYTES).map_err(|_| ProgramError::Metric)?
            }
            _ => return Err(ProgramError::Metric),
        };
        let command = parse_command(required(&metric_fields, "command")?)?;
        let editable = parse_scopes(required(&front, "editable")?)?;
        let readonly = parse_scopes(required(&front, "readonly")?)?;
        if !scope_covers(&readonly, "program.md") {
            return Err(ProgramError::ProtectedPath);
        }
        for scope in &editable {
            if readonly
                .iter()
                .any(|protected| scopes_overlap(scope, protected))
            {
                return Err(ProgramError::OverlappingScopes);
            }
        }
        if !scope_covers(&readonly, harness) {
            return Err(ProgramError::ProtectedPath);
        }
        let budget = Budget::new(
            parse_number(required(&front, "wall_ms")?)?,
            parse_number(required(&front, "tokens")?)?,
            parse_number(required(&front, "micro_usd")?)?,
            parse_number(required(&front, "max_experiments")?)?,
        )
        .map_err(|_| ProgramError::Budget)?;
        Ok(Self {
            source: source.into(),
            cid: content_id(source.as_bytes()),
            goal: goal.into(),
            metric: MetricSpec {
                name: name.into(),
                direction,
                command,
                harness: harness.into(),
                parser,
            },
            budget,
            editable,
            readonly,
        })
    }

    /// The exact original Markdown bytes as UTF-8.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// BLAKE3 content ID of the exact original Markdown bytes.
    #[must_use]
    pub const fn cid(&self) -> Cid {
        self.cid
    }

    /// Human-authored research goal.
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// Immutable metric definition.
    #[must_use]
    pub const fn metric(&self) -> &MetricSpec {
        &self.metric
    }

    /// Run resource and iteration limits.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// Globs the experiment writer may edit.
    #[must_use]
    pub fn editable(&self) -> &[Box<str>] {
        &self.editable
    }

    /// Globs protected from experiment writers.
    #[must_use]
    pub fn readonly(&self) -> &[Box<str>] {
        &self.readonly
    }

    /// Whether a normalized repository-relative path is within the editable scope.
    #[must_use]
    pub fn allows_edit(&self, path: &str) -> bool {
        valid_path(path)
            && scope_list_covers(&self.editable, path)
            && !scope_list_covers(&self.readonly, path)
    }
}

/// Fixed metric command and its strict output parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricSpec {
    name: Box<str>,
    direction: Direction,
    command: Vec<Box<str>>,
    harness: Box<str>,
    parser: MetricParser,
}

impl MetricSpec {
    /// Stable metric key used in evidence and reports.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Declared optimization direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Argument vector; no shell expansion is performed by the research runner.
    #[must_use]
    pub fn command(&self) -> &[Box<str>] {
        &self.command
    }

    /// Repository path of the executable metric harness, protected from experiment edits.
    #[must_use]
    pub fn harness(&self) -> &str {
        &self.harness
    }

    /// Parse a successful bounded harness output.
    ///
    /// # Errors
    /// Returns the metric parser error when output is ambiguous or invalid.
    pub fn parse_output(
        &self,
        output: &[u8],
    ) -> Result<crate::MetricValue, crate::MetricParseError> {
        self.parser.parse(output)
    }
}

fn insert_field(fields: &mut BTreeMap<String, String>, line: &str) -> Result<(), ProgramError> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(ProgramError::Format);
    };
    let key = key.trim();
    let value = value.trim();
    if key.is_empty()
        || value.is_empty()
        || fields.insert(key.to_owned(), value.to_owned()).is_some()
    {
        return Err(ProgramError::Format);
    }
    Ok(())
}

fn reject_unknown(fields: &BTreeMap<String, String>, allowed: &[&str]) -> Result<(), ProgramError> {
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ProgramError::Format);
    }
    Ok(())
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, ProgramError> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or(ProgramError::Format)
}

fn take_required(fields: &BTreeMap<String, String>, key: &str) -> Result<String, ProgramError> {
    fields.get(key).cloned().ok_or(ProgramError::Format)
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Result<T, ProgramError> {
    value.parse().map_err(|_| ProgramError::Budget)
}

fn parse_command(value: &str) -> Result<Vec<Box<str>>, ProgramError> {
    if value.len() > MAX_COMMAND_BYTES || value.chars().any(char::is_control) {
        return Err(ProgramError::Metric);
    }
    let command: Vec<Box<str>> = value.split_ascii_whitespace().map(Into::into).collect();
    if command.is_empty() {
        return Err(ProgramError::Metric);
    }
    Ok(command)
}

fn parse_scopes(value: &str) -> Result<Vec<Box<str>>, ProgramError> {
    let scopes: Vec<Box<str>> = value.split(',').map(str::trim).map(Into::into).collect();
    if scopes.is_empty()
        || scopes.len() > MAX_SCOPES
        || scopes.iter().any(|scope| !valid_scope(scope))
    {
        return Err(ProgramError::Scope);
    }
    for (index, scope) in scopes.iter().enumerate() {
        if scopes
            .iter()
            .skip(index.saturating_add(1))
            .any(|other| scope == other)
        {
            return Err(ProgramError::Scope);
        }
    }
    Ok(scopes)
}

fn valid_scope(scope: &str) -> bool {
    if scope == "**" {
        return true;
    }
    let path = match scope.strip_suffix("/**") {
        Some(prefix) => prefix,
        None => scope,
    };
    valid_path(path) && scope.len() <= 512 && !path.contains(['*', '?', '[', ']'])
}

fn scope_covers(scopes: &[Box<str>], path: &str) -> bool {
    scope_list_covers(scopes, path)
}

fn scopes_overlap(left: &str, right: &str) -> bool {
    match (recursive_base(left), recursive_base(right)) {
        (Some(left_base), Some(right_base)) => {
            left_base == right_base
                || descendant_of(left_base, right_base)
                || descendant_of(right_base, left_base)
        }
        (Some(base), None) => right == base || descendant_of(right, base),
        (None, Some(base)) => left == base || descendant_of(left, base),
        (None, None) => left == right,
    }
}

fn recursive_base(scope: &str) -> Option<&str> {
    if scope == "**" {
        Some("")
    } else {
        scope.strip_suffix("/**")
    }
}

fn scope_list_covers(scopes: &[Box<str>], path: &str) -> bool {
    scopes.iter().any(|scope| {
        if scope.as_ref() == "**" || scope.as_ref() == path {
            return true;
        }
        recursive_base(scope).is_some_and(|base| !base.is_empty() && descendant_of(path, base))
    })
}

fn descendant_of(path: &str, base: &str) -> bool {
    // The root scope `**` (empty base) contains every path, so `editable: **` overlaps any
    // read-only scope and is rejected instead of silently making `program.md` editable.
    if base.is_empty() {
        return !path.is_empty();
    }
    path.strip_prefix(base)
        .is_some_and(|tail| tail.starts_with('/') && tail.len() > 1)
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '*', '?', '[', ']'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Malformed or unsafe immutable program definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    /// Invalid Markdown layout, duplicate key, or unknown field.
    #[error("invalid program.md structure")]
    Format,
    /// Empty, oversized, or control-containing goal.
    #[error("invalid research goal")]
    Goal,
    /// Invalid metric definition or command.
    #[error("invalid metric definition")]
    Metric,
    /// Zero, malformed, or overflowing run limit.
    #[error("invalid research budget")]
    Budget,
    /// Invalid or duplicate path glob.
    #[error("invalid path scope")]
    Scope,
    /// Editable paths overlap a protected program or metric input.
    #[error("editable and protected path scopes overlap")]
    OverlappingScopes,
    /// Program or metric harness is not protected by readonly scope.
    #[error("program or metric harness path must be protected")]
    ProtectedPath,
}
