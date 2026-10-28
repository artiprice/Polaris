//! The aurora engine: cell grid state, underglow blur and the RGBA blitter.
//!
//! All per-frame buffers are allocated in [`Aurora::new`] / [`Aurora::resize`];
//! [`Aurora::render_rgba`] never allocates.

use crate::atlas::GlyphAtlas;
use crate::conv::{u32_to_f32, unit_to_u8, usize_to_f32};
use crate::error::AuroraError;
use crate::field::{FieldFrame, edge_mask, star_look};
use crate::math::{or_val, smoothstep};
use crate::params::{
    GLOW_GAIN, GLOW_RADIUS, GLYPH_GAMMA, GRID_ALPHA, INVERTED_WHITEN, STAR_LUMINANCE_MAX,
    TILE_FLOOR, TILE_FLOOR_RANGE,
};
use crate::ramp::{RAMP, glyph_index, index_of, tile_levels};

/// Construction parameters of [`Aurora`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuroraConfig {
    /// Cell width in pixels.
    pub cell_width_px: u16,
    /// Cell height in pixels.
    pub cell_height_px: u16,
    /// Seed for stars, texture and tint phase.
    pub seed: u64,
    /// Maximum frame width in pixels (larger requests are clamped).
    pub max_width_px: u32,
    /// Maximum frame height in pixels (larger requests are clamped).
    pub max_height_px: u32,
}

impl Default for AuroraConfig {
    fn default() -> Self {
        Self {
            cell_width_px: 10,
            cell_height_px: 15,
            seed: 0x5EED_A0E0,
            max_width_px: 2560,
            max_height_px: 1440,
        }
    }
}

/// Pixel and cell dimensions of the current frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSize {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// Number of cell columns (`ceil(width / cell_width)`).
    pub cols: u32,
    /// Number of cell rows (`ceil(height / cell_height)`).
    pub rows: u32,
}

/// Number of internal buffers reported by [`Aurora::debug_buffer_ptrs`].
pub const DEBUG_BUFFERS: usize = 18;

/// Animated ASCII aurora renderer.
#[derive(Debug)]
pub struct Aurora {
    config: AuroraConfig,
    atlas: GlyphAtlas,
    star_glyphs: (usize, usize),
    cov_f: Vec<f32>,
    black_row: Vec<u8>,
    size: FrameSize,
    cols: usize,
    rows: usize,
    // per pixel of a cell
    px_const: Vec<[f32; 4]>,
    wy_up: Vec<f32>,
    wy_down: Vec<f32>,
    // per axis
    u_cell: Vec<f32>,
    v_cell: Vec<f32>,
    edge_x: Vec<f32>,
    edge_y: Vec<f32>,
    // per cell (structure of arrays)
    lum: Vec<f32>,
    glyph: Vec<u8>,
    tint_r: Vec<f32>,
    tint_g: Vec<f32>,
    tint_b: Vec<f32>,
    tile_r: Vec<f32>,
    tile_g: Vec<f32>,
    tile_b: Vec<f32>,
    fg_r: Vec<f32>,
    fg_g: Vec<f32>,
    fg_b: Vec<f32>,
    glow: Vec<f32>,
    scratch: Vec<f32>,
    // vertically blended glow for the pixel rows of one cell band
    glow_band: Vec<f32>,
}

fn put(v: &mut [f32], i: usize, x: f32) {
    if let Some(slot) = v.get_mut(i) {
        *slot = x;
    }
}

/// Bilinear weights of the pixels of one cell along an axis:
/// `(towards the previous cell, towards the next cell)` per pixel.
fn axis_weights(cell: usize) -> (Vec<f32>, Vec<f32>) {
    let n = usize_to_f32(cell);
    let mut before = Vec::with_capacity(cell);
    let mut after = Vec::with_capacity(cell);
    for p in 0..cell {
        let t = (usize_to_f32(p) + 0.5) / n - 0.5;
        if t < 0.0 {
            before.push(-t);
            after.push(0.0);
        } else {
            before.push(0.0);
            after.push(t);
        }
    }
    (before, after)
}

