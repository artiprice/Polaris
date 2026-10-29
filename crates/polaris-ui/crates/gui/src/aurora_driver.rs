//! Feeds ASCII aurora frames into the welcome window.
//!
//! One repeating `slint::Timer` (20 fps) calls [`AuroraDriver::tick`]. Each tick renders
//! into one of two pre-allocated pixel buffers, alternating, so the frame Slint still shows is
//! never written to and the buffer being filled is normally uniquely owned (no copy). Nothing is
//! rendered while the window is minimised or hidden, and reduced motion draws one still frame.

use core::time::Duration;
use std::time::Instant;

use polaris_ui_ascii_aurora::{Aurora, AuroraConfig, FrameClock, FrameSize};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::{AppWindow, GuiError};

/// Simulation and repaint step of the aurora: 20 fps. The ribbons drift about 5 px/s across
/// 10 px cells, so a glyph changes every second or two; 30 fps cost ~28% of a core (measured
/// 2026-10-28) for no visible gain. See @prime/evidence/ui/desktop/desktop-verification.txt.
pub(crate) const FRAME_STEP: Duration = Duration::from_millis(50);

/// Logical cell size of the aurora grid, measured from the reference video (10 x 15 px).
const CELL_LOGICAL: (f32, f32) = (10.0, 15.0);

/// Upper bounds (inclusive, milliseconds) of the frame-time histogram buckets; the last bucket
/// takes everything slower.
pub const HISTOGRAM_BUCKETS_MS: [u64; 8] = [1, 2, 4, 8, 16, 33, 66, u64::MAX];

/// Render-time histogram of aurora frames (time spent in `render_rgba` plus the hand-off to
/// Slint; GPU upload and presentation are not included).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameHistogram {
    /// Frames per bucket of [`HISTOGRAM_BUCKETS_MS`].
    pub counts: [u64; 8],
    /// Frames recorded.
    pub frames: u64,
    /// Sum of all frame times in microseconds.
    pub total_micros: u64,
    /// Slowest frame in microseconds.
    pub max_micros: u64,
}

impl FrameHistogram {
    /// Adds one frame time.
    pub fn record(&mut self, took: Duration) {
        let micros = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
        let millis = match micros.checked_div(1000) {
            Some(millis) => millis,
            None => 0,
        };
        let bucket = HISTOGRAM_BUCKETS_MS
            .iter()
            .position(|&limit| millis < limit || limit == u64::MAX);
        if let Some(slot) = bucket.and_then(|index| self.counts.get_mut(index)) {
            *slot = slot.saturating_add(1);
        }
        self.frames = self.frames.saturating_add(1);
        self.total_micros = self.total_micros.saturating_add(micros);
        self.max_micros = self.max_micros.max(micros);
    }

    /// Mean frame time in microseconds, or `None` before the first frame.
    #[must_use]
    pub fn mean_micros(&self) -> Option<u64> {
        self.total_micros.checked_div(self.frames)
    }

    /// One-line summary for logs and evidence files.
    #[must_use]
    pub fn summary(&self) -> String {
        let mean = match self.mean_micros() {
            Some(mean) => mean.to_string(),
            None => String::from("-"),
        };
        let mut buckets = String::new();
        let mut lower = 0_u64;
        for (limit, count) in HISTOGRAM_BUCKETS_MS.iter().zip(self.counts.iter()) {
            let label = if *limit == u64::MAX {
                format!(">={lower}ms")
            } else {
                format!("{lower}-{limit}ms")
            };
            buckets.push_str(&format!(" {label}:{count}"));
            lower = *limit;
        }
        format!(
            "frames={} mean_us={mean} max_us={}{buckets}",
            self.frames, self.max_micros
        )
    }
}

/// Converts a logical length to whole physical pixels (rounded, clamped to `0..=u32::MAX`).
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is finite, rounded and clamped to the u32 range first"
)]
fn to_px(logical: f32, scale: f32) -> u32 {
    let physical = (logical * scale).round();
    if physical.is_finite() && physical > 0.0 {
        physical.min(4_294_967_040.0) as u32
    } else {
        0
    }
}

