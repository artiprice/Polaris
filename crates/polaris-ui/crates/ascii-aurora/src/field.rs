//! Pure aurora field: two animated S-curve ribbons, mesh and nebula haze,
//! curtain shimmer, twinkling stars and a soft edge fade.
//!
//! Coordinates: `u`, `v` are normalised frame coordinates in `[0, 1]`. Inside
//! the field they are mapped to the demo's space (`x = u * 1.6`, `y = v`,
//! one unit = 1000 view-box pixels).

use std::sync::LazyLock;

use crate::conv::{floor_to_u32, floor_to_usize, usize_to_f32};
use crate::math::{
    bits16, hash3, lerp, mix, or_val, ping_pong, smoothstep, unit_from_bits, value_noise,
};
use crate::params::{
    ASPECT, BAND_SIGMA, BAND_WEIGHT, BLOB_COUNT, BLOB_DRIFT, BLOB_GAIN, BLOB_ROTATION_DEG, BLOBS,
    CORE_SIGMA, CORE_WEIGHT, CROSS_FRAMES, CROSS_ORIGIN, CROSS_PERIOD, CROSS_STOPS, CURVE_POINTS,
    CURVE_SEG1, CURVE_SEG2, CURVE_SEGMENTS, EDGE_FADE_X, EDGE_FADE_Y, FLOW_FRAMES, FLOW_ORIGIN,
    FLOW_PERIOD, FLOW_STOPS, GRADIENT_EXPONENT, GRADIENT_PEAK, GRADIENT_STOPS, Keyframe,
    MESH_BLOBS, MESH_PERIOD, NEBULA_PERIOD, PALETTE, RIBBON_CUTOFF_SIGMAS, RIBBON_GAIN,
    SHIMMER_AMOUNT, SHIMMER_DRIFT, SHIMMER_FREQ_X, SHIMMER_FREQ_Y, SHIMMER_SPEED, STAR_DENSITY_16,
    STAR_LATTICE_COLS, STAR_LATTICE_ROWS, STAR_MAX, STAR_MIN, STAR_PALETTE, STAR_PERIODS,
    TEXTURE_JITTER, TINT_GAIN, TINT_LUMINANCE_WEIGHT,
};

/// One field sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Brightness `0..=1`, already multiplied by the edge mask.
    pub luminance: f32,
    /// Sage tint, linear-ish rgb in `0..=1`.
    pub tint: [f32; 3],
    /// Star twinkle opacity (`0` when the lattice cell holds no star).
    pub star: f32,
}

/// Look of a star: glyph choice and palette colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarLook {
    /// `true` for a `:` glyph, `false` for a `.` glyph.
    pub colon: bool,
    /// Star palette colour, rgb `0..=1`.
    pub color: [f32; 3],
}

const FALLBACK_BLOB: crate::params::Blob = crate::params::Blob {
    centre: [0.5, 0.5],
    radius: [1.0, 1.0],
    alpha: 0.0,
    drift: [0.0, 0.0],
};

const STAR_SALT: u64 = 0x57A2_5EED_0000_0001;
const TEXTURE_SALT: u64 = 0x7E47_0DD5_0000_0002;
const NOISE_SALT: u64 = 0x2015_E000_0000_0003;
const PHASE_SALT: u64 = 0x91A5_E000_0000_0004;

static CURVE: LazyLock<[[f32; 2]; CURVE_POINTS]> = LazyLock::new(build_curve);

/// Point on a cubic Bezier segment.
fn bezier(p: &[[f32; 2]; 4], t: f32) -> [f32; 2] {
    let [p0, p1, p2, p3] = p;
    let m = 1.0 - t;
    let w0 = m * m * m;
    let w1 = 3.0 * m * m * t;
    let w2 = 3.0 * m * t * t;
    let w3 = t * t * t;
    [
        w0 * p0[0] + w1 * p1[0] + w2 * p2[0] + w3 * p3[0],
        w0 * p0[1] + w1 * p1[1] + w2 * p2[1] + w3 * p3[1],
    ]
}