/// Build the per-cell-pixel masks: `(tile weight, grid weight)`.
///
/// The tile is a rectangle inset by one pixel with its four corner pixels
/// skipped (a cheap rounded corner); the grid is the right-hand pixel column.
fn build_masks(cw: usize, ch: usize) -> (Vec<f32>, Vec<f32>) {
    let area = cw.saturating_mul(ch);
    let mut tile = Vec::with_capacity(area);
    let mut grid = Vec::with_capacity(area);
    let x_last = cw.saturating_sub(1);
    let y_last = ch.saturating_sub(1);
    let tile_w = x_last.saturating_sub(1);
    let tile_h = y_last.saturating_sub(1);
    for y in 0..ch {
        for x in 0..cw {
            let inside = x >= 1 && x < x_last && y >= 1 && y < y_last;
            let on_x_edge = x == 1 || x == x_last.saturating_sub(1);
            let on_y_edge = y == 1 || y == y_last.saturating_sub(1);
            let corner = tile_w >= 3 && tile_h >= 3 && on_x_edge && on_y_edge;
            let is_grid = x == x_last && cw > 1;
            tile.push(if inside && !corner && !is_grid {
                1.0
            } else {
                0.0
            });
            grid.push(if is_grid { 1.0 } else { 0.0 });
        }
    }
    (tile, grid)
}

/// Glyph coverage as floats with the boldening gamma applied.
fn bake_coverage(atlas: &GlyphAtlas) -> Vec<f32> {
    let mut out = Vec::new();
    for i in 0..atlas.len() {
        if let Some(bitmap) = atlas.glyph(i) {
            out.extend(
                bitmap
                    .iter()
                    .map(|&v| (f32::from(v) * (1.0 / 255.0)).powf(GLYPH_GAMMA)),
            );
        }
    }
    out
}

/// Per-pixel constants of one cell: `[tile weight, grid weight, glow weight
/// towards the left neighbour, glow weight towards the right neighbour]`.
fn build_px_const(tile: &[f32], grid: &[f32], wx_left: &[f32], wx_right: &[f32]) -> Vec<[f32; 4]> {
    let cw = wx_left.len().max(1);
    tile.iter()
        .zip(grid)
        .enumerate()
        .map(|(i, (&t, &g))| {
            let x = or_val(i.checked_rem(cw), 0);
            [
                t,
                g,
                or_val(wx_left.get(x).copied(), 0.0),
                or_val(wx_right.get(x).copied(), 0.0),
            ]
        })
        .collect()
}

fn blur_row(src: &[f32], dst: &mut [f32]) {
    let last = src.len().saturating_sub(1);
    let inv = 1.0 / usize_to_f32(GLOW_RADIUS * 2 + 1);
    for (x, d) in dst.iter_mut().enumerate() {
        let mut acc = 0.0_f32;
        for k in 0..=(GLOW_RADIUS * 2) {
            let idx = x.saturating_add(k).saturating_sub(GLOW_RADIUS).min(last);
            if let Some(v) = src.get(idx) {
                acc += *v;
            }
        }
        *d = acc * inv;
    }
}

