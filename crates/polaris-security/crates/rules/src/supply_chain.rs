//! Bounded manifest-name review against a caller-owned trusted package baseline.

use crate::error::RulesError;
use crate::model::{Cwe, Severity};
use crate::pack::RuleEngine;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const MAX_MANIFEST_BYTES: usize = 524_288;
const MAX_MANIFEST_PATH_UNITS: usize = 4096;
const MAX_MANIFEST_PACKAGES: usize = 4096;
const MAX_BASELINE_PACKAGES: usize = 20_000;
const MAX_BASELINE_NAME_BYTES: usize = 128;
const MAX_BASELINE_BYTES: usize = 2_097_152;
const MAX_PACKAGE_COMPARISONS: usize = 250_000;
const MAX_TIED_CANDIDATES: usize = 8;
const MAX_EDIT_DISTANCE: u8 = 2;

/// Package ecosystem represented by a supported manifest format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PackageEcosystem {
    /// Cargo.toml dependency sections.
    Cargo,
    /// package.json dependency sections.
    Npm,
    /// Python pyproject.toml or requirements.txt dependencies.
    Python,
}

/// Explicit result state for one manifest audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestAuditStatus {
    /// Supported manifest parsed and compared against the supplied baseline.
    Analyzed,
    /// No trusted baseline was supplied; no dependency parsing or comparisons were performed.
    BaselineUnavailable(PackageEcosystem),
    /// The path is not one of the supported manifest names.
    UnsupportedManifest,
}

/// Caller-supplied trusted package names; no network or embedded registry data is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPackageBaseline {
    ecosystem: PackageEcosystem,
    names: BTreeMap<String, String>,
    names_by_length: BTreeMap<usize, Vec<(String, String)>>,
}

impl TrustedPackageBaseline {
    /// Validates and indexes known names supplied by the caller.
    ///
    /// # Errors
    /// Returns a bounds error for too many names, oversized aggregate input, or invalid names.
    pub fn new(
        ecosystem: PackageEcosystem,
        names: impl IntoIterator<Item = String>,
    ) -> Result<Self, RulesError> {
        let mut normalized = BTreeMap::new();
        let mut total_bytes = 0_usize;
        let mut count = 0_usize;
        for name in names {
            count = count
                .checked_add(1)
                .ok_or_else(|| RulesError::Bound(String::from("trusted package count overflow")))?;
            if count > MAX_BASELINE_PACKAGES {
                return Err(RulesError::Bound(String::from(
                    "trusted package count exceeds its bound",
                )));
            }
            total_bytes = total_bytes.checked_add(name.len()).ok_or_else(|| {
                RulesError::Bound(String::from("trusted package input length overflow"))
            })?;
            if total_bytes > MAX_BASELINE_BYTES {
                return Err(RulesError::Bound(String::from(
                    "trusted package baseline exceeds its byte bound",
                )));
            }
            validate_package_name(ecosystem, &name)?;
            let canonical = normalize_package_name(ecosystem, &name);
            match normalized.get_mut(&canonical) {
                Some(existing) if name < *existing => *existing = name,
                Some(_) => {}
                None => {
                    normalized.insert(canonical, name);
                }
            }
        }
        let mut names_by_length = BTreeMap::<usize, Vec<(String, String)>>::new();
        for (canonical, display_name) in &normalized {
            names_by_length
                .entry(canonical.len())
                .or_default()
                .push((canonical.clone(), display_name.clone()));
        }
        Ok(Self {
            ecosystem,
            names: normalized,
            names_by_length,
        })
    }

    /// Ecosystem covered by this baseline.
    #[must_use]
    pub const fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }

    /// Count of unique normalized names in the baseline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether the baseline contains no names.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// One informational near-name review against the caller's known-package set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyposquatFinding {
    ecosystem: PackageEcosystem,
    manifest_path: PathBuf,
    package_name: String,
    nearest_known_names: Vec<String>,
    distance: u8,
    ambiguous: bool,
}

