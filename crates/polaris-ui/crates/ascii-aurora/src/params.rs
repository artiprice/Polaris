//! Tunable constants for the aurora look.
//!
//! The geometry and timing values are ported from the web demo's northern
//! lights (1600x1000 view box); the gains are tuned against the reference
//! ASCII frames.

/// Horizontal extent of the unit square in the demo's coordinate system
/// (view box 1600 x 1000, one unit = 1000 view-box pixels).
pub const ASPECT: f32 = 1.6;

/// Number of pre-sampled points on the aurora S-curve.
pub const CURVE_POINTS: usize = 64;
/// Number of line segments between the curve points.
pub const CURVE_SEGMENTS: usize = CURVE_POINTS - 1;

/// Demo SVG path first cubic: `M-280 880 C100 1100 270 240 660 530`.
pub const CURVE_SEG1: [[f32; 2]; 4] = [
    [-280.0, 880.0],
    [100.0, 1100.0],
    [270.0, 240.0],
    [660.0, 530.0],
];
/// Demo SVG path second cubic: `S1240 1070 1880 170` (first control point is
/// the reflection of `(270, 240)` about `(660, 530)`).
pub const CURVE_SEG2: [[f32; 2]; 4] = [
    [660.0, 530.0],
    [1050.0, 820.0],
    [1240.0, 1070.0],
    [1880.0, 170.0],
];

/// One animation keyframe of a ribbon or blob group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    /// Horizontal translation as a fraction of the frame width.
    pub tx: f32,
    /// Vertical translation as a fraction of the frame height.
    pub ty: f32,
    /// Rotation in degrees.
    pub rot_deg: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Opacity multiplier.
    pub opacity: f32,
}

/// Ribbon 1 (`aurora-flow`) keyframes at 0 %, 55 % and 100 %.
pub const FLOW_FRAMES: [Keyframe; 3] = [
    Keyframe {
        tx: -0.04,
        ty: 0.06,
        rot_deg: -12.0,
        scale: 1.06,
        opacity: 0.45,
    },
    Keyframe {
        tx: 0.02,
        ty: -0.02,
        rot_deg: -2.0,
        scale: 1.15,
        opacity: 0.72,
    },
    Keyframe {
        tx: 0.05,
        ty: 0.01,
        rot_deg: 8.0,
        scale: 1.04,
        opacity: 0.55,
    },
];
/// Keyframe stops of ribbon 1.
pub const FLOW_STOPS: [f32; 3] = [0.0, 0.55, 1.0];
/// One-way duration of ribbon 1 in seconds (alternate).
pub const FLOW_PERIOD: f32 = 31.0;
/// Transform origin of ribbon 1 in unit coordinates.
pub const FLOW_ORIGIN: [f32; 2] = [0.5, 0.5];

/// Ribbon 2 (`aurora-cross`) keyframes at 0 % and 100 %.
pub const CROSS_FRAMES: [Keyframe; 2] = [
    Keyframe {
        tx: 0.04,
        ty: -0.08,
        rot_deg: 156.0,
        scale: 1.12,
        opacity: 0.28,
    },
    Keyframe {
        tx: -0.03,
        ty: 0.05,
        rot_deg: 176.0,
        scale: 1.03,
        opacity: 0.5,
    },
];
/// Keyframe stops of ribbon 2.
pub const CROSS_STOPS: [f32; 2] = [0.0, 1.0];
/// One-way duration of ribbon 2 in seconds (alternate).
pub const CROSS_PERIOD: f32 = 48.0;
/// Transform origin of ribbon 2 in unit coordinates.
pub const CROSS_ORIGIN: [f32; 2] = [0.65, 0.6];