impl Aurora {
    /// Bake the glyph atlas and create an empty (0 x 0) renderer.
    ///
    /// # Errors
    /// [`AuroraError::Size`] for a zero cell dimension, [`AuroraError::Font`] /
    /// [`AuroraError::MissingGlyph`] when the font cannot supply the ramp.
    pub fn new(config: AuroraConfig, font: &[u8]) -> Result<Self, AuroraError> {
        let atlas = GlyphAtlas::bake(font, RAMP, config.cell_width_px, config.cell_height_px)?;
        let cw = usize::from(config.cell_width_px);
        let ch = usize::from(config.cell_height_px);
        let (wx_left, wx_right) = axis_weights(cw);
        let (wy_up, wy_down) = axis_weights(ch);
        let (tile_w, grid_w) = build_masks(cw, ch);
        let px_const = build_px_const(&tile_w, &grid_w, &wx_left, &wx_right);
        let cov_f = bake_coverage(&atlas);
        let star_glyphs = (or_val(index_of('.'), 1), or_val(index_of(':'), 1));
        Ok(Self {
            config,
            atlas,
            star_glyphs,
            black_row: [0_u8, 0, 0, 255]
                .iter()
                .copied()
                .cycle()
                .take(cw.saturating_mul(4))
                .collect(),
            size: FrameSize {
                width_px: 0,
                height_px: 0,
                cols: 0,
                rows: 0,
            },
            cols: 0,
            rows: 0,
            px_const,
            cov_f,
            wy_up,
            wy_down,
            u_cell: Vec::new(),
            v_cell: Vec::new(),
            edge_x: Vec::new(),
            edge_y: Vec::new(),
            lum: Vec::new(),
            glyph: Vec::new(),
            tint_r: Vec::new(),
            tint_g: Vec::new(),
            tint_b: Vec::new(),
            tile_r: Vec::new(),
            tile_g: Vec::new(),
            tile_b: Vec::new(),
            fg_r: Vec::new(),
            fg_g: Vec::new(),
            fg_b: Vec::new(),
            glow: Vec::new(),
            scratch: Vec::new(),
            glow_band: Vec::new(),
        })
    }

    /// Resize the frame (each axis clamped to the configured maximum).
    ///
    /// Buffers are reallocated only when the cell grid changes; resizing to the
    /// current size is a no-op.
    ///
    /// # Errors
    /// [`AuroraError::Size`] when a size computation overflows.
    pub fn resize(&mut self, width_px: u32, height_px: u32) -> Result<FrameSize, AuroraError> {
        let width = width_px.min(self.config.max_width_px);
        let height = height_px.min(self.config.max_height_px);
        let cw = u32::from(self.config.cell_width_px);
        let ch = u32::from(self.config.cell_height_px);
        let cols = ceil_div(width, cw)?;
        let rows = ceil_div(height, ch)?;
        let next = FrameSize {
            width_px: width,
            height_px: height,
            cols,
            rows,
        };
        // Reject sizes whose byte count overflows before touching any state.
        frame_bytes(next)?;
        if next == self.size {
            return Ok(self.size);
        }
        if cols != self.size.cols || rows != self.size.rows {
            self.rebuild_cells(cols, rows)?;
        }
        self.size = next;
        Ok(next)
    }

    fn rebuild_cells(&mut self, cols: u32, rows: u32) -> Result<(), AuroraError> {
        let cols_n = usize::try_from(cols).map_err(|_| AuroraError::Size)?;
        let rows_n = usize::try_from(rows).map_err(|_| AuroraError::Size)?;
        let cells = cols_n.checked_mul(rows_n).ok_or(AuroraError::Size)?;
        let band = cols_n
            .checked_mul(usize::from(self.config.cell_height_px))
            .ok_or(AuroraError::Size)?;
        let cols_f = u32_to_f32(cols);
        let rows_f = u32_to_f32(rows);

        fill(&mut self.u_cell, cols_n, 0.0);
        fill(&mut self.edge_x, cols_n, 0.0);
        for c in 0..cols_n {
            let u = (usize_to_f32(c) + 0.5) / cols_f;
            put(&mut self.u_cell, c, u);
            put(&mut self.edge_x, c, edge_mask(u, 0.5));
        }
        fill(&mut self.v_cell, rows_n, 0.0);
        fill(&mut self.edge_y, rows_n, 0.0);
        for r in 0..rows_n {
            let v = (usize_to_f32(r) + 0.5) / rows_f;
            put(&mut self.v_cell, r, v);
            put(&mut self.edge_y, r, edge_mask(0.5, v));
        }
        for v in [
            &mut self.lum,
            &mut self.tint_r,
            &mut self.tint_g,
            &mut self.tint_b,
            &mut self.tile_r,
            &mut self.tile_g,
            &mut self.tile_b,
            &mut self.fg_r,
            &mut self.fg_g,
            &mut self.fg_b,
            &mut self.glow,
            &mut self.scratch,
        ] {
            fill(v, cells, 0.0);
        }
        self.glyph.clear();
        self.glyph.resize(cells, 0);
        fill(&mut self.glow_band, band, 0.0);
        self.cols = cols_n;
        self.rows = rows_n;
        Ok(())
    }