impl TyposquatFinding {
    /// The parsed package name as declared by the manifest.
    #[must_use]
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    /// Stable-sorted closest names from the exact caller-supplied baseline.
    #[must_use]
    pub fn nearest_known_names(&self) -> &[String] {
        &self.nearest_known_names
    }

    /// Bounded edit distance between normalized names, including adjacent transpositions.
    #[must_use]
    pub const fn distance(&self) -> u8 {
        self.distance
    }

    /// True when more than one distinct known package shares the minimum distance.
    #[must_use]
    pub const fn ambiguous(&self) -> bool {
        self.ambiguous
    }

    /// Supply-chain typo heuristics have no assigned CWE.
    #[must_use]
    pub const fn cwe(&self) -> Option<Cwe> {
        None
    }

    /// Informational because name similarity alone does not establish malicious intent.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        Severity::Info
    }

    /// Rule identifier for this finite heuristic.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        "manifest-package-near-name"
    }

    /// Manifest path associated with the review result.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Ecosystem used to normalize the package name.
    #[must_use]
    pub const fn ecosystem(&self) -> PackageEcosystem {
        self.ecosystem
    }
}

/// Parsed manifest coverage and its deterministic name-review findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestAuditReport {
    status: ManifestAuditStatus,
    findings: Vec<TyposquatFinding>,
    package_count: usize,
}

impl ManifestAuditReport {
    /// Coverage or explicit absence state.
    #[must_use]
    pub const fn status(&self) -> ManifestAuditStatus {
        self.status
    }

    /// Near-name review findings. No source text or checksum is returned.
    #[must_use]
    pub fn findings(&self) -> &[TyposquatFinding] {
        &self.findings
    }

    /// Number of unique dependency package names parsed.
    #[must_use]
    pub const fn package_count(&self) -> usize {
        self.package_count
    }
}

impl RuleEngine {
    /// Compares package names in a supported manifest against the supplied trusted baseline.
    ///
    /// The heuristic is informational and deterministic. It never fetches or invents registry
    /// names, reports no CWE, excludes exact normalized names, and marks equal-distance ties.
    ///
    /// # Errors
    /// Returns a typed bound, manifest parse, or baseline/ecosystem error. A missing baseline and
    /// an unsupported manifest are explicit report states rather than guessed findings.
    pub fn audit_manifest(
        &self,
        path: &Path,
        manifest_bytes: &[u8],
        baseline: Option<&TrustedPackageBaseline>,
    ) -> Result<ManifestAuditReport, RulesError> {
        if manifest_bytes.len() > MAX_MANIFEST_BYTES {
            return Err(RulesError::Bound(String::from(
                "manifest exceeds its byte bound",
            )));
        }
        if !safe_relative_path(path) {
            return Err(RulesError::Bound(String::from(
                "manifest path must be relative and normalized",
            )));
        }
        let Some(ecosystem) = ecosystem_for_path(path) else {
            return Ok(ManifestAuditReport {
                status: ManifestAuditStatus::UnsupportedManifest,
                findings: Vec::new(),
                package_count: 0,
            });
        };
        let Some(baseline) = baseline else {
            return Ok(ManifestAuditReport {
                status: ManifestAuditStatus::BaselineUnavailable(ecosystem),
                findings: Vec::new(),
                package_count: 0,
            });
        };
        if baseline.ecosystem != ecosystem {
            return Err(RulesError::Bound(String::from(
                "manifest and trusted package baseline ecosystems differ",
            )));
        }
        let manifest = std::str::from_utf8(manifest_bytes)
            .map_err(|_| RulesError::Manifest(String::from("manifest is not UTF-8")))?;
        let package_names = parse_manifest_names(ecosystem, path, manifest)?;
        let package_count = package_names.len();
        let findings = compare_package_names(ecosystem, path, package_names, baseline)?;
        Ok(ManifestAuditReport {
            status: ManifestAuditStatus::Analyzed,
            findings,
            package_count,
        })
    }
}

