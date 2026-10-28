//! Glyph atlas: one cell-sized coverage bitmap per ramp glyph, baked once.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};

use crate::conv::{floor_to_i32, unit_to_u8};
use crate::error::AuroraError;
use crate::params::GLYPH_HEIGHT;

/// Rasterised ramp glyphs, each exactly `cell_w * cell_h` bytes of coverage.
#[derive(Debug, Clone)]
pub struct GlyphAtlas {
    cell_w: usize,
    cell_h: usize,
    chars: Vec<char>,
    coverage: Vec<u8>,
    ink: Vec<u32>,
}

impl GlyphAtlas {
    /// Rasterise every character of `ramp` into a `cell_w x cell_h` bitmap.
    ///
    /// The glyph is centred horizontally by its advance and vertically so the
    /// cap height (about `0.62 * cell_h`) is centred in the cell.
    ///
    /// # Errors
    /// [`AuroraError::Size`] for a zero cell, [`AuroraError::Font`] when the
    /// bytes are not a usable font and [`AuroraError::MissingGlyph`] when a ramp
    /// character has no glyph.
    pub fn bake(font: &[u8], ramp: &str, cell_w: u16, cell_h: u16) -> Result<Self, AuroraError> {
        if cell_w == 0 || cell_h == 0 {
            return Err(AuroraError::Size);
        }
        let font = FontRef::try_from_slice(font).map_err(|_| AuroraError::Font)?;
        let w = usize::from(cell_w);
        let h = usize::from(cell_h);
        let cell_area = w.checked_mul(h).ok_or(AuroraError::Size)?;
        let cell_h_px = f32::from(cell_h);
        let scale = cap_scale(&font, cell_h_px)?;
        let scaled = font.as_scaled(scale);
        let cap_px = GLYPH_HEIGHT * cell_h_px;
        let baseline = (cell_h_px + cap_px) * 0.5;

        let mut chars = Vec::new();
        let mut coverage = Vec::new();
        let mut ink = Vec::new();
        for c in ramp.chars() {
            let id = font.glyph_id(c);
            if id.0 == 0 {
                return Err(AuroraError::MissingGlyph(c));
            }
            let pen_x = (f32::from(cell_w) - scaled.h_advance(id)) * 0.5;
            let glyph = id.with_scale_and_position(scale, point(pen_x, baseline));
            let start = coverage.len();
            coverage.resize(start.saturating_add(cell_area), 0_u8);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let ox = floor_to_i32(bounds.min.x);
                let oy = floor_to_i32(bounds.min.y);
                let bitmap = coverage.get_mut(start..start.saturating_add(cell_area));
                if let Some(bitmap) = bitmap {
                    outlined.draw(|gx, gy, c| {
                        let x = i64::from(ox) + i64::from(gx);
                        let y = i64::from(oy) + i64::from(gy);
                        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
                            return;
                        };
                        if x >= w || y >= h {
                            return;
                        }
                        let idx = y.saturating_mul(w).saturating_add(x);
                        if let Some(px) = bitmap.get_mut(idx) {
                            *px = unit_to_u8(c).max(*px);
                        }
                    });
                }
            }
            let total = coverage
                .get(start..)
                .map_or(0_u32, |g| g.iter().map(|&v| u32::from(v)).sum());
            ink.push(total);
            chars.push(c);
        }
        Ok(Self {
            cell_w: w,
            cell_h: h,
            chars,
            coverage,
            ink,
        })
    }

    /// Number of baked glyphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether the atlas holds no glyphs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Cell width in pixels.
    #[must_use]
    pub fn cell_w(&self) -> usize {
        self.cell_w
    }

    /// Cell height in pixels.
    #[must_use]
    pub fn cell_h(&self) -> usize {
        self.cell_h
    }

    /// Character of glyph `index`.
    #[must_use]
    pub fn glyph_char(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    /// Coverage bitmap (`cell_w * cell_h` bytes, row major) of glyph `index`.
    #[must_use]
    pub fn glyph(&self, index: usize) -> Option<&[u8]> {
        let area = self.cell_w.checked_mul(self.cell_h)?;
        let start = index.checked_mul(area)?;
        self.coverage.get(start..start.checked_add(area)?)
    }

    /// One bitmap row (`cell_w` bytes) of glyph `index`.
    #[must_use]
    pub fn row(&self, index: usize, y: usize) -> Option<&[u8]> {
        let glyph = self.glyph(index)?;
        let start = y.checked_mul(self.cell_w)?;
        glyph.get(start..start.checked_add(self.cell_w)?)
    }

    /// Sum of all coverage bytes of glyph `index` (its "ink").
    #[must_use]
    pub fn ink(&self, index: usize) -> Option<u32> {
        self.ink.get(index).copied()
    }
}

/// Pick the pixel scale so that the cap height is `GLYPH_HEIGHT * cell_h`.
fn cap_scale(font: &FontRef<'_>, cell_h: f32) -> Result<PxScale, AuroraError> {
    const PROBE: f32 = 100.0;
    let probe = font.glyph_id('X').with_scale(PxScale::from(PROBE));
    let bounds = font
        .outline_glyph(probe)
        .map(|g| g.px_bounds())
        .ok_or(AuroraError::Font)?;
    let ratio = bounds.height() / PROBE;
    if !(ratio.is_finite() && ratio > 0.0) {
        return Err(AuroraError::Font);
    }
    Ok(PxScale::from(GLYPH_HEIGHT * cell_h / ratio))
}