    /// Current frame dimensions.
    #[must_use]
    pub fn frame_size(&self) -> FrameSize {
        self.size
    }

    /// Render one frame as straight, opaque RGBA8 into `out`.
    ///
    /// Every pixel is written. Does not allocate.
    ///
    /// # Errors
    /// [`AuroraError::BufferSize`] unless `out.len() == width * height * 4`.
    pub fn render_rgba(&mut self, t_seconds: f32, out: &mut [u8]) -> Result<(), AuroraError> {
        let expected = frame_bytes(self.size)?;
        if out.len() != expected {
            return Err(AuroraError::BufferSize {
                expected,
                actual: out.len(),
            });
        }
        if expected == 0 {
            return Ok(());
        }
        self.compute_cells(t_seconds);
        self.compute_glow();
        self.blit(out);
        Ok(())
    }

    /// Cold path: rows of ramp glyphs joined by `'\n'` (for snapshots).
    pub fn render_text(&mut self, t_seconds: f32) -> String {
        self.compute_cells(t_seconds);
        let mut text = String::with_capacity(self.cols.saturating_add(1).saturating_mul(self.rows));
        for (r, row) in self.glyph.chunks_exact(self.cols.max(1)).enumerate() {
            if r > 0 {
                text.push('\n');
            }
            for &g in row {
                text.push(or_val(self.atlas.glyph_char(usize::from(g)), ' '));
            }
        }
        text
    }

    /// Addresses of every internal buffer (for no-reallocation tests).
    #[doc(hidden)]
    #[must_use]
    pub fn debug_buffer_ptrs(&self) -> [usize; DEBUG_BUFFERS] {
        let f = |v: &Vec<f32>| v.as_ptr().addr();
        [
            f(&self.u_cell),
            f(&self.v_cell),
            f(&self.edge_x),
            f(&self.edge_y),
            f(&self.lum),
            self.glyph.as_ptr().addr(),
            f(&self.tint_r),
            f(&self.tint_g),
            f(&self.tint_b),
            f(&self.tile_r),
            f(&self.tile_g),
            f(&self.tile_b),
            f(&self.fg_r),
            f(&self.fg_g),
            f(&self.fg_b),
            f(&self.glow),
            f(&self.scratch),
            f(&self.glow_band),
        ]
    }

    /// Lengths of the per-cell buffers: `[lum, glyph, glow_band]`.
    #[doc(hidden)]
    #[must_use]
    pub fn debug_buffer_lens(&self) -> [usize; 3] {
        [self.lum.len(), self.glyph.len(), self.glow_band.len()]
    }