fn ecosystem_for_path(path: &Path) -> Option<PackageEcosystem> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    match name.as_str() {
        "cargo.toml" => Some(PackageEcosystem::Cargo),
        "package.json" => Some(PackageEcosystem::Npm),
        "pyproject.toml" | "requirements.txt" => Some(PackageEcosystem::Python),
        _ => None,
    }
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.as_os_str().len() <= MAX_MANIFEST_PATH_UNITS
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn parse_manifest_names(
    ecosystem: PackageEcosystem,
    path: &Path,
    input: &str,
) -> Result<BTreeMap<String, String>, RulesError> {
    match ecosystem {
        PackageEcosystem::Cargo => parse_cargo_names(input),
        PackageEcosystem::Npm => parse_npm_names(input),
        PackageEcosystem::Python
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("requirements.txt")) =>
        {
            parse_requirements_names(input)
        }
        PackageEcosystem::Python => parse_python_toml_names(input),
    }
}

fn parse_cargo_names(input: &str) -> Result<BTreeMap<String, String>, RulesError> {
    let document = toml::from_str::<toml::Value>(input)
        .map_err(|_| RulesError::Manifest(String::from("malformed Cargo TOML")))?;
    let mut names = BTreeMap::new();
    if let Some(table) = document.as_table() {
        collect_cargo_sections(table, &mut names)?;
        if let Some(workspace) = table.get("workspace").and_then(toml::Value::as_table) {
            collect_cargo_sections(workspace, &mut names)?;
        }
        if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
            for target in targets.values().filter_map(toml::Value::as_table) {
                collect_cargo_sections(target, &mut names)?;
            }
        }
    }
    Ok(names)
}

fn collect_cargo_sections(
    table: &toml::map::Map<String, toml::Value>,
    names: &mut BTreeMap<String, String>,
) -> Result<(), RulesError> {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = table.get(section).and_then(toml::Value::as_table) else {
            continue;
        };
        for (alias, spec) in dependencies {
            let renamed = spec
                .as_table()
                .and_then(|spec| spec.get("package"))
                .and_then(toml::Value::as_str);
            let package = renamed.map_or(alias.as_str(), |name| name);
            insert_package_name(PackageEcosystem::Cargo, package, names)?;
        }
    }
    Ok(())
}

fn parse_npm_names(input: &str) -> Result<BTreeMap<String, String>, RulesError> {
    let document = serde_json::from_str::<serde_json::Value>(input)
        .map_err(|_| RulesError::Manifest(String::from("malformed npm JSON")))?;
    let mut names = BTreeMap::new();
    let Some(root) = document.as_object() else {
        return Err(RulesError::Manifest(String::from(
            "package.json root must be an object",
        )));
    };
    for section in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(dependencies) = root.get(section).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (alias, spec) in dependencies {
            let alias_target = spec.as_str().and_then(npm_alias_target);
            let package = alias_target.map_or(alias.as_str(), |name| name);
            insert_package_name(PackageEcosystem::Npm, package, &mut names)?;
        }
    }
    Ok(names)
}

fn npm_alias_target(spec: &str) -> Option<&str> {
    let alias = spec.strip_prefix("npm:")?;
    let Some(version_separator) = alias.rfind('@') else {
        return Some(alias);
    };
    if version_separator == 0 {
        return Some(alias);
    }
    alias.get(..version_separator)
}

fn parse_python_toml_names(input: &str) -> Result<BTreeMap<String, String>, RulesError> {
    let document = toml::from_str::<toml::Value>(input)
        .map_err(|_| RulesError::Manifest(String::from("malformed Python TOML")))?;
    let mut names = BTreeMap::new();
    let Some(root) = document.as_table() else {
        return Err(RulesError::Manifest(String::from(
            "Python project manifest root must be a table",
        )));
    };
    if let Some(project) = root.get("project").and_then(toml::Value::as_table) {
        if let Some(dependencies) = project.get("dependencies").and_then(toml::Value::as_array) {
            for dependency in dependencies.iter().filter_map(toml::Value::as_str) {
                if let Some(name) = pep508_name(dependency) {
                    insert_package_name(PackageEcosystem::Python, name, &mut names)?;
                }
            }
        }
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(toml::Value::as_table)
        {
            for dependencies in optional.values().filter_map(toml::Value::as_array) {
                for dependency in dependencies.iter().filter_map(toml::Value::as_str) {
                    if let Some(name) = pep508_name(dependency) {
                        insert_package_name(PackageEcosystem::Python, name, &mut names)?;
                    }
                }
            }
        }
    }
    if let Some(tool) = root.get("tool").and_then(toml::Value::as_table)
        && let Some(poetry) = tool.get("poetry").and_then(toml::Value::as_table)
    {
        if let Some(dependencies) = poetry.get("dependencies").and_then(toml::Value::as_table) {
            collect_python_dependency_keys(dependencies, &mut names)?;
        }
        if let Some(groups) = poetry.get("group").and_then(toml::Value::as_table) {
            for group in groups.values().filter_map(toml::Value::as_table) {
                if let Some(dependencies) =
                    group.get("dependencies").and_then(toml::Value::as_table)
                {
                    collect_python_dependency_keys(dependencies, &mut names)?;
                }
            }
        }
    }
    Ok(names)
}