/// Pre-sample the demo S-curve into [`CURVE_POINTS`] points (unit = 1000 px).
fn build_curve() -> [[f32; 2]; CURVE_POINTS] {
    let last = usize_to_f32(CURVE_POINTS.saturating_sub(1));
    std::array::from_fn(|i| {
        let s = usize_to_f32(i) / last;
        let p = if s <= 0.5 {
            bezier(&CURVE_SEG1, s * 2.0)
        } else {
            bezier(&CURVE_SEG2, (s - 0.5) * 2.0)
        };
        [p[0] * 0.001, p[1] * 0.001]
    })
}

/// Resolution of the along-curve brightness table.
const GRADIENT_LUT_LEN: usize = 257;

static GRADIENT_LUT: LazyLock<[f32; GRADIENT_LUT_LEN]> = LazyLock::new(|| {
    let last = usize_to_f32(GRADIENT_LUT_LEN.saturating_sub(1));
    std::array::from_fn(|i| gradient_exact(usize_to_f32(i) / last))
});

/// Brightness along the ribbon (SVG gradient stops), normalised to `0..=1`
/// and flattened by [`GRADIENT_EXPONENT`].
fn gradient_exact(s: f32) -> f32 {
    for w in GRADIENT_STOPS.windows(2) {
        if let [a, b] = w
            && s <= b[0]
        {
            let span = b[0] - a[0];
            let local = if span > 0.0 { (s - a[0]) / span } else { 0.0 };
            let raw = lerp(a[1], b[1], local.clamp(0.0, 1.0)) / GRADIENT_PEAK;
            return raw.powf(GRADIENT_EXPONENT);
        }
    }
    0.0
}

/// Table lookup of [`gradient_exact`] with linear interpolation.
fn gradient(s: f32) -> f32 {
    let last = GRADIENT_LUT_LEN.saturating_sub(1);
    let pos = s.clamp(0.0, 1.0) * usize_to_f32(last);
    let i = floor_to_usize(pos, last.saturating_sub(1));
    let w = pos - usize_to_f32(i);
    let a = or_val(GRADIENT_LUT.get(i).copied(), 0.0);
    let b = or_val(GRADIENT_LUT.get(i.saturating_add(1)).copied(), 0.0);
    lerp(a, b, w)
}

const IDENTITY_FRAME: Keyframe = Keyframe {
    tx: 0.0,
    ty: 0.0,
    rot_deg: 0.0,
    scale: 1.0,
    opacity: 0.0,
};

/// Interpolate keyframes at `pos` (0..=1) with per-segment ease-in-out.
fn keyframe_at(frames: &[Keyframe], stops: &[f32], pos: f32) -> Keyframe {
    for (fw, sw) in frames.windows(2).zip(stops.windows(2)) {
        if let ([a, b], [s0, s1]) = (fw, sw)
            && pos <= *s1
        {
            let w = smoothstep(*s0, *s1, pos);
            return Keyframe {
                tx: lerp(a.tx, b.tx, w),
                ty: lerp(a.ty, b.ty, w),
                rot_deg: lerp(a.rot_deg, b.rot_deg, w),
                scale: lerp(a.scale, b.scale, w),
                opacity: lerp(a.opacity, b.opacity, w),
            };
        }
    }
    match frames.last() {
        Some(k) => *k,
        None => IDENTITY_FRAME,
    }
}

/// One line segment of a transformed ribbon, with its inverse squared length.
#[derive(Debug, Clone, Copy)]
struct Segment {
    ax: f32,
    ay: f32,
    dx: f32,
    dy: f32,
    inv_len2: f32,
}

/// Segments per culling group.
const GROUP_SIZE: usize = 8;
/// Number of culling groups covering all segments.
const GROUPS: usize = CURVE_SEGMENTS.div_ceil(GROUP_SIZE);

