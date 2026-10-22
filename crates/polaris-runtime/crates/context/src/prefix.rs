//! Deterministic cached-prefix representation.

use crate::ContextError;
use polaris_sdk::registry::ToolSpec;

/// BLAKE3 identity of the serialized stable prompt prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrefixHash([u8; 32]);

impl PrefixHash {
    /// Raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hexadecimal digest.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        use std::fmt::Write as _;
        for byte in self.0 {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Stable prefix: system prompt, deterministically ordered tool schemas, and a versioned repo map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StablePrefix {
    /// Canonical serialized bytes, using LF line endings.
    pub bytes: String,
    /// Identity of [`Self::bytes`].
    pub hash: PrefixHash,
    /// Tool definitions in the order represented in the canonical prefix.
    pub tools: Vec<ToolSpec>,
    /// Version tag for the repository map.
    pub repo_map_version: String,
}

/// Stable tool-profile identifier and the tool specifications included in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProfile {
    /// Versioned profile name chosen by the caller.
    pub name: String,
    /// Tools belonging to this profile; serialization sorts by name.
    pub tools: Vec<ToolSpec>,
}

impl StablePrefix {
    /// Construct a canonical prefix from input fields.
    ///
    /// Tool definitions are sorted by name; equal names are rejected because their relative
    /// order would otherwise make an ambiguous prompt contract. Input line endings are normalized
    /// to LF in the system and repo-map text. Tool schema JSON is kept byte-for-byte as supplied.
    pub fn new(
        system: &str,
        tools: &[ToolSpec],
        repo_map_version: &str,
        repo_map: &str,
    ) -> Result<Self, ContextError> {
        let mut sorted = tools.to_vec();
        sorted.sort_by(|left, right| left.name.cmp(&right.name));
        if sorted.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(a, b)| a.name == b.name)
        }) {
            return Err(ContextError::DuplicateToolName);
        }
        let system = normalize_lf(system);
        let repo_map = normalize_lf(repo_map);
        let bytes = serialize_prefix(&system, &sorted, repo_map_version, &repo_map)?;
        let digest = blake3::hash(bytes.as_bytes());
        Ok(Self {
            bytes,
            hash: PrefixHash(*digest.as_bytes()),
            tools: sorted,
            repo_map_version: repo_map_version.to_owned(),
        })
    }
}

/// Serialize prefix fields with length-delimited records, avoiding delimiter ambiguity.
///
/// The version tag is part of the byte stream. Lengths are decimal byte counts and always use
/// base-10 with no leading zeros. The output contains LF separators and no timestamps or ids.
pub fn serialize_prefix(
    system: &str,
    tools: &[ToolSpec],
    repo_map_version: &str,
    repo_map: &str,
) -> Result<String, ContextError> {
    let mut sorted = tools.to_vec();
    sorted.sort_by(|left, right| left.name.cmp(&right.name));
    if sorted.windows(2).any(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.name == b.name)
    }) {
        return Err(ContextError::DuplicateToolName);
    }
    let mut out = String::new();
    out.push_str("polaris-prefix-v1\n");
    write_field(&mut out, "system", &normalize_lf(system))?;
    for tool in &sorted {
        write_field(&mut out, "tool.name", &tool.name)?;
        write_field(&mut out, "tool.description", &tool.description)?;
        write_field(&mut out, "tool.schema", &tool.input_schema_json)?;
        let effects = format!(
            "{}{}{}{}",
            bool_digit(tool.effects.reads_workspace),
            bool_digit(tool.effects.writes_workspace),
            bool_digit(tool.effects.network),
            bool_digit(tool.effects.executes)
        );
        write_field(&mut out, "tool.effects", &effects)?;
    }
    write_field(&mut out, "repo.version", repo_map_version)?;
    write_field(&mut out, "repo.map", &normalize_lf(repo_map))?;
    Ok(out)
}

fn write_field(out: &mut String, name: &str, value: &str) -> Result<(), ContextError> {
    let length = u64::try_from(value.len()).map_err(|_| ContextError::PrefixTooLarge)?;
    use std::fmt::Write as _;
    writeln!(out, "{name}:{length}").map_err(|_| ContextError::PrefixTooLarge)?;
    out.push_str(value);
    out.push('\n');
    Ok(())
}

fn normalize_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn bool_digit(value: bool) -> char {
    if value { '1' } else { '0' }
}