fn collect_python_dependency_keys(
    dependencies: &toml::map::Map<String, toml::Value>,
    names: &mut BTreeMap<String, String>,
) -> Result<(), RulesError> {
    for name in dependencies.keys().filter(|name| name.as_str() != "python") {
        insert_package_name(PackageEcosystem::Python, name, names)?;
    }
    Ok(())
}

fn parse_requirements_names(input: &str) -> Result<BTreeMap<String, String>, RulesError> {
    let mut names = BTreeMap::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let Some(requirement_head) = line.split(';').next() else {
            continue;
        };
        let Some(requirement_word) = requirement_head.split_whitespace().next() else {
            continue;
        };
        let Some(name) = requirement_word
            .split(['<', '>', '=', '!', '~', '['])
            .next()
        else {
            continue;
        };
        if !name.is_empty() {
            insert_package_name(PackageEcosystem::Python, name, &mut names)?;
        }
    }
    Ok(names)
}

fn pep508_name(requirement: &str) -> Option<&str> {
    let head = requirement.split(';').next()?.trim();
    let first = head.split_whitespace().next()?;
    let name = first.split(['<', '>', '=', '!', '~', '[']).next()?;
    (!name.is_empty()).then_some(name)
}

fn insert_package_name(
    ecosystem: PackageEcosystem,
    name: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<(), RulesError> {
    validate_package_name(ecosystem, name)?;
    if names.len() >= MAX_MANIFEST_PACKAGES {
        let normalized = normalize_package_name(ecosystem, name);
        if !names.contains_key(&normalized) {
            return Err(RulesError::Bound(String::from(
                "manifest dependency count exceeds its bound",
            )));
        }
    }
    let normalized = normalize_package_name(ecosystem, name);
    names.entry(normalized).or_insert_with(|| name.to_owned());
    Ok(())
}

fn validate_package_name(ecosystem: PackageEcosystem, name: &str) -> Result<(), RulesError> {
    if name.is_empty()
        || name.len() > MAX_BASELINE_NAME_BYTES
        || !name.is_ascii()
        || name.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.')
                || (ecosystem == PackageEcosystem::Npm && matches!(byte, b'@' | b'/')))
        })
    {
        return Err(RulesError::Manifest(String::from(
            "package name is invalid or exceeds its bound",
        )));
    }
    Ok(())
}

fn normalize_package_name(ecosystem: PackageEcosystem, name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut previous_python_separator = false;
    for byte in name.bytes().map(|byte| byte.to_ascii_lowercase()) {
        if ecosystem == PackageEcosystem::Python && matches!(byte, b'-' | b'_' | b'.') {
            if !previous_python_separator {
                normalized.push('-');
            }
            previous_python_separator = true;
            continue;
        }
        previous_python_separator = false;
        let normalized_byte = match (ecosystem, byte) {
            (PackageEcosystem::Cargo, b'_') => b'-',
            _ => byte,
        };
        normalized.push(char::from(normalized_byte));
    }
    normalized
}