/// Bounding circle of a run of consecutive segments (used to skip far ribbons).
#[derive(Debug, Clone, Copy)]
struct Group {
    cx: f32,
    cy: f32,
    radius: f32,
}

/// A transformed ribbon polyline.
#[derive(Debug, Clone)]
struct Ribbon {
    points: [[f32; 2]; CURVE_POINTS],
    segments: [Segment; CURVE_SEGMENTS],
    groups: [Group; GROUPS],
    opacity: f32,
}

impl Ribbon {
    fn build(frame: &Keyframe, origin: [f32; 2]) -> Self {
        let (sin, cos) = frame.rot_deg.to_radians().sin_cos();
        let ox = origin[0] * ASPECT;
        let oy = origin[1];
        let tx = ox + frame.tx * ASPECT;
        let ty = oy + frame.ty;
        let points: [[f32; 2]; CURVE_POINTS] = std::array::from_fn(|i| {
            let p = or_val(CURVE.get(i).copied(), [0.0, 0.0]);
            let rx = (p[0] - ox) * frame.scale;
            let ry = (p[1] - oy) * frame.scale;
            [tx + cos * rx - sin * ry, ty + sin * rx + cos * ry]
        });
        let mut segments = [Segment {
            ax: 0.0,
            ay: 0.0,
            dx: 0.0,
            dy: 0.0,
            inv_len2: 0.0,
        }; CURVE_SEGMENTS];
        for (slot, pair) in segments.iter_mut().zip(points.windows(2)) {
            if let [a, b] = pair {
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let len2 = dx * dx + dy * dy;
                *slot = Segment {
                    ax: a[0],
                    ay: a[1],
                    dx,
                    dy,
                    inv_len2: if len2 > 1e-12 { 1.0 / len2 } else { 0.0 },
                };
            }
        }
        let groups: [Group; GROUPS] = std::array::from_fn(|g| {
            let first = g.saturating_mul(GROUP_SIZE);
            let end = first
                .saturating_add(GROUP_SIZE)
                .saturating_add(1)
                .min(CURVE_POINTS);
            let pts: &[[f32; 2]] = or_val(points.get(first..end), &[]);
            let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for p in pts {
                x0 = x0.min(p[0]);
                x1 = x1.max(p[0]);
                y0 = y0.min(p[1]);
                y1 = y1.max(p[1]);
            }
            let cx = 0.5 * (x0 + x1);
            let cy = 0.5 * (y0 + y1);
            let radius = pts
                .iter()
                .map(|p| ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt())
                .fold(0.0_f32, f32::max);
            Group { cx, cy, radius }
        });
        Self {
            points,
            segments,
            groups,
            opacity: frame.opacity,
        }
    }

    /// Squared distance to the polyline and the curve parameter `s` of the
    /// nearest point, or `None` when the polyline is farther than `max_dist`.
    ///
    /// Whole groups of segments are skipped by their bounding circle.
    fn nearest(&self, x: f32, y: f32, max_dist: f32) -> Option<(f32, f32)> {
        let mut best = max_dist * max_dist;
        let mut found = false;
        let mut best_i = 0_usize;
        let mut best_h = 0.0_f32;
        for (g, (group, segs)) in self
            .groups
            .iter()
            .zip(self.segments.chunks(GROUP_SIZE))
            .enumerate()
        {
            let dc = ((x - group.cx).powi(2) + (y - group.cy).powi(2)).sqrt();
            let lower = (dc - group.radius).max(0.0);
            if lower * lower >= best {
                continue;
            }
            for (k, sg) in segs.iter().enumerate() {
                let px = x - sg.ax;
                let py = y - sg.ay;
                let h = ((px * sg.dx + py * sg.dy) * sg.inv_len2).clamp(0.0, 1.0);
                let qx = px - h * sg.dx;
                let qy = py - h * sg.dy;
                let d2 = qx * qx + qy * qy;
                if d2 < best {
                    best = d2;
                    found = true;
                    best_i = g.saturating_mul(GROUP_SIZE).saturating_add(k);
                    best_h = h;
                }
            }
        }
        if !found {
            return None;
        }
        let s = (usize_to_f32(best_i) + best_h) / usize_to_f32(CURVE_SEGMENTS);
        Some((best, s))
    }

