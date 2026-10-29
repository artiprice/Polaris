//! Perceptual comparison of screenshots: a 64-bit DCT hash (pHash) for "is this the same page",
//! and a coarse luminance grid that says where two same-sized screenshots differ.

use image::{DynamicImage, GrayImage, imageops::FilterType};

use crate::BrowserError;

/// Largest encoded screenshot accepted, in bytes.
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// Largest decoded screenshot side, in pixels.
const MAX_SIDE: u32 = 8192;
/// Hamming distance up to which two hashes count as the same page.
pub const SIMILAR_DISTANCE: u32 = 10;
/// Cells per side of the change grid.
pub const GRID: u32 = 8;
/// Mean luminance difference (0–255) above which a grid cell counts as changed.
const CELL_THRESHOLD: f64 = 12.0;

/// A 64-bit perceptual hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PerceptualHash(pub u64);

impl PerceptualHash {
    /// Bits that differ from `other`.
    #[must_use]
    pub const fn distance(self, other: Self) -> u32 {
        (self.0 ^ other.0).count_ones()
    }
}

/// How two screenshots compare.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualDiff {
    /// Hash of the first image.
    pub before: PerceptualHash,
    /// Hash of the second image.
    pub after: PerceptualHash,
    /// Hamming distance of the hashes (0–64).
    pub distance: u32,
    /// `distance <= SIMILAR_DISTANCE`.
    pub similar: bool,
    /// Changed cells of a `GRID`×`GRID` grid as `(column, row)`, when both images have the same
    /// size; `None` when the sizes differ.
    pub changed_cells: Option<Vec<(u32, u32)>>,
}

/// Decode an encoded screenshot (PNG, JPEG or WebP).
/// # Errors
/// Oversized or undecodable input.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage, BrowserError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(BrowserError::TooLarge {
            what: "screenshot",
            limit: MAX_IMAGE_BYTES,
        });
    }
    let image =
        image::load_from_memory(bytes).map_err(|_| BrowserError::Malformed("screenshot"))?;
    if image.width() == 0
        || image.height() == 0
        || image.width() > MAX_SIDE
        || image.height() > MAX_SIDE
    {
        return Err(BrowserError::Malformed("screenshot size"));
    }
    Ok(image)
}

/// pHash: grayscale 32×32, 2-D DCT-II, the 8×8 low frequencies without the DC term compared
/// with their median.
#[must_use]
pub fn perceptual_hash(image: &DynamicImage) -> PerceptualHash {
    const SIZE: usize = 32;
    const LOW: usize = 8;
    let small = image
        .resize_exact(32, 32, FilterType::Triangle)
        .into_luma8();
    let pixels: Vec<f64> = small.pixels().map(|pixel| f64::from(pixel.0[0])).collect();
    let cosines: Vec<f64> = (0..LOW)
        .flat_map(|frequency| {
            (0..SIZE).map(move |position| {
                let angle = std::f64::consts::PI * (2.0 * position as f64 + 1.0) * frequency as f64
                    / (2.0 * SIZE as f64);
                angle.cos()
            })
        })
        .collect();
    let cosine = |frequency: usize, position: usize| {
        cosines
            .get(frequency * SIZE + position)
            .copied()
            .unwrap_or_default()
    };
    let mut coefficients = Vec::with_capacity(LOW * LOW);
    for v in 0..LOW {
        for u in 0..LOW {
            let mut sum = 0.0;
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let pixel = pixels.get(y * SIZE + x).copied().unwrap_or_default();
                    sum += pixel * cosine(u, x) * cosine(v, y);
                }
            }
            coefficients.push(sum);
        }
    }
    let ac: Vec<f64> = coefficients.iter().skip(1).copied().collect();
    let mut sorted = ac.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    let bits = coefficients
        .iter()
        .enumerate()
        .filter(|(index, value)| *index > 0 && **value > median)
        .fold(0_u64, |hash, (index, _)| hash | (1_u64 << index));
    PerceptualHash(bits)
}

/// Compare two decoded screenshots.
#[must_use]
pub fn compare(before: &DynamicImage, after: &DynamicImage) -> VisualDiff {
    let (first, second) = (perceptual_hash(before), perceptual_hash(after));
    let distance = first.distance(second);
    let changed_cells = (before.width() == after.width() && before.height() == after.height())
        .then(|| changed_cells(&before.to_luma8(), &after.to_luma8()));
    VisualDiff {
        before: first,
        after: second,
        distance,
        similar: distance <= SIMILAR_DISTANCE,
        changed_cells,
    }
}

/// Decode and compare two encoded screenshots (`/ui-diff`).
/// # Errors
/// Either input is oversized or undecodable.
pub fn visual_diff(before: &[u8], after: &[u8]) -> Result<VisualDiff, BrowserError> {
    Ok(compare(&decode(before)?, &decode(after)?))
}

fn changed_cells(before: &GrayImage, after: &GrayImage) -> Vec<(u32, u32)> {
    let (width, height) = before.dimensions();
    let mut changed = Vec::new();
    for row in 0..GRID {
        for column in 0..GRID {
            let (x0, x1) = (column * width / GRID, (column + 1) * width / GRID);
            let (y0, y1) = (row * height / GRID, (row + 1) * height / GRID);
            let mut total = 0.0;
            let mut count = 0_u32;
            for y in y0..y1.max(y0 + 1).min(height) {
                for x in x0..x1.max(x0 + 1).min(width) {
                    if let (Some(a), Some(b)) = (
                        before.get_pixel_checked(x, y),
                        after.get_pixel_checked(x, y),
                    ) {
                        total += (f64::from(a.0[0]) - f64::from(b.0[0])).abs();
                        count = count.saturating_add(1);
                    }
                }
            }
            if count > 0 && total / f64::from(count) > CELL_THRESHOLD {
                changed.push((column, row));
            }
        }
    }
    changed
}
