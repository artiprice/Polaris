//! Tests for the glyph ramp (descriptor `ui_ascii_aurora_ramp.json`).

use std::collections::BTreeSet;

use polaris_ui_ascii_aurora::ramp::{RAMP_LEN, glyph_index, tile_levels};
use polaris_ui_ascii_aurora::{GlyphAtlas, RAMP};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../gui/assets/fonts/jetbrains-mono.ttf"
));

#[test]
fn ramp_is_thirteen_unique_glyphs() {
    let unique: BTreeSet<char> = RAMP.chars().collect();
    assert_eq!(RAMP.chars().count(), 13);
    assert_eq!(RAMP.chars().count(), RAMP_LEN);
    assert_eq!(unique.len(), 13);
    assert_eq!(RAMP.chars().next(), Some(' '));
}

#[test]
fn ramp_ink_monotonic_along_ramp() -> TestResult {
    for (w, h) in [(10_u16, 15_u16), (8, 12), (7, 11)] {
        let atlas = GlyphAtlas::bake(FONT, RAMP, w, h)?;
        let mut last = 0_u32;
        for i in 0..atlas.len() {
            let ink = atlas.ink(i).ok_or("missing ink")?;
            assert!(
                ink >= last,
                "cell {w}x{h}: ink of {:?} ({ink}) < previous ({last})",
                atlas.glyph_char(i)
            );
            last = ink;
        }
    }
    Ok(())
}

#[test]
fn zero_luminance_is_space() {
    assert_eq!(glyph_index(0.0), 0);
    assert_eq!(RAMP.chars().nth(glyph_index(0.0)), Some(' '));
    assert_eq!(glyph_index(0.01), 0);
}

#[test]
fn out_of_range_and_nan_clamp() {
    assert_eq!(glyph_index(f32::NAN), 0);
    assert_eq!(glyph_index(-5.0), 0);
    assert_eq!(glyph_index(f32::NEG_INFINITY), 0);
    assert_eq!(glyph_index(1.0), RAMP_LEN - 1);
    assert_eq!(glyph_index(7.5), RAMP_LEN - 1);
    assert_eq!(glyph_index(f32::INFINITY), RAMP_LEN - 1);
    let (bg, fg, inv) = tile_levels(f32::NAN);
    assert!(bg == 0.0 && !inv && fg.is_finite());
}

proptest! {
    #[test]
    fn prop_glyph_index_below_ramp_len(bits in any::<u32>()) {
        let l = f32::from_bits(bits);
        prop_assert!(glyph_index(l) < RAMP_LEN);
        let (bg, fg, _) = tile_levels(l);
        prop_assert!(bg.is_finite() && fg.is_finite());
    }
}

#[test]
fn glyph_index_is_monotonic() {
    let mut last = 0_usize;
    for step in 0..=2000_u16 {
        let l = f32::from(step) / 2000.0;
        let i = glyph_index(l);
        assert!(i >= last, "index fell at {l}");
        last = i;
    }
    assert_eq!(last, RAMP_LEN - 1);
}

#[test]
fn top_band_is_inverted() {
    for l in [0.86_f32, 0.9, 0.97, 1.0, 3.0] {
        let (bg, fg, inv) = tile_levels(l);
        assert!(inv, "{l} should be inverted");
        assert!((bg - 0.85).abs() < 0.05, "tile {bg}");
        assert!((fg - 0.12).abs() < 0.05, "glyph {fg}");
    }
    for l in [0.0_f32, 0.3, 0.6, 0.85, 0.859] {
        assert!(!tile_levels(l).2, "{l} must not be inverted");
    }
}

#[test]
fn tile_levels_follow_spec_curve() {
    for l in [0.0_f32, 0.1, 0.3, 0.449] {
        assert!(tile_levels(l).0 == 0.0, "tile at {l}");
    }
    let (bg80, _, _) = tile_levels(0.8);
    assert!((bg80 - 0.18).abs() < 0.02, "tile at 0.8 = {bg80}");
    let mut last = 0.0_f32;
    for step in 0..=80_u16 {
        let l = f32::from(step) / 100.0;
        let bg = tile_levels(l).0;
        assert!(bg >= last, "tile background not monotonic at {l}");
        last = bg;
    }
    let (_, fg_low, _) = tile_levels(0.0);
    let (_, fg_85, _) = tile_levels(0.85);
    assert!((fg_low - 0.35).abs() < 0.02, "fg low {fg_low}");
    assert!((fg_85 - 0.95).abs() < 0.02, "fg at .85 {fg_85}");
    let mut last_fg = 0.0_f32;
    for step in 0..=85_u16 {
        let fg = tile_levels(f32::from(step) / 100.0).1;
        assert!(fg >= last_fg);
        last_fg = fg;
    }
}