fn cell_config(scale: f32) -> AuroraConfig {
    let clamp_cell = |logical: f32| match u16::try_from(to_px(logical, scale)) {
        Ok(0) | Err(_) => 1,
        Ok(px) => px,
    };
    AuroraConfig {
        cell_width_px: clamp_cell(CELL_LOGICAL.0),
        cell_height_px: clamp_cell(CELL_LOGICAL.1),
        ..AuroraConfig::default()
    }
}

/// Owns the aurora engine, its clock and the two frame buffers.
#[derive(Debug)]
pub(crate) struct AuroraDriver {
    font: &'static [u8],
    aurora: Aurora,
    clock: FrameClock,
    front: SharedPixelBuffer<Rgba8Pixel>,
    back: SharedPixelBuffer<Rgba8Pixel>,
    size: FrameSize,
    scale: f32,
    last_tick: Instant,
    histogram: FrameHistogram,
}

impl AuroraDriver {
    /// Bakes the glyph atlas for scale 1; the first tick adapts to the real window.
    pub(crate) fn new(font: &'static [u8], reduced_motion: bool) -> Result<Self, GuiError> {
        let aurora = Aurora::new(cell_config(1.0), font)?;
        let size = aurora.frame_size();
        Ok(Self {
            font,
            aurora,
            clock: FrameClock::new(FRAME_STEP, reduced_motion),
            front: SharedPixelBuffer::new(size.width_px, size.height_px),
            back: SharedPixelBuffer::new(size.width_px, size.height_px),
            size,
            scale: 1.0,
            last_tick: Instant::now(),
            histogram: FrameHistogram::default(),
        })
    }

    /// Frame-time statistics so far.
    pub(crate) fn histogram(&self) -> &FrameHistogram {
        &self.histogram
    }

    /// Advances the clock and, when a frame is due and the window is visible, renders it.
    pub(crate) fn tick(&mut self, ui: &AppWindow) -> Result<(), GuiError> {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;

        let window = ui.window();
        // The session screen has no aurora: treat it like a hidden window (no frames, no CPU).
        let visible = window.is_visible() && !window.is_minimized() && !ui.get_in_session();
        self.clock.set_visible(visible);
        if !visible {
            return Ok(());
        }

        let scale = window.scale_factor();
        if (scale - self.scale).abs() > 0.001 {
            self.aurora = Aurora::new(cell_config(scale), self.font)?;
            self.scale = scale;
            self.size = FrameSize {
                width_px: 0,
                height_px: 0,
                cols: 0,
                rows: 0,
            };
        }
        let want_w = to_px(ui.get_atmosphere_width(), scale);
        let want_h = to_px(ui.get_atmosphere_height(), scale);
        if want_w == 0 || want_h == 0 {
            return Ok(());
        }
        if want_w != self.size.width_px || want_h != self.size.height_px {
            let size = self.aurora.resize(want_w, want_h)?;
            if size.width_px != self.size.width_px || size.height_px != self.size.height_px {
                self.front = SharedPixelBuffer::new(size.width_px, size.height_px);
                self.back = SharedPixelBuffer::new(size.width_px, size.height_px);
            }
            self.size = size;
            self.clock.invalidate();
        }

        let Some(t) = self.clock.advance(elapsed) else {
            return Ok(());
        };
        let started = Instant::now();
        self.aurora.render_rgba(t, self.back.make_mut_bytes())?;
        // Every aurora pixel is opaque (alpha 255), so straight and premultiplied RGBA are the
        // same bytes; declaring it premultiplied spares the renderer a conversion pass.
        ui.set_aurora_frame(Image::from_rgba8_premultiplied(self.back.clone()));
        core::mem::swap(&mut self.front, &mut self.back);
        self.histogram.record(started.elapsed());
        Ok(())
    }
}
