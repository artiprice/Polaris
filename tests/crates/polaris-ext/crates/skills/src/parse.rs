// Unit tests of crates/polaris-ext/crates/skills/src/parse.rs (`tests`); compiled into that module via #[path].
use super::*;

const VALID: &str = "---\nname: git-bisect\ndescription: Find the commit that introduced a regression.\nrequires_tools: [git_read]\n---\n# Bisect\r\n\r\nUse binary search over commits.\r\n";

#[test]
fn parses_yaml_and_preserves_body_bytes() {
    let parsed = parse_skill(VALID).expect("valid front matter should parse");
    assert_eq!(parsed.name.as_str(), "git-bisect");
    assert_eq!(parsed.requires_tools, ["git_read"]);
    assert_eq!(
        parsed.body,
        "# Bisect\r\n\r\nUse binary search over commits.\r\n"
    );
    assert_eq!(
        parsed.body_blake3,
        Cid::from_bytes(*blake3::hash(parsed.body.as_bytes()).as_bytes())
    );
}

#[test]
fn rejects_bad_metadata_and_unsafe_resource_paths() {
    let bad_name = VALID.replace("git-bisect", "../escape");
    assert!(matches!(
        parse_skill(&bad_name),
        Err(SkillError::InvalidName(_))
    ));

    let unsafe_link = format!("{VALID}[escape](../../secret.txt)");
    assert!(matches!(
        parse_skill(&unsafe_link),
        Err(SkillError::InvalidResource)
    ));

    let duplicate_tool = VALID.replace("[git_read]", "[git_read, git_read]");
    assert!(matches!(
        parse_skill(&duplicate_tool),
        Err(SkillError::InvalidRequiredTool(_))
    ));
}

#[test]
fn rejects_missing_front_matter_and_oversized_source() {
    assert!(matches!(
        parse_skill("# no front matter"),
        Err(SkillError::FrontMatter)
    ));
    let oversized = format!(
        "---\nname: test\ndescription: test\n---\n{}",
        "x".repeat(MAX_SKILL_BYTES)
    );
    assert!(matches!(parse_skill(&oversized), Err(SkillError::TooLarge)));
}

#[test]
fn markdown_external_links_are_not_treated_as_local_paths() {
    let src =
        format!("{VALID}[docs](https://example.invalid/docs) [mail](mailto:a@example.invalid)");
    assert!(parse_skill(&src).is_ok());
}

#[test]
fn bundled_fixtures_have_valid_front_matter() {
    let fixtures = [
        include_str!("../fixtures/git-bisect/SKILL.md"),
        include_str!("../fixtures/database-migrate/SKILL.md"),
    ];
    let parsed = fixtures
        .into_iter()
        .map(|source| parse_skill(source).expect("bundled fixture parses"))
        .collect::<Vec<_>>();
    assert_eq!(parsed[0].name.as_str(), "git-bisect");
    assert_eq!(parsed[1].name.as_str(), "database-migrate");
    assert!(!parsed[0].body.is_empty());
    assert!(!parsed[1].body.is_empty());
}
