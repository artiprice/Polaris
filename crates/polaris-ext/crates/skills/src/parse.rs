use std::collections::BTreeSet;
use std::path::Path;

use polaris_sdk::ids::Cid;
use pulldown_cmark::{Event, Parser, Tag};
use serde::Deserialize;

use crate::SkillError;

pub const MAX_SKILL_BYTES: usize = 256 * 1024;
pub const MAX_FRONT_MATTER_BYTES: usize = 8 * 1024;
pub const MAX_SKILL_BODY_BYTES: usize = MAX_SKILL_BYTES - MAX_FRONT_MATTER_BYTES;
const MAX_DESCRIPTION_BYTES: usize = 2 * 1024;
const MAX_REQUIRED_TOOLS: usize = 32;
const MAX_RESOURCE_REFERENCES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillName(String);

impl SkillName {
    pub fn new(value: &str) -> Result<Self, SkillError> {
        let valid = !value.is_empty()
            && value.len() <= 64
            && !value.starts_with('-')
            && !value.ends_with('-')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err(SkillError::InvalidName(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSkill {
    pub name: SkillName,
    pub description: String,
    pub requires_tools: Vec<String>,
    pub body: String,
    pub body_blake3: Cid,
    pub source_blake3: Cid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    name: String,
    description: String,
    #[serde(default)]
    requires_tools: Vec<String>,
}

pub fn parse_skill(src: &str) -> Result<ParsedSkill, SkillError> {
    if src.len() > MAX_SKILL_BYTES {
        return Err(SkillError::TooLarge);
    }
    let mut lines = src.split_inclusive('\n');
    let first = lines.next().ok_or(SkillError::FrontMatter)?;
    if first.trim_end_matches(['\r', '\n']) != "---" {
        return Err(SkillError::FrontMatter);
    }
    let mut offset = first.len();
    let mut closing = None;
    for line in lines {
        if offset > MAX_FRONT_MATTER_BYTES {
            return Err(SkillError::FrontMatterTooLarge);
        }
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closing = Some((offset, offset + line.len()));
            break;
        }
        offset = offset.checked_add(line.len()).ok_or(SkillError::TooLarge)?;
    }
    let (yaml_end, body_start) = closing.ok_or(SkillError::FrontMatter)?;
    let yaml_start = first.len();
    let yaml = src
        .get(yaml_start..yaml_end)
        .ok_or(SkillError::FrontMatter)?;
    if yaml.len() > MAX_FRONT_MATTER_BYTES {
        return Err(SkillError::FrontMatterTooLarge);
    }
    let body = src.get(body_start..).ok_or(SkillError::FrontMatter)?;
    if body.len() > MAX_SKILL_BODY_BYTES {
        return Err(SkillError::BodyTooLarge);
    }
    let front: FrontMatter = serde_saphyr::from_str(yaml)
        .map_err(|error| SkillError::InvalidFrontMatter(error.to_string()))?;
    let name = SkillName::new(&front.name)?;
    if front.description.trim().is_empty()
        || front.description.len() > MAX_DESCRIPTION_BYTES
        || front.description.chars().any(char::is_control)
    {
        return Err(SkillError::InvalidDescription);
    }
    if front.requires_tools.len() > MAX_REQUIRED_TOOLS {
        return Err(SkillError::TooManyRequiredTools);
    }
    let mut unique_tools = BTreeSet::new();
    for tool in &front.requires_tools {
        if tool.is_empty()
            || tool.len() > 128
            || !tool
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            || !unique_tools.insert(tool.as_str())
        {
            return Err(SkillError::InvalidRequiredTool(tool.clone()));
        }
    }
    validate_resource_links(body)?;
    Ok(ParsedSkill {
        name,
        description: front.description,
        requires_tools: front.requires_tools,
        body: body.to_owned(),
        body_blake3: Cid::from_bytes(*blake3::hash(body.as_bytes()).as_bytes()),
        source_blake3: Cid::from_bytes(*blake3::hash(src.as_bytes()).as_bytes()),
    })
}

fn validate_resource_links(body: &str) -> Result<(), SkillError> {
    let mut seen = 0_usize;
    for event in Parser::new(body) {
        let (Event::Start(Tag::Link { dest_url, .. }) | Event::Start(Tag::Image { dest_url, .. })) =
            event
        else {
            continue;
        };
        seen = seen.checked_add(1).ok_or(SkillError::TooManyResources)?;
        if seen > MAX_RESOURCE_REFERENCES {
            return Err(SkillError::TooManyResources);
        }
        validate_resource_target(&dest_url)?;
    }
    Ok(())
}

fn validate_resource_target(target: &str) -> Result<(), SkillError> {
    if target.is_empty()
        || target.starts_with('#')
        || target.contains("://")
        || target.starts_with("mailto:")
    {
        return Ok(());
    }
    let path = target
        .split(['?', '#'])
        .next()
        .ok_or(SkillError::InvalidResource)?;
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains(['\\', '\0', '%'])
    {
        return Err(SkillError::InvalidResource);
    }
    if path
        .split('/')
        .any(|part| part == ".." || part.contains(':'))
    {
        return Err(SkillError::InvalidResource);
    }
    let candidate = Path::new(path);
    if candidate.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err(SkillError::InvalidResource);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ext/crates/skills/src/parse.rs"]
mod tests;
