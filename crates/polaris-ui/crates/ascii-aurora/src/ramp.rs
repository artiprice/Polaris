//! Luminance to glyph and tile mapping.

use crate::conv::{finite_or, floor_to_usize};
use crate::math::{lerp, smoothstep};
use crate::params::{
    EMPTY_BELOW, GLYPH_FG_FULL_AT, GLYPH_FG_HIGH, GLYPH_FG_INVERTED, GLYPH_FG_LOW, INVERT_FROM,
    TILE_BG_INVERTED, TILE_BG_MAX, TILE_RISE_FROM, TILE_RISE_TO,
};

/// Glyph ramp from sparse to dense; index 0 is the blank space.
///
/// Ordered by measured ink coverage of JetBrains Mono at the default cell
/// sizes (10x15, 8x12, 7x11), so the ramp is monotonic in visual weight. The
/// requested order `o t % S X 8 @ #` was not monotonic for the real font
/// (`o` has more ink than `t`, `#` less than `%`, `S` and `X` less than `%`).
pub const RAMP: &str = " .':;toXS#%8@";

/// Number of glyphs in [`RAMP`].
pub const RAMP_LEN: usize = 13;

const _: () = assert!(RAMP.len() == RAMP_LEN, "RAMP_LEN must match RAMP");

/// Clamp any float into `0..=1` (NaN becomes 0).
fn sanitize(luminance: f32) -> f32 {
    finite_or(luminance, 0.0).clamp(0.0, 1.0)
}

/// Ramp index for a luminance. Below [`EMPTY_BELOW`] the result is 0 (space);
/// the rest of the range is split evenly over the remaining glyphs.
#[must_use]
pub fn glyph_index(luminance: f32) -> usize {
    let l = sanitize(luminance);
    if l < EMPTY_BELOW {
        return 0;
    }
    let dense = RAMP_LEN - 1;
    let x = (l - EMPTY_BELOW) / (1.0 - EMPTY_BELOW);
    let steps = dense.saturating_sub(1);
    let scaled = x * crate::conv::usize_to_f32(dense);
    floor_to_usize(scaled, steps).saturating_add(1)
}

/// Position of `c` in [`RAMP`], if present.
#[must_use]
pub fn index_of(c: char) -> Option<usize> {
    RAMP.chars().position(|r| r == c)
}

/// Tile background level, glyph foreground level and the inverted flag.
///
/// Below 0.45 the tile is transparent; it rises to about 0.18 at 0.8; from
/// 0.86 up the tile is bright and the glyph is dark.
#[must_use]
pub fn tile_levels(luminance: f32) -> (f32, f32, bool) {
    let l = sanitize(luminance);
    if l >= INVERT_FROM {
        return (TILE_BG_INVERTED, GLYPH_FG_INVERTED, true);
    }
    let bg = TILE_BG_MAX * smoothstep(TILE_RISE_FROM, TILE_RISE_TO, l);
    let fg = lerp(
        GLYPH_FG_LOW,
        GLYPH_FG_HIGH,
        (l / GLYPH_FG_FULL_AT).clamp(0.0, 1.0),
    );
    (bg, fg, false)
}
