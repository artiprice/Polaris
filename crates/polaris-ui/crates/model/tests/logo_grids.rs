//! Logo Grids (descriptor: specs/descriptors/uimodel_logo_grids.json).

use std::error::Error;

use polaris_ui_model::logo::{COMPACT, EMBLEM, LogoRole, WELCOME};

type TestResult = Result<(), Box<dyn Error>>;

/// PLAN cell art, middle dot (`·`) replaced by an ordinary space.
const WELCOME_ROWS: [&str; 7] = [
    "    │  │  │    ",
    "  ┌─┴──┴──┴─┐  ",
    "  │    ╷    │  ",
    "──┤  ╶─✦─╴  ├──",
    "  │    ╵    │  ",
    "  └─┬──┬──┬─┘  ",
    "    │  │  │    ",
];

const COMPACT_ROWS: [&str; 5] = [
    "   │ │ │   ",
    " ┌─┴─┴─┴─┐ ",
    "─┤  ─✦─  ├─",
    " └─┬─┬─┬─┘ ",
    "   │ │ │   ",
];

#[test]
fn welcome_is_15_by_7() {
    assert_eq!(WELCOME.height(), 7);
    assert_eq!(WELCOME.width(), 15);
}

#[test]
fn compact_is_11_by_5() {
    assert_eq!(COMPACT.height(), 5);
    assert_eq!(COMPACT.width(), 11);
}

#[test]
fn emblem_is_11_by_1() {
    assert_eq!(EMBLEM.height(), 1);
    assert_eq!(EMBLEM.width(), 11);
}

#[test]
fn welcome_rows_exact() {
    assert_eq!(WELCOME.rows(), WELCOME_ROWS);
    for row in WELCOME.rows() {
        assert_eq!(row.chars().count(), 15, "row {row:?}");
    }
}

#[test]
fn compact_rows_exact() {
    assert_eq!(COMPACT.rows(), COMPACT_ROWS);
    for row in COMPACT.rows() {
        assert_eq!(row.chars().count(), 11, "row {row:?}");
    }
}

#[test]
fn emblem_is_star_flanked_by_four_spaces() {
    assert_eq!(EMBLEM.rows(), ["    ─✦─    "]);
}

#[test]
fn mint_highlight_coordinates() {
    assert_eq!(WELCOME.mint(), Some((7, 2)));
    assert_eq!(COMPACT.mint(), Some((5, 0)));
    assert_eq!(EMBLEM.mint(), None);
    assert_eq!(WELCOME.glyph(7, 2), Some('╷'));
    assert_eq!(COMPACT.glyph(5, 0), Some('│'));
}

#[test]
fn package_rectangles() -> TestResult {
    let w = WELCOME.package().ok_or("welcome package")?;
    assert_eq!((w.x0, w.x1, w.y0, w.y1), (2, 12, 1, 5));
    let c = COMPACT.package().ok_or("compact package")?;
    assert_eq!((c.x0, c.x1, c.y0, c.y1), (1, 9, 1, 3));
    assert!(EMBLEM.package().is_none());
    assert_eq!(WELCOME.glyph(w.x0, w.y0), Some('┌'));
    assert_eq!(WELCOME.glyph(w.x1, w.y0), Some('┐'));
    assert_eq!(WELCOME.glyph(w.x0, w.y1), Some('└'));
    assert_eq!(WELCOME.glyph(w.x1, w.y1), Some('┘'));
    assert_eq!(COMPACT.glyph(c.x0, c.y0), Some('┌'));
    assert_eq!(COMPACT.glyph(c.x1, c.y0), Some('┐'));
    assert_eq!(COMPACT.glyph(c.x0, c.y1), Some('└'));
    assert_eq!(COMPACT.glyph(c.x1, c.y1), Some('┘'));
    Ok(())
}

#[test]
fn ascii_fallback_maps_frame_and_star() {
    assert_eq!(
        WELCOME.ascii_rows(),
        [
            "    |  |  |    ",
            "  +-+--+--+-+  ",
            "  |    |    |  ",
            "--+  --*--  +--",
            "  |    |    |  ",
            "  +-+--+--+-+  ",
            "    |  |  |    ",
        ]
    );
    assert_eq!(
        COMPACT.ascii_rows(),
        [
            "   | | |   ",
            " +-+-+-+-+ ",
            "-+  -*-  +-",
            " +-+-+-+-+ ",
            "   | | |   ",
        ]
    );
    assert_eq!(EMBLEM.ascii_rows(), ["    -*-    "]);
}

#[test]
fn ascii_fallback_is_pure_ascii_and_same_size() {
    for grid in [&WELCOME, &COMPACT, &EMBLEM] {
        let ascii = grid.ascii_rows();
        assert_eq!(ascii.len(), grid.height());
        for row in &ascii {
            assert!(row.is_ascii(), "non-ASCII row {row:?}");
            assert_eq!(row.chars().count(), grid.width());
        }
    }
}

#[test]
fn roles_cover_star_mint_and_frame() {
    // Welcome: star, mint, upper/left frame light, lower/right frame dark, pins dark.
    assert_eq!(WELCOME.role_at(7, 3), Some(LogoRole::Star));
    assert_eq!(WELCOME.role_at(7, 2), Some(LogoRole::Mint));
    assert_eq!(WELCOME.role_at(6, 3), Some(LogoRole::Trace));
    assert_eq!(WELCOME.role_at(2, 1), Some(LogoRole::FrameLight));
    assert_eq!(WELCOME.role_at(2, 3), Some(LogoRole::FrameLight));
    assert_eq!(WELCOME.role_at(12, 1), Some(LogoRole::FrameLight));
    assert_eq!(WELCOME.role_at(12, 3), Some(LogoRole::FrameDark));
    assert_eq!(WELCOME.role_at(2, 5), Some(LogoRole::FrameLight));
    assert_eq!(WELCOME.role_at(12, 5), Some(LogoRole::FrameDark));
    assert_eq!(WELCOME.role_at(5, 5), Some(LogoRole::FrameDark));
    assert_eq!(WELCOME.role_at(4, 0), Some(LogoRole::FrameDark));
    assert_eq!(WELCOME.role_at(0, 3), Some(LogoRole::FrameDark));
    assert_eq!(WELCOME.role_at(4, 2), Some(LogoRole::Package));
    assert_eq!(WELCOME.role_at(0, 0), Some(LogoRole::Canvas));
    assert_eq!(WELCOME.role_at(15, 0), None);
    // Compact mint sits on the top-middle pin.
    assert_eq!(COMPACT.role_at(5, 0), Some(LogoRole::Mint));
    assert_eq!(COMPACT.role_at(5, 2), Some(LogoRole::Star));
    // Emblem has a star and canvas only.
    assert_eq!(EMBLEM.role_at(5, 0), Some(LogoRole::Star));
    assert_eq!(EMBLEM.role_at(4, 0), Some(LogoRole::Trace));
    assert_eq!(EMBLEM.role_at(0, 0), Some(LogoRole::Canvas));
}

#[test]
fn glyph_lookup_is_bounds_checked() {
    assert_eq!(WELCOME.glyph(15, 0), None);
    assert_eq!(WELCOME.glyph(0, 7), None);
    assert_eq!(EMBLEM.glyph(0, 1), None);
    assert_eq!(WELCOME.glyph(usize::MAX, usize::MAX), None);
    assert_eq!(WELCOME.glyph(0, 0), Some(' '));
}