fn compare_package_names(
    ecosystem: PackageEcosystem,
    path: &Path,
    package_names: BTreeMap<String, String>,
    baseline: &TrustedPackageBaseline,
) -> Result<Vec<TyposquatFinding>, RulesError> {
    let mut comparisons = 0_usize;
    let mut findings = Vec::new();
    for (normalized, declared_name) in package_names {
        if baseline.names.contains_key(&normalized) {
            continue;
        }
        let threshold = distance_threshold(normalized.len());
        let low = normalized.len().saturating_sub(usize::from(threshold));
        let high = normalized
            .len()
            .checked_add(usize::from(threshold))
            .ok_or_else(|| RulesError::Bound(String::from("package comparison bound overflow")))?;
        let mut best_distance = None;
        let mut candidates = Vec::new();
        for length in low..=high {
            let Some(names) = baseline.names_by_length.get(&length) else {
                continue;
            };
            for (candidate, display_name) in names {
                comparisons = comparisons.checked_add(1).ok_or_else(|| {
                    RulesError::Bound(String::from("package comparison count overflow"))
                })?;
                if comparisons > MAX_PACKAGE_COMPARISONS {
                    return Err(RulesError::Bound(String::from(
                        "package-name comparison budget exceeded",
                    )));
                }
                let Some(distance) = edit_distance_at_most(&normalized, candidate, threshold)?
                else {
                    continue;
                };
                match best_distance {
                    None => {
                        best_distance = Some(distance);
                        candidates.push(display_name.clone());
                    }
                    Some(best) if distance < best => {
                        best_distance = Some(distance);
                        candidates.clear();
                        candidates.push(display_name.clone());
                    }
                    Some(best) if distance == best => candidates.push(display_name.clone()),
                    Some(_) => {}
                }
            }
        }
        let Some(distance) = best_distance else {
            continue;
        };
        candidates.sort_unstable();
        candidates.dedup();
        let ambiguous = candidates.len() > 1;
        candidates.truncate(MAX_TIED_CANDIDATES);
        findings.push(TyposquatFinding {
            ecosystem,
            manifest_path: path.to_path_buf(),
            package_name: declared_name,
            nearest_known_names: candidates,
            distance,
            ambiguous,
        });
    }
    Ok(findings)
}

const fn distance_threshold(name_length: usize) -> u8 {
    if name_length < 5 {
        1
    } else {
        MAX_EDIT_DISTANCE
    }
}