    /// Sample the field for every cell and derive glyph and paint colours.
    fn compute_cells(&mut self, t_seconds: f32) {
        let seed = self.config.seed;
        let frame = FieldFrame::new(seed, t_seconds);
        for r in 0..self.rows {
            let v = or_val(self.v_cell.get(r).copied(), 0.0);
            for c in 0..self.cols {
                let u = or_val(self.u_cell.get(c).copied(), 0.0);
                let i = r.saturating_mul(self.cols).saturating_add(c);
                let s = frame.sample(u, v);
                let mut glyph = glyph_index(s.luminance);
                let (tile_level, fg_level, inverted) = tile_levels(s.luminance);
                let mut tint = s.tint;
                let mut tile = [0.0_f32; 3];
                let mut fg = [0.0_f32; 3];
                if s.star > 0.0 && s.luminance < STAR_LUMINANCE_MAX {
                    let look = star_look(seed, u, v);
                    glyph = if look.colon {
                        self.star_glyphs.1
                    } else {
                        self.star_glyphs.0
                    };
                    tint = look.color;
                    let strength = s.star * (1.0 - s.luminance / STAR_LUMINANCE_MAX);
                    fg = [tint[0] * strength, tint[1] * strength, tint[2] * strength];
                } else if glyph > 0 {
                    let base = if inverted {
                        [
                            tint[0] + (1.0 - tint[0]) * INVERTED_WHITEN,
                            tint[1] + (1.0 - tint[1]) * INVERTED_WHITEN,
                            tint[2] + (1.0 - tint[2]) * INVERTED_WHITEN,
                        ]
                    } else {
                        tint
                    };
                    let level = if inverted {
                        tile_level
                    } else {
                        tile_level.max(
                            TILE_FLOOR
                                * smoothstep(TILE_FLOOR_RANGE[0], TILE_FLOOR_RANGE[1], s.luminance),
                        )
                    };
                    tile = [base[0] * level, base[1] * level, base[2] * level];
                    fg = [tint[0] * fg_level, tint[1] * fg_level, tint[2] * fg_level];
                }
                put(&mut self.lum, i, s.luminance);
                if let Some(slot) = self.glyph.get_mut(i) {
                    *slot = or_val(u8::try_from(glyph).ok(), 0);
                }
                put(&mut self.tint_r, i, tint[0]);
                put(&mut self.tint_g, i, tint[1]);
                put(&mut self.tint_b, i, tint[2]);
                put(&mut self.tile_r, i, tile[0]);
                put(&mut self.tile_g, i, tile[1]);
                put(&mut self.tile_b, i, tile[2]);
                put(&mut self.fg_r, i, fg[0]);
                put(&mut self.fg_g, i, fg[1]);
                put(&mut self.fg_b, i, fg[2]);
            }
        }
    }

    /// Separable box blur of the luminance, faded by the edge mask.
    fn compute_glow(&mut self) {
        let cols = self.cols;
        let rows = self.rows;
        if cols == 0 || rows == 0 {
            return;
        }
        for (src, dst) in self
            .lum
            .chunks_exact(cols)
            .zip(self.scratch.chunks_exact_mut(cols))
        {
            blur_row(src, dst);
        }
        let inv = 1.0 / usize_to_f32(GLOW_RADIUS * 2 + 1);
        let last = rows.saturating_sub(1);
        for (r, dst) in self.glow.chunks_exact_mut(cols).enumerate() {
            dst.fill(0.0);
            for k in 0..=(GLOW_RADIUS * 2) {
                let rr = r.saturating_add(k).saturating_sub(GLOW_RADIUS).min(last);
                let lo = rr.saturating_mul(cols);
                if let Some(src) = self.scratch.get(lo..lo.saturating_add(cols)) {
                    for (d, s) in dst.iter_mut().zip(src) {
                        *d += *s;
                    }
                }
            }
            let ey = or_val(self.edge_y.get(r).copied(), 0.0) * inv;
            for (d, ex) in dst.iter_mut().zip(&self.edge_x) {
                *d *= ex * ey;
            }
        }
    }