    /// Ribbon energy at a point (band plus core, along-curve gradient).
    fn energy(&self, x: f32, y: f32) -> f32 {
        let cutoff = RIBBON_CUTOFF_SIGMAS * BAND_SIGMA;
        let Some((d2, s)) = self.nearest(x, y, cutoff) else {
            return 0.0;
        };
        let band = (-d2 / (2.0 * BAND_SIGMA * BAND_SIGMA)).exp();
        let core_cut = RIBBON_CUTOFF_SIGMAS * CORE_SIGMA;
        let core = if d2 < core_cut * core_cut {
            (-d2 / (2.0 * CORE_SIGMA * CORE_SIGMA)).exp()
        } else {
            0.0
        };
        self.opacity * gradient(s) * (BAND_WEIGHT * band + CORE_WEIGHT * core)
    }
}

/// An animated blob.
#[derive(Debug, Clone, Copy)]
struct BlobState {
    cu: f32,
    cv: f32,
    cos: f32,
    sin: f32,
    inv_ru: f32,
    inv_rv: f32,
    weight: f32,
}

/// Everything that depends on `(seed, t)` but not on the sampled position.
///
/// Build it once per frame with [`FieldFrame::new`], then call
/// [`FieldFrame::sample`] for every cell. It lives on the stack (no heap).
#[derive(Debug, Clone)]
pub struct FieldFrame {
    seed: u64,
    t: f32,
    ribbons: [Ribbon; 2],
    blobs: [BlobState; BLOB_COUNT],
    phase: [f32; 2],
}

impl FieldFrame {
    /// Prepare the frame at simulation time `t` (non-finite `t` is treated as 0).
    #[must_use]
    pub fn new(seed: u64, t: f32) -> Self {
        let t = if t.is_finite() { t } else { 0.0 };
        let flow = keyframe_at(&FLOW_FRAMES, &FLOW_STOPS, ping_pong(t, FLOW_PERIOD));
        let cross = keyframe_at(&CROSS_FRAMES, &CROSS_STOPS, ping_pong(t, CROSS_PERIOD));
        let mesh_m = group_offset(t, MESH_PERIOD);
        let nebula_m = group_offset(t, NEBULA_PERIOD);
        let blobs: [BlobState; BLOB_COUNT] = std::array::from_fn(|i| {
            let blob = match BLOBS.get(i) {
                Some(b) => *b,
                None => FALLBACK_BLOB,
            };
            let m = if i < MESH_BLOBS { mesh_m } else { nebula_m };
            let (sin, cos) = (BLOB_ROTATION_DEG * m).to_radians().sin_cos();
            BlobState {
                cu: blob.centre[0] + blob.drift[0] * BLOB_DRIFT * m,
                cv: blob.centre[1] + blob.drift[1] * BLOB_DRIFT * m,
                cos,
                sin,
                inv_ru: 1.0 / blob.radius[0],
                inv_rv: 1.0 / blob.radius[1],
                weight: blob.alpha * BLOB_GAIN,
            }
        });
        Self {
            seed,
            t,
            ribbons: [
                Ribbon::build(&flow, FLOW_ORIGIN),
                Ribbon::build(&cross, CROSS_ORIGIN),
            ],
            blobs,
            phase: [
                unit_from_bits(mix(seed ^ PHASE_SALT), 0) * core::f32::consts::TAU,
                unit_from_bits(mix(seed ^ PHASE_SALT), 16) * core::f32::consts::TAU,
            ],
        }
    }