fn edit_distance_at_most(
    left: &str,
    right: &str,
    max_distance: u8,
) -> Result<Option<u8>, RulesError> {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let difference = left_bytes.len().abs_diff(right_bytes.len());
    if difference > usize::from(max_distance) {
        return Ok(None);
    }
    let row_length = right_bytes
        .len()
        .checked_add(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance row length overflow")))?;
    let sentinel = max_distance
        .checked_add(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance threshold overflow")))?;
    let mut previous = vec![sentinel; row_length];
    let mut previous_previous = vec![sentinel; row_length];
    let initial_limit = right_bytes.len().min(usize::from(max_distance));
    for column in 0..=initial_limit {
        let cell = previous.get_mut(column).ok_or_else(|| {
            RulesError::Bound(String::from("edit-distance initial row is incomplete"))
        })?;
        *cell = u8::try_from(column).map_err(|_| {
            RulesError::Bound(String::from("edit-distance name length exceeds byte bound"))
        })?;
    }
    for (left_index, left_byte) in left_bytes.iter().enumerate() {
        let row_number = left_index
            .checked_add(1)
            .ok_or_else(|| RulesError::Bound(String::from("edit-distance row overflow")))?;
        let mut current = vec![sentinel; row_length];
        let Some(first_cell) = current.first_mut() else {
            return Err(RulesError::Bound(String::from(
                "edit-distance row is unexpectedly empty",
            )));
        };
        *first_cell = u8::try_from(row_number).map_or(sentinel, |value| value.min(sentinel));
        let band = usize::from(max_distance);
        let start_column = row_number.saturating_sub(band).max(1);
        let end_column = row_number
            .checked_add(band)
            .ok_or_else(|| RulesError::Bound(String::from("edit-distance band overflow")))?
            .min(right_bytes.len());
        for column in start_column..=end_column {
            let right_index = column
                .checked_sub(1)
                .ok_or_else(|| RulesError::Bound(String::from("edit-distance column underflow")))?;
            let Some(right_byte) = right_bytes.get(right_index) else {
                return Err(RulesError::Bound(String::from(
                    "edit-distance input column is unavailable",
                )));
            };
            let Some(above) = previous.get(column).copied() else {
                return Err(RulesError::Bound(String::from(
                    "edit-distance previous row is incomplete",
                )));
            };
            let Some(diagonal) = previous.get(right_index).copied() else {
                return Err(RulesError::Bound(String::from(
                    "edit-distance diagonal is unavailable",
                )));
            };
            let Some(left_value) = current.get(right_index).copied() else {
                return Err(RulesError::Bound(String::from(
                    "edit-distance current row is incomplete",
                )));
            };
            let ordinary_distance = ordinary_edit_distance(
                above,
                diagonal,
                left_value,
                *left_byte,
                *right_byte,
                sentinel,
            )?;
            let transposition = transposition_distance(
                left_bytes,
                right_bytes,
                left_index,
                right_index,
                column,
                &previous_previous,
                sentinel,
            )?;
            let Some(cell) = current.get_mut(column) else {
                return Err(RulesError::Bound(String::from(
                    "edit-distance output cell is unavailable",
                )));
            };
            *cell = ordinary_distance.min(transposition);
        }
        previous_previous = previous;
        previous = current;
    }
    let Some(distance) = previous.last().copied() else {
        return Err(RulesError::Bound(String::from(
            "edit-distance output is unexpectedly empty",
        )));
    };
    Ok((distance <= max_distance).then_some(distance))
}

fn ordinary_edit_distance(
    above: u8,
    diagonal: u8,
    left_value: u8,
    left_byte: u8,
    right_byte: u8,
    sentinel: u8,
) -> Result<u8, RulesError> {
    let edit_cost = u8::from(left_byte != right_byte);
    let deletion = above
        .checked_add(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance arithmetic overflow")))?
        .min(sentinel);
    let insertion = left_value
        .checked_add(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance arithmetic overflow")))?
        .min(sentinel);
    let substitution = diagonal
        .checked_add(edit_cost)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance arithmetic overflow")))?
        .min(sentinel);
    Ok(deletion.min(insertion).min(substitution))
}

fn transposition_distance(
    left_bytes: &[u8],
    right_bytes: &[u8],
    left_index: usize,
    right_index: usize,
    column: usize,
    previous_previous: &[u8],
    sentinel: u8,
) -> Result<u8, RulesError> {
    if left_index == 0 || column <= 1 {
        return Ok(sentinel);
    }
    let left_previous_index = left_index
        .checked_sub(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance left index underflow")))?;
    let right_previous_index = right_index
        .checked_sub(1)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance right index underflow")))?;
    let left_previous = left_bytes.get(left_previous_index).ok_or_else(|| {
        RulesError::Bound(String::from("edit-distance previous left byte missing"))
    })?;
    let right_previous = right_bytes.get(right_previous_index).ok_or_else(|| {
        RulesError::Bound(String::from("edit-distance previous right byte missing"))
    })?;
    let Some(left_byte) = left_bytes.get(left_index) else {
        return Err(RulesError::Bound(String::from(
            "edit-distance current left byte missing",
        )));
    };
    let Some(right_byte) = right_bytes.get(right_index) else {
        return Err(RulesError::Bound(String::from(
            "edit-distance current right byte missing",
        )));
    };
    if left_byte != right_previous || left_previous != right_byte {
        return Ok(sentinel);
    }
    let two_back_column = column
        .checked_sub(2)
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance transposition underflow")))?;
    let two_back = previous_previous
        .get(two_back_column)
        .copied()
        .ok_or_else(|| {
            RulesError::Bound(String::from(
                "edit-distance transposition row is incomplete",
            ))
        })?;
    two_back
        .checked_add(1)
        .map(|distance| distance.min(sentinel))
        .ok_or_else(|| RulesError::Bound(String::from("edit-distance arithmetic overflow")))
}