    /// Paint all pixels. Pure arithmetic over pre-allocated buffers.
    fn blit(&mut self, out: &mut [u8]) {
        let cw = self.atlas.cell_w();
        let ch = self.atlas.cell_h();
        let cols = self.cols;
        let rows = self.rows;
        let Ok(width) = usize::try_from(self.size.width_px) else {
            return;
        };
        let stride = width.saturating_mul(4);
        let cw4 = cw.saturating_mul(4);
        let band_len = stride.saturating_mul(ch);
        if stride == 0 || band_len == 0 || cols == 0 {
            return;
        }
        let last_row = rows.saturating_sub(1);
        let last_col = cols.saturating_sub(1);

        for (r, band) in out.chunks_mut(band_len).enumerate() {
            if r >= rows {
                break;
            }
            let band_rows = or_val(band.len().checked_div(stride), 0).min(ch);
            let lo = r.saturating_mul(cols);
            let Some(glow_self) = self.glow.get(lo..lo.saturating_add(cols)) else {
                continue;
            };
            let up = r.saturating_sub(1).saturating_mul(cols);
            let down = r.saturating_add(1).min(last_row).saturating_mul(cols);

            // Vertical blend of the underglow for each pixel row of this band.
            for (py, dst) in self
                .glow_band
                .chunks_exact_mut(cols)
                .enumerate()
                .take(band_rows)
            {
                let wy_u = or_val(self.wy_up.get(py).copied(), 0.0);
                let wy_d = or_val(self.wy_down.get(py).copied(), 0.0);
                let (nb_lo, wy) = if wy_u > 0.0 { (up, wy_u) } else { (down, wy_d) };
                let Some(glow_nb) = self.glow.get(nb_lo..nb_lo.saturating_add(cols)) else {
                    continue;
                };
                for ((d, a), b) in dst.iter_mut().zip(glow_self).zip(glow_nb) {
                    *d = a + (b - a) * wy;
                }
            }

            let ey = or_val(self.edge_y.get(r).copied(), 0.0);
            for c in 0..cols {
                let i = lo.saturating_add(c);
                let (
                    Some(&glyph),
                    Some(&tr),
                    Some(&tg),
                    Some(&tb),
                    Some(&bgr),
                    Some(&bgg),
                    Some(&bgb),
                    Some(&fr),
                    Some(&fgc),
                    Some(&fb),
                ) = (
                    self.glyph.get(i),
                    self.tint_r.get(i),
                    self.tint_g.get(i),
                    self.tint_b.get(i),
                    self.tile_r.get(i),
                    self.tile_g.get(i),
                    self.tile_b.get(i),
                    self.fg_r.get(i),
                    self.fg_g.get(i),
                    self.fg_b.get(i),
                )
                else {
                    continue;
                };
                let grid = GRID_ALPHA * ey * or_val(self.edge_x.get(c).copied(), 0.0);
                let grid_byte = unit_to_u8(grid);
                let has_glyph = glyph != 0;
                let c_left = c.saturating_sub(1);
                let c_right = c.saturating_add(1).min(last_col);
                let x0 = c.saturating_mul(cw4);
                let x1 = x0.saturating_add(cw4).min(stride);
                let area = cw.saturating_mul(ch);
                let g_lo = usize::from(glyph).saturating_mul(area);
                let Some(bitmap) = self.cov_f.get(g_lo..g_lo.saturating_add(area)) else {
                    continue;
                };
                let tile_rgb = [bgr, bgg, bgb];
                let fg_rgb = [fr, fgc, fb];
                let tint = [tr, tg, tb];

                let rows_iter = band
                    .chunks_exact_mut(stride)
                    .zip(self.glow_band.chunks_exact(cols))
                    .zip(self.px_const.chunks_exact(cw))
                    .zip(bitmap.chunks_exact(cw))
                    .enumerate()
                    .take(band_rows);
                for (py, (((line, gv), pc), cov)) in rows_iter {
                    let Some(seg) = line.get_mut(x0..x1) else {
                        continue;
                    };
                    let gs = or_val(gv.get(c).copied(), 0.0);
                    let gl = or_val(gv.get(c_left).copied(), gs);
                    let gr = or_val(gv.get(c_right).copied(), gs);
                    let row = RowPaint {
                        grid_byte,
                        grid,
                        grid_row: py.saturating_add(1) == ch,
                        tint,
                        glow: [gs, gl, gr],
                    };
                    if has_glyph {
                        paint_glyph_row(
                            seg,
                            &GlyphInputs {
                                pc,
                                cov,
                                tile: tile_rgb,
                                fg: fg_rgb,
                            },
                            &row,
                        );
                    } else if gs.max(gl).max(gr) < QUIET_GLOW {
                        paint_quiet_row(seg, &self.black_row, &row);
                    } else {
                        paint_glow_row(seg, pc, &row);
                    }
                }
            }
        }
    }
}

/// Glow below this contributes less than half a byte and is treated as zero.
const QUIET_GLOW: f32 = 0.008;