    /// The transformed ribbon polyline in `(u, v)` coordinates.
    ///
    /// `ribbon` 0 is `aurora-flow`, 1 is `aurora-cross`; anything else yields
    /// the cross ribbon.
    #[must_use]
    pub fn ribbon_polyline(&self, ribbon: usize) -> [[f32; 2]; CURVE_POINTS] {
        let [flow, cross] = &self.ribbons;
        let r = if ribbon == 0 { flow } else { cross };
        std::array::from_fn(|i| {
            let p = or_val(r.points.get(i).copied(), [0.0, 0.0]);
            [p[0] / ASPECT, p[1]]
        })
    }

    /// Sample the field at normalised coordinates.
    #[must_use]
    pub fn sample(&self, u: f32, v: f32) -> Sample {
        let mask = edge_mask(u, v);
        let x = u * ASPECT;
        let y = v;
        if mask <= 0.0 {
            return Sample {
                luminance: 0.0,
                tint: palette_tint(0.0),
                star: 0.0,
            };
        }
        let ribbons = self.ribbons.iter().map(|r| r.energy(x, y)).sum::<f32>();
        let shimmer = if ribbons > 1e-3 {
            1.0 + SHIMMER_AMOUNT * (2.0 * self.curtain(x, y) - 1.0)
        } else {
            1.0
        };
        let haze: f32 = self
            .blobs
            .iter()
            .map(|b| {
                let du = u - b.cu;
                let dv = v - b.cv;
                let ru = (b.cos * du + b.sin * dv) * b.inv_ru;
                let rv = (b.cos * dv - b.sin * du) * b.inv_rv;
                let q = ru * ru + rv * rv;
                if q < 1.0 {
                    let w = 1.0 - q;
                    b.weight * w * w
                } else {
                    0.0
                }
            })
            .sum();
        // Contrast curve x^1.5 (crisper ribbon edge, thinner haze).
        let base = (RIBBON_GAIN * ribbons * shimmer + haze).clamp(0.0, 1.0);
        let base = base * base.sqrt();
        let (col, row) = star_cell(u, v);
        let h = star_hash(self.seed, col, row);
        let jitter = unit_from_bits(mix(h ^ TEXTURE_SALT), 0);
        let luminance =
            (base * (1.0 + TEXTURE_JITTER * (2.0 * jitter - 1.0))).clamp(0.0, 1.0) * mask;
        Sample {
            luminance,
            tint: self.tint(x, y, luminance),
            star: star_opacity(h, self.t) * mask,
        }
    }

    /// Vertical curtain streaks: two-octave FBM value noise stretched along `y`.
    fn curtain(&self, x: f32, y: f32) -> f32 {
        let mut acc = 0.0_f32;
        let mut amp = 0.5_f32;
        let mut freq = 1.0_f32;
        let mut norm = 0.0_f32;
        for octave in 0_u64..2 {
            let n = value_noise(
                self.seed ^ NOISE_SALT ^ mix(octave),
                x * SHIMMER_FREQ_X * freq,
                (y * SHIMMER_FREQ_Y + self.t * SHIMMER_DRIFT) * freq,
                self.t * SHIMMER_SPEED * freq,
            );
            acc += amp * n;
            norm += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        acc / norm
    }

    /// Slow sage tint field, pushed towards the light end where it is bright.
    fn tint(&self, x: f32, y: f32, luminance: f32) -> [f32; 3] {
        let p1 = 0.5 + 0.5 * (1.3 * x + 0.9 * y + 0.045 * self.t + self.phase[0]).sin();
        let p2 = 0.5 + 0.5 * (2.3 * y - 0.8 * x - 0.03 * self.t + self.phase[1]).sin();
        let slow = 0.5 * (p1 + p2);
        let q = (1.0 - TINT_LUMINANCE_WEIGHT) * slow + TINT_LUMINANCE_WEIGHT * luminance;
        palette_tint(q)
    }
}

/// Eased ping-pong offset in `-1..=1` for a blob group.
fn group_offset(t: f32, period: f32) -> f32 {
    2.0 * smoothstep(0.0, 1.0, ping_pong(t, period)) - 1.0
}

fn byte_rgb(c: [u8; 3]) -> [f32; 3] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
    ]
}

