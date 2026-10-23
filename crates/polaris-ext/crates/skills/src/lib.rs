//! Bounded, trust-aware loading and selection of procedural `SKILL.md` context.
//!
//! Repository skills are untrusted prompt input until a trusted caller supplies an approval
//! witness bound to the indexed source digest. Skills provide context and tool suggestions only;
//! they do not grant capabilities. Bodies are re-read only after selection and are never sent to
//! the relevance engine.

mod approval_cache;
mod discover;
mod parse;
mod select;

pub use approval_cache::SkillApprovalCache;
pub use discover::{
    MAX_SKILLS, Skill, SkillMeta, SkillRegistry, SkillRoot, SkillRootKind, SkillSource,
};
pub use parse::{
    MAX_FRONT_MATTER_BYTES, MAX_SKILL_BODY_BYTES, MAX_SKILL_BYTES, ParsedSkill, SkillName,
    parse_skill,
};
pub use select::{RankedSkill, RelevanceVerdict, SelectionReason, TopK};

/// Fail-closed parsing, discovery, trust, and selection errors.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill I/O failed: {0}")]
    Io(String),
    #[error("skill is missing valid YAML front matter")]
    FrontMatter,
    #[error("skill front matter exceeds {MAX_FRONT_MATTER_BYTES} bytes")]
    FrontMatterTooLarge,
    #[error("skill source exceeds {MAX_SKILL_BYTES} bytes")]
    TooLarge,
    #[error("skill body exceeds {MAX_SKILL_BODY_BYTES} bytes")]
    BodyTooLarge,
    #[error("invalid YAML skill front matter: {0}")]
    InvalidFrontMatter(String),
    #[error("invalid skill name: {0}")]
    InvalidName(String),
    #[error("skill description is empty, too long, or contains control characters")]
    InvalidDescription,
    #[error("skill declares too many required tools")]
    TooManyRequiredTools,
    #[error("invalid or duplicate required tool: {0}")]
    InvalidRequiredTool(String),
    #[error("skill contains too many local resource references")]
    TooManyResources,
    #[error("skill contains an unsafe local resource reference")]
    InvalidResource,
    #[error("skill index exceeds configured bounds")]
    LimitExceeded,
    #[error("invalid skill root: {0}")]
    InvalidRoot(std::path::PathBuf),
    #[error("skill is not valid UTF-8: {0}")]
    InvalidUtf8(std::path::PathBuf),
    #[error("skill path escaped its root: {0}")]
    PathEscape(std::path::PathBuf),
    #[error("skill changed during a bounded read: {0}")]
    ChangedDuringRead(std::path::PathBuf),
    #[error("skill name collision: {0}")]
    NameCollision(SkillName),
    #[error("skill is not approved: {0}")]
    NotApproved(SkillName),
    #[error("approval does not bind the current skill source: {0}")]
    ApprovalMismatch(SkillName),
    #[error("approved skill changed and needs approval again: {0}")]
    ApprovalStale(SkillName),
    #[error("skill index is stale; rediscover before loading: {0}")]
    StaleIndex(SkillName),
    #[error("unknown skill: {0}")]
    Unknown(SkillName),
    #[error("task text exceeds the bounded selector input")]
    TaskTooLarge,
    #[error("selection request has invalid bounds")]
    InvalidTopK,
    #[error("skill selection failed: {0}")]
    Selection(String),
    #[error("required skill relevance schema is not available")]
    SchemaUnavailable,
    #[error("skill relevance schema does not match the required canonical shape")]
    InvalidSelectionSchema,
    #[error("decision engine failed under a closed skill selection policy: {0}")]
    ClosedSelection(String),
    #[error("skill schema catalog failed: {0}")]
    SchemaCatalog(#[from] polaris_sdk::system_one::DecisionSchemaError),
    #[error("skill approval persistence failed: {0}")]
    ApprovalCache(String),
}