/// Gaussian sigma of the broad ribbon band (unit = 1000 view-box pixels).
pub const BAND_SIGMA: f32 = 0.05;
/// Gaussian sigma of the thin bright core.
pub const CORE_SIGMA: f32 = 0.011;
/// Weight of the broad band.
pub const BAND_WEIGHT: f32 = 0.78;
/// Weight of the bright core.
pub const CORE_WEIGHT: f32 = 0.45;
/// Brightness along the ribbon by curve parameter (SVG gradient stops).
pub const GRADIENT_STOPS: [[f32; 2]; 5] = [
    [0.0, 0.0],
    [0.22, 0.28],
    [0.48, 0.55],
    [0.72, 0.30],
    [1.0, 0.0],
];
/// Peak of [`GRADIENT_STOPS`], used to normalise the gradient to 0..=1.
pub const GRADIENT_PEAK: f32 = 0.55;
/// Final gain applied to the ribbon energy.
pub const RIBBON_GAIN: f32 = 1.15;
/// Exponent applied to the normalised along-curve gradient (below 1 flattens it
/// so more of the visible ribbon stays bright).
pub const GRADIENT_EXPONENT: f32 = 0.6;
/// Ribbons farther than this many sigmas contribute nothing.
pub const RIBBON_CUTOFF_SIGMAS: f32 = 4.0;

/// Curtain shimmer: horizontal noise frequency (streaks are narrow).
pub const SHIMMER_FREQ_X: f32 = 11.0;
/// Curtain shimmer: vertical noise frequency (streaks are tall).
pub const SHIMMER_FREQ_Y: f32 = 1.7;
/// Curtain shimmer: vertical drift speed (noise units per second).
pub const SHIMMER_DRIFT: f32 = 0.05;
/// Curtain shimmer: evolution speed along the noise time axis.
pub const SHIMMER_SPEED: f32 = 0.09;
/// Curtain shimmer: modulation amplitude (+-25 %).
pub const SHIMMER_AMOUNT: f32 = 0.25;

/// A mesh or nebula blob: elliptical falloff in unit coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blob {
    /// Centre (u, v).
    pub centre: [f32; 2],
    /// Radii (u, v).
    pub radius: [f32; 2],
    /// Peak alpha in the demo.
    pub alpha: f32,
    /// Drift direction multiplier (u, v) applied to the group offset.
    pub drift: [f32; 2],
}

/// Number of blobs (3 mesh + 4 nebula).
pub const BLOB_COUNT: usize = 7;
/// Blobs from the demo's mesh gradient and nebula layers.
pub const BLOBS: [Blob; BLOB_COUNT] = [
    Blob {
        centre: [0.25, 0.62],
        radius: [0.55, 0.34],
        alpha: 0.38,
        drift: [1.0, -0.6],
    },
    Blob {
        centre: [0.77, 0.38],
        radius: [0.45, 0.50],
        alpha: 0.31,
        drift: [-0.8, 0.7],
    },
    Blob {
        centre: [0.63, 0.78],
        radius: [0.48, 0.28],
        alpha: 0.21,
        drift: [0.5, 1.0],
    },
    Blob {
        centre: [0.32, 0.55],
        radius: [0.30, 0.18],
        alpha: 0.21,
        drift: [-1.0, 0.5],
    },
    Blob {
        centre: [0.65, 0.63],
        radius: [0.28, 0.27],
        alpha: 0.16,
        drift: [0.9, 0.8],
    },
    Blob {
        centre: [0.78, 0.39],
        radius: [0.24, 0.13],
        alpha: 0.17,
        drift: [-0.6, -1.0],
    },
    Blob {
        centre: [0.42, 0.83],
        radius: [0.34, 0.20],
        alpha: 0.25,
        drift: [1.0, 0.4],
    },
];
/// How many of the leading [`BLOBS`] belong to the mesh group.
pub const MESH_BLOBS: usize = 3;
/// Mesh group one-way period in seconds.
pub const MESH_PERIOD: f32 = 44.0;
/// Nebula group one-way period in seconds.
pub const NEBULA_PERIOD: f32 = 37.0;
/// Maximum blob drift as a fraction of the frame.
pub const BLOB_DRIFT: f32 = 0.04;
/// Maximum blob rotation in degrees.
pub const BLOB_ROTATION_DEG: f32 = 4.0;
/// Gain applied to the summed blob alpha.
pub const BLOB_GAIN: f32 = 0.40;

/// Static per-cell texture jitter (fraction of luminance, +-).
pub const TEXTURE_JITTER: f32 = 0.20;

/// Edge fade width at the top and bottom (fraction of height).
pub const EDGE_FADE_Y: f32 = 0.14;
/// Edge fade width at the left and right (fraction of width).
pub const EDGE_FADE_X: f32 = 0.12;