/// Palette colour at `q` in `0..=1`, times the tint gain, clamped to 1.
fn palette_tint(q: f32) -> [f32; 3] {
    let last = PALETTE.len().saturating_sub(1);
    let pos = q.clamp(0.0, 1.0) * usize_to_f32(last);
    let i = floor_to_usize(pos, last.saturating_sub(1));
    let w = (pos - usize_to_f32(i)).clamp(0.0, 1.0);
    let a = byte_rgb(or_val(PALETTE.get(i).copied(), [0, 0, 0]));
    let b = byte_rgb(or_val(PALETTE.get(i.saturating_add(1)).copied(), [0, 0, 0]));
    [
        (lerp(a[0], b[0], w) * TINT_GAIN).min(1.0),
        (lerp(a[1], b[1], w) * TINT_GAIN).min(1.0),
        (lerp(a[2], b[2], w) * TINT_GAIN).min(1.0),
    ]
}

/// Smooth fade to 0 at the frame edges (12 % left/right, 14 % top/bottom).
#[must_use]
pub fn edge_mask(u: f32, v: f32) -> f32 {
    smoothstep(0.0, EDGE_FADE_X, u)
        * smoothstep(0.0, EDGE_FADE_X, 1.0 - u)
        * smoothstep(0.0, EDGE_FADE_Y, v)
        * smoothstep(0.0, EDGE_FADE_Y, 1.0 - v)
}

/// Star lattice cell of a normalised coordinate.
#[must_use]
pub fn star_cell(u: f32, v: f32) -> (u32, u32) {
    let col = floor_to_u32(
        u * crate::conv::u32_to_f32(STAR_LATTICE_COLS),
        STAR_LATTICE_COLS.saturating_sub(1),
    );
    let row = floor_to_u32(
        v * crate::conv::u32_to_f32(STAR_LATTICE_ROWS),
        STAR_LATTICE_ROWS.saturating_sub(1),
    );
    (col, row)
}

/// Seeded hash of a star lattice cell.
fn star_hash(seed: u64, col: u32, row: u32) -> u64 {
    hash3(seed ^ STAR_SALT, col.cast_signed(), row.cast_signed(), 0)
}

/// Twinkle opacity of the star in a lattice cell (`0` when there is none).
fn star_opacity(h: u64, t: f32) -> f32 {
    if bits16(h, 0) >= STAR_DENSITY_16 {
        return 0.0;
    }
    let slot = usize::from((bits16(h, 16) >> 8) & 3).min(STAR_PERIODS.len().saturating_sub(1));
    let period = or_val(STAR_PERIODS.get(slot).copied(), 13.0);
    let phase = unit_from_bits(h, 32);
    let tw = 0.5 + 0.5 * (core::f32::consts::TAU * (t / period + phase)).sin();
    lerp(STAR_MIN, STAR_MAX, tw)
}

/// Glyph and palette colour of the star in the lattice cell of `(u, v)`.
#[must_use]
pub fn star_look(seed: u64, u: f32, v: f32) -> StarLook {
    let (col, row) = star_cell(u, v);
    let h = mix(star_hash(seed, col, row) ^ STAR_SALT);
    let idx = usize::from(bits16(h, 0) & 3);
    let color = byte_rgb(or_val(STAR_PALETTE.get(idx).copied(), [255, 255, 255]));
    StarLook {
        colon: bits16(h, 16) & 1 == 1,
        color,
    }
}

/// Convenience pure sampler: builds a [`FieldFrame`] and samples once.
///
/// For many samples at the same time prefer [`FieldFrame::new`] +
/// [`FieldFrame::sample`].
#[must_use]
pub fn sample(seed: u64, t: f32, u: f32, v: f32) -> Sample {
    FieldFrame::new(seed, t).sample(u, v)
}
