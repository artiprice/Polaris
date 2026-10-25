//! Logo Grids (descriptor: specs/descriptors/uimodel_logo_grids.json).

use polaris_ui_model::logo::{COMPACT, EMBLEM, LogoRole, WELCOME};

const WELCOME_ROWS: [&str; 5] = [
    "    ╷    ",
    "    │    ",
    "╶───✦───╴",
    "    │    ",
    "    ╵    ",
];
const COMPACT_ROWS: [&str; 3] = ["  ╷  ", "╶─✦─╴", "  ╵  "];

#[test]
fn welcome_is_9_by_5() {
    assert_eq!((WELCOME.width(), WELCOME.height()), (9, 5));
}
#[test]
fn compact_is_5_by_3() {
    assert_eq!((COMPACT.width(), COMPACT.height()), (5, 3));
}
#[test]
fn emblem_is_one_star_cell() {
    assert_eq!((EMBLEM.width(), EMBLEM.height()), (1, 1));
    assert_eq!(EMBLEM.rows(), ["✦"]);
}
#[test]
fn welcome_rows_exact() {
    assert_eq!(WELCOME.rows(), WELCOME_ROWS);
}
#[test]
fn compact_rows_exact() {
    assert_eq!(COMPACT.rows(), COMPACT_ROWS);
}
#[test]
fn ascii_fallback_keeps_dimensions() {
    assert_eq!(
        WELCOME.ascii_rows(),
        [
            "    |    ",
            "    |    ",
            "----*----",
            "    |    ",
            "    |    "
        ]
    );
    assert_eq!(COMPACT.ascii_rows(), ["  |  ", "--*--", "  |  "]);
    assert_eq!(EMBLEM.ascii_rows(), ["*"]);
    for grid in [&WELCOME, &COMPACT, &EMBLEM] {
        let rows = grid.ascii_rows();
        assert_eq!(rows.len(), grid.height());
        for row in rows {
            assert!(row.is_ascii());
            assert_eq!(row.chars().count(), grid.width());
        }
    }
}
#[test]
fn roles_follow_star_rays_and_gray_tips() {
    assert_eq!(WELCOME.role_at(4, 2), Some(LogoRole::Star));
    assert_eq!(WELCOME.role_at(3, 2), Some(LogoRole::Ray));
    assert_eq!(WELCOME.role_at(0, 2), Some(LogoRole::Tip));
    assert_eq!(WELCOME.role_at(4, 0), Some(LogoRole::Tip));
    assert_eq!(WELCOME.role_at(0, 0), Some(LogoRole::Canvas));
    assert_eq!(COMPACT.role_at(2, 1), Some(LogoRole::Star));
    assert_eq!(EMBLEM.role_at(0, 0), Some(LogoRole::Star));
}
#[test]
fn glyph_lookup_is_bounds_checked() {
    assert_eq!(WELCOME.glyph(9, 0), None);
    assert_eq!(WELCOME.glyph(0, 5), None);
    assert_eq!(EMBLEM.glyph(0, 1), None);
    assert_eq!(WELCOME.glyph(0, 0), Some(' '));
}
