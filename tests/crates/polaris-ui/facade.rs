//! The UI facade names every child crate and exposes the shared design tokens.

use std::collections::BTreeSet;

#[test]
fn design_tokens_are_reachable_through_the_facade_and_well_formed() {
    let roles = polaris_ui::model::tokens::ROLES;
    assert!(!roles.is_empty());
    let mut names = BTreeSet::new();
    for role in roles {
        assert!(names.insert(role.name), "duplicate role {}", role.name);
        let digits = role.hex.strip_prefix('#').unwrap_or_default();
        assert!(
            matches!(digits.len(), 6 | 8) && digits.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{} has colour {}",
            role.name,
            role.hex
        );
    }
    assert!(polaris_ui::model::tokens::RADIUS_PX > 0);
    assert!(polaris_ui::model::tokens::SPEED_MS > 0);
}

#[test]
fn the_session_host_summary_is_shared_by_every_frontend() {
    let report = polaris_ui::session::TurnReport {
        answer: Some("Done.".to_owned()),
        attempts: 2,
        tool_rounds: 1,
        stop: "Completed".to_owned(),
        elapsed: std::time::Duration::from_secs(3),
        changed_files: vec!["notes.md".to_owned()],
    };
    assert_eq!(
        report.summary(),
        "Worked for 3s · 2 requests · 1 tool round · 1 file changed"
    );
}