/// Virtual star lattice columns (normalised, resolution independent).
pub const STAR_LATTICE_COLS: u32 = 256;
/// Virtual star lattice rows.
pub const STAR_LATTICE_ROWS: u32 = 96;
/// Fraction of lattice cells that hold a star, in 1/65536 units (about 0.6 %).
pub const STAR_DENSITY_16: u16 = 393;
/// Twinkle periods in seconds.
pub const STAR_PERIODS: [f32; 3] = [13.0, 19.0, 23.0];
/// Minimum twinkle opacity.
pub const STAR_MIN: f32 = 0.38;
/// Maximum twinkle opacity.
pub const STAR_MAX: f32 = 0.88;
/// Stars only show where the ribbon luminance is below this.
pub const STAR_LUMINANCE_MAX: f32 = 0.10;
/// Star palette (#a7b19e #b5c1ad #819b70 #d0d3cb).
pub const STAR_PALETTE: [[u8; 3]; 4] = [
    [0xa7, 0xb1, 0x9e],
    [0xb5, 0xc1, 0xad],
    [0x81, 0x9b, 0x70],
    [0xd0, 0xd3, 0xcb],
];

/// Sage aurora palette (#415543 #53795a #648e65 #789e4b #a2b990).
pub const PALETTE: [[u8; 3]; 5] = [
    [0x41, 0x55, 0x43],
    [0x53, 0x79, 0x5a],
    [0x64, 0x8e, 0x65],
    [0x78, 0x9e, 0x4b],
    [0xa2, 0xb9, 0x90],
];
/// Brightness gain applied to palette colours to get the glyph tint.
pub const TINT_GAIN: f32 = 1.7;
/// How strongly luminance pushes the tint towards the light palette end.
pub const TINT_LUMINANCE_WEIGHT: f32 = 0.55;

/// Luminance below which a cell is a blank space.
pub const EMPTY_BELOW: f32 = 0.07;
/// Luminance at and above which the tile is bright and the glyph inverted.
pub const INVERT_FROM: f32 = 0.86;
/// Luminance where the tile background starts to rise.
pub const TILE_RISE_FROM: f32 = 0.45;
/// Luminance where the tile background reaches [`TILE_BG_MAX`].
pub const TILE_RISE_TO: f32 = 0.80;
/// Tile background level at the end of the rise.
pub const TILE_BG_MAX: f32 = 0.18;
/// Tile background level of an inverted (bright) tile.
pub const TILE_BG_INVERTED: f32 = 0.85;
/// Glyph foreground level at low luminance.
pub const GLYPH_FG_LOW: f32 = 0.35;
/// Glyph foreground level at luminance [`GLYPH_FG_FULL_AT`].
pub const GLYPH_FG_HIGH: f32 = 0.95;
/// Luminance at which the foreground reaches [`GLYPH_FG_HIGH`].
pub const GLYPH_FG_FULL_AT: f32 = 0.85;
/// Glyph foreground level (fraction of tint) on an inverted tile.
pub const GLYPH_FG_INVERTED: f32 = 0.12;
/// How far the bright tile colour is mixed towards white.
pub const INVERTED_WHITEN: f32 = 0.45;

/// Glyph height as a fraction of the cell height.
pub const GLYPH_HEIGHT: f32 = 0.62;

/// Underglow blur radius in cells.
pub const GLOW_RADIUS: usize = 3;
/// Underglow strength (`tint * glow * GLOW_GAIN`).
pub const GLOW_GAIN: f32 = 0.22;
/// Faint grid line alpha (white over the background).
pub const GRID_ALPHA: f32 = 0.03;

/// Coverage gamma applied when blitting glyphs (below 1 makes strokes bolder).
pub const GLYPH_GAMMA: f32 = 0.55;
/// Minimum tile background level of any glyph cell (dark slate tile as in the
/// reference, where even dim cells sit on a faint tile).
pub const TILE_FLOOR: f32 = 0.10;

/// Luminance range over which the tile floor fades in.
pub const TILE_FLOOR_RANGE: [f32; 2] = [0.12, 0.35];
