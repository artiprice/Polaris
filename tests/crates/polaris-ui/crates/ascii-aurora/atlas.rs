//! Tests for the glyph atlas (descriptor `ui_ascii_aurora_atlas.json`).

use polaris_ui_ascii_aurora::{AuroraError, GlyphAtlas, RAMP};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../gui/assets/fonts/jetbrains-mono.ttf"
));

#[test]
fn bakes_every_ramp_glyph() -> TestResult {
    let atlas = GlyphAtlas::bake(FONT, RAMP, 10, 15)?;
    assert_eq!(atlas.len(), RAMP.chars().count());
    assert!(!atlas.is_empty());
    for (i, c) in RAMP.chars().enumerate() {
        assert_eq!(atlas.glyph_char(i), Some(c));
        assert!(atlas.glyph(i).is_some());
    }
    assert!(atlas.glyph(atlas.len()).is_none());
    Ok(())
}

#[test]
fn missing_glyph_reports_char() {
    let r = GlyphAtlas::bake(FONT, " .\u{E000}", 10, 15);
    assert!(
        matches!(r, Err(AuroraError::MissingGlyph('\u{E000}'))),
        "{r:?}"
    );
}

#[test]
fn invalid_font_bytes_rejected() {
    let r = GlyphAtlas::bake(&[0_u8, 1, 2, 3, 4, 5, 6, 7], RAMP, 10, 15);
    assert!(matches!(r, Err(AuroraError::Font)), "{r:?}");
    let r = GlyphAtlas::bake(&[], RAMP, 10, 15);
    assert!(matches!(r, Err(AuroraError::Font)), "{r:?}");
}

#[test]
fn coverage_bitmaps_are_cell_sized() -> TestResult {
    for (w, h) in [(10_u16, 15_u16), (7, 11), (16, 24)] {
        let atlas = GlyphAtlas::bake(FONT, RAMP, w, h)?;
        assert_eq!(atlas.cell_w(), usize::from(w));
        assert_eq!(atlas.cell_h(), usize::from(h));
        for i in 0..atlas.len() {
            let g = atlas.glyph(i).ok_or("glyph")?;
            assert_eq!(g.len(), usize::from(w) * usize::from(h));
            for y in 0..usize::from(h) {
                assert_eq!(atlas.row(i, y).ok_or("row")?.len(), usize::from(w));
            }
            assert!(atlas.row(i, usize::from(h)).is_none());
        }
    }
    Ok(())
}

#[test]
fn bake_is_deterministic() -> TestResult {
    let a = GlyphAtlas::bake(FONT, RAMP, 10, 15)?;
    let b = GlyphAtlas::bake(FONT, RAMP, 10, 15)?;
    for i in 0..a.len() {
        assert_eq!(a.glyph(i), b.glyph(i));
        assert_eq!(a.ink(i), b.ink(i));
    }
    Ok(())
}

#[test]
fn zero_cell_size_rejected() {
    assert!(matches!(
        GlyphAtlas::bake(FONT, RAMP, 0, 15),
        Err(AuroraError::Size)
    ));
    assert!(matches!(
        GlyphAtlas::bake(FONT, RAMP, 10, 0),
        Err(AuroraError::Size)
    ));
}

#[test]
fn space_is_blank_and_dense_glyph_has_ink() -> TestResult {
    let (w, h) = (10_u16, 15_u16);
    let atlas = GlyphAtlas::bake(FONT, RAMP, w, h)?;
    assert_eq!(atlas.ink(0), Some(0));
    assert!(atlas.glyph(0).ok_or("space")?.iter().all(|&v| v == 0));
    let dense = atlas.len() - 1;
    assert!(atlas.ink(dense).ok_or("ink")? > 500);
    // Bounding box of a cap-height glyph ('X'): centred, about 0.62 of the cell height.
    let x_idx = (0..atlas.len())
        .find(|&i| atlas.glyph_char(i) == Some('X'))
        .ok_or("no X in ramp")?;
    let bitmap = atlas.glyph(x_idx).ok_or("X bitmap")?;
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
    for (i, &v) in bitmap.iter().enumerate() {
        if v > 40 {
            let (x, y) = (i % usize::from(w), i / usize::from(w));
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    assert!(x0 <= x1 && y0 <= y1, "no ink found");
    let glyph_h = f32::from(u16::try_from(y1 - y0 + 1)?);
    let ratio = glyph_h / f32::from(h);
    assert!((0.5..=0.75).contains(&ratio), "glyph height ratio {ratio}");
    // Centred within one pixel on each axis.
    let left = x0;
    let right = usize::from(w) - 1 - x1;
    let top = y0;
    let bottom = usize::from(h) - 1 - y1;
    assert!(
        left.abs_diff(right) <= 2,
        "horizontal margins {left}/{right}"
    );
    assert!(top.abs_diff(bottom) <= 2, "vertical margins {top}/{bottom}");
    Ok(())
}