/// Per-row inputs shared by the pixel painters.
struct RowPaint {
    grid_byte: u8,
    grid: f32,
    grid_row: bool,
    tint: [f32; 3],
    glow: [f32; 3],
}

/// Extra inputs of a glyph row.
struct GlyphInputs<'a> {
    pc: &'a [[f32; 4]],
    cov: &'a [f32],
    tile: [f32; 3],
    fg: [f32; 3],
}

/// Black pixels (alpha 255) plus the grid line; no per-pixel arithmetic.
fn paint_quiet_row(seg: &mut [u8], black_row: &[u8], row: &RowPaint) {
    let g = row.grid_byte;
    if row.grid_row {
        for px in seg.as_chunks_mut::<4>().0 {
            *px = [g, g, g, 255];
        }
        return;
    }
    if let Some(src) = black_row.get(..seg.len()) {
        seg.copy_from_slice(src);
    }
    if seg.len() == black_row.len()
        && let Some(last) = seg.rchunks_exact_mut(4).next()
    {
        last.copy_from_slice(&[g, g, g, 255]);
    }
}

/// Underglow only (cell without glyph).
fn paint_glow_row(seg: &mut [u8], pc: &[[f32; 4]], row: &RowPaint) {
    let [gs, gl, gr] = row.glow;
    let [tr, tg, tb] = row.tint;
    let (dl, dr) = (gl - gs, gr - gs);
    let line = if row.grid_row { row.grid } else { 0.0 };
    for (px, &[_, gw, wl, wr]) in seg.as_chunks_mut::<4>().0.iter_mut().zip(pc) {
        let g = (gs + dl * wl + dr * wr) * GLOW_GAIN;
        let add = row.grid * gw + line;
        *px = [
            unit_to_u8(tr * g + add),
            unit_to_u8(tg * g + add),
            unit_to_u8(tb * g + add),
            255,
        ];
    }
}

/// Cell with a glyph: underglow, tile, glyph coverage and grid (branch free).
fn paint_glyph_row(seg: &mut [u8], gi: &GlyphInputs<'_>, row: &RowPaint) {
    // Under a tile the underglow is hidden, so a glyph cell uses one glow value
    // per row instead of the per-pixel bilinear blend.
    let [gs, _, _] = row.glow;
    let [tr, tg, tb] = row.tint;
    let g = gs * GLOW_GAIN;
    let line = if row.grid_row { row.grid } else { 0.0 };
    let it = seg.as_chunks_mut::<4>().0.iter_mut().zip(gi.pc).zip(gi.cov);
    for ((px, &[tw, gw, _, _]), &a) in it {
        let add = row.grid * gw + line;
        let br = tr * g + gi.tile[0] * tw;
        let bg = tg * g + gi.tile[1] * tw;
        let bb = tb * g + gi.tile[2] * tw;
        *px = [
            unit_to_u8(br + (gi.fg[0] - br) * a + add),
            unit_to_u8(bg + (gi.fg[1] - bg) * a + add),
            unit_to_u8(bb + (gi.fg[2] - bb) * a + add),
            255,
        ];
    }
}

/// Reset `v` to `len` copies of `value`, reusing capacity when possible.
fn fill(v: &mut Vec<f32>, len: usize, value: f32) {
    v.clear();
    v.resize(len, value);
}

/// `ceil(value / divisor)` without panicking.
fn ceil_div(value: u32, divisor: u32) -> Result<u32, AuroraError> {
    value
        .checked_add(divisor.saturating_sub(1))
        .and_then(|v| v.checked_div(divisor))
        .ok_or(AuroraError::Size)
}

/// `width * height * 4` with overflow checking.
fn frame_bytes(size: FrameSize) -> Result<usize, AuroraError> {
    let w = usize::try_from(size.width_px).map_err(|_| AuroraError::Size)?;
    let h = usize::try_from(size.height_px).map_err(|_| AuroraError::Size)?;
    w.checked_mul(h)
        .and_then(|px| px.checked_mul(4))
        .ok_or(AuroraError::Size)
}
