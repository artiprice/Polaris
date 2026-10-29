//! Tests for the RGBA blitter (descriptor `ui_ascii_aurora_blit.json`).

use std::time::Instant;

use polaris_ui_ascii_aurora::{Aurora, AuroraConfig, AuroraError, FrameSize};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../gui/assets/fonts/jetbrains-mono.ttf"
));

fn aurora_at(w: u32, h: u32) -> Result<Aurora, AuroraError> {
    let mut a = Aurora::new(AuroraConfig::default(), FONT)?;
    let _ = a.resize(w, h)?;
    Ok(a)
}

fn frame_len(size: FrameSize) -> usize {
    usize::try_from(size.width_px).unwrap_or(0) * usize::try_from(size.height_px).unwrap_or(0) * 4
}

#[test]
fn render_writes_every_pixel_opaque() -> TestResult {
    // Deliberately not a multiple of the 10x15 cell.
    let mut a = aurora_at(203, 97)?;
    let size = a.frame_size();
    assert_eq!((size.width_px, size.height_px), (203, 97));
    let mut buf = vec![0_u8; frame_len(size)];
    a.render_rgba(12.0, &mut buf)?;
    assert_eq!(buf.len(), 203 * 97 * 4);
    assert!(
        buf.as_chunks::<4>()
            .0
            .iter()
            .all(|px| px.get(3).copied() == Some(255))
    );
    // Pre-filled with a different garbage value the result is identical:
    // nothing is left untouched.
    let mut other = vec![0x5A_u8; frame_len(size)];
    a.render_rgba(12.0, &mut other)?;
    assert_eq!(buf, other);
    assert!(
        buf.as_chunks::<4>()
            .0
            .iter()
            .any(|px| px.first().is_some_and(|&r| r > 0))
    );
    Ok(())
}

#[test]
fn wrong_buffer_size_is_error() -> TestResult {
    let mut a = aurora_at(100, 60)?;
    let need = frame_len(a.frame_size());
    let mut small = vec![0_u8; need - 1];
    assert_eq!(
        a.render_rgba(0.0, &mut small),
        Err(AuroraError::BufferSize {
            expected: need,
            actual: need - 1
        })
    );
    let mut big = vec![0_u8; need + 4];
    assert!(matches!(
        a.render_rgba(0.0, &mut big),
        Err(AuroraError::BufferSize { .. })
    ));
    // The empty 0x0 renderer accepts only an empty buffer.
    let mut empty = Aurora::new(AuroraConfig::default(), FONT)?;
    assert_eq!(empty.render_rgba(0.0, &mut []), Ok(()));
    assert!(empty.render_rgba(0.0, &mut [0_u8; 4]).is_err());
    Ok(())
}

#[test]
fn render_does_not_reallocate() -> TestResult {
    let mut a = aurora_at(960, 600)?;
    let mut buf = vec![0_u8; frame_len(a.frame_size())];
    a.render_rgba(0.0, &mut buf)?;
    let before = a.debug_buffer_ptrs();
    let lens = a.debug_buffer_lens();
    for i in 0..20_u16 {
        a.render_rgba(f32::from(i) * 1.7, &mut buf)?;
    }
    assert_eq!(a.debug_buffer_ptrs(), before);
    assert_eq!(a.debug_buffer_lens(), lens);
    let _ = a.render_text(3.0);
    assert_eq!(a.debug_buffer_ptrs(), before);
    Ok(())
}

#[test]
fn resize_same_size_keeps_buffers() -> TestResult {
    let mut a = aurora_at(960, 600)?;
    let before = a.debug_buffer_ptrs();
    let lens = a.debug_buffer_lens();
    let size = a.resize(960, 600)?;
    assert_eq!(size, a.frame_size());
    assert_eq!(a.debug_buffer_ptrs(), before);
    assert_eq!(a.debug_buffer_lens(), lens);
    // A pixel-only change inside the same cell grid also keeps the buffers.
    let same_cells = a.resize(955, 595)?;
    assert_eq!((same_cells.cols, same_cells.rows), (96, 40));
    assert_eq!(a.debug_buffer_ptrs(), before);
    Ok(())
}

#[test]
fn resize_new_size_rebuilds() -> TestResult {
    let mut a = aurora_at(960, 600)?;
    let old = a.debug_buffer_lens();
    assert_eq!(old, [96 * 40, 96 * 40, 96 * 15]);
    let size = a.resize(1440, 900)?;
    assert_eq!((size.cols, size.rows), (144, 60));
    assert_eq!(a.debug_buffer_lens(), [144 * 60, 144 * 60, 144 * 15]);
    let mut buf = vec![0_u8; frame_len(size)];
    a.render_rgba(1.0, &mut buf)?;
    let small = a.resize(100, 100)?;
    assert_eq!((small.cols, small.rows), (10, 7));
    assert_eq!(a.debug_buffer_lens(), [70, 70, 150]);
    let mut buf = vec![0_u8; frame_len(small)];
    a.render_rgba(1.0, &mut buf)?;
    let zero = a.resize(0, 0)?;
    assert_eq!((zero.cols, zero.rows), (0, 0));
    a.render_rgba(1.0, &mut [])?;
    Ok(())
}

#[test]
fn resize_clamps_to_max_and_ceils_cells() -> TestResult {
    let mut a = Aurora::new(AuroraConfig::default(), FONT)?;
    let s = a.resize(5000, 4000)?;
    assert_eq!(
        s,
        FrameSize {
            width_px: 2560,
            height_px: 1440,
            cols: 256,
            rows: 96
        }
    );
    let s = a.resize(101, 31)?;
    assert_eq!((s.width_px, s.height_px, s.cols, s.rows), (101, 31, 11, 3));
    let s = a.resize(u32::MAX, 1)?;
    assert_eq!((s.width_px, s.cols, s.rows), (2560, 256, 1));
    assert_eq!(a.frame_size(), s);
    Ok(())
}

#[test]
fn zero_cell_is_size_error() {
    for (w, h) in [(0_u16, 15_u16), (10, 0), (0, 0)] {
        let config = AuroraConfig {
            cell_width_px: w,
            cell_height_px: h,
            ..AuroraConfig::default()
        };
        assert!(matches!(Aurora::new(config, FONT), Err(AuroraError::Size)));
    }
    assert!(matches!(
        Aurora::new(AuroraConfig::default(), b"not a font"),
        Err(AuroraError::Font)
    ));
}

#[test]
fn render_is_deterministic() -> TestResult {
    let mut a = aurora_at(400, 300)?;
    let mut b = aurora_at(400, 300)?;
    let mut x = vec![0_u8; frame_len(a.frame_size())];
    let mut y = vec![1_u8; x.len()];
    a.render_rgba(20.0, &mut x)?;
    b.render_rgba(20.0, &mut y)?;
    assert_eq!(x, y);
    // Rendering other times in between does not change the result (no hidden state).
    a.render_rgba(3.0, &mut y)?;
    a.render_rgba(20.0, &mut y)?;
    assert_eq!(x, y);
    // Time actually moves the picture.
    a.render_rgba(21.0, &mut y)?;
    assert_ne!(x, y);
    // A different seed gives a different picture.
    let config = AuroraConfig {
        seed: 7,
        ..AuroraConfig::default()
    };
    let mut c = Aurora::new(config, FONT)?;
    let _ = c.resize(400, 300)?;
    c.render_rgba(20.0, &mut y)?;
    assert_ne!(x, y);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn prop_render_valid_over_time(t in 0.0_f32..=600.0, w in 1_u32..=130, h in 1_u32..=90) {
        let mut a = aurora_at(w, h).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut buf = vec![0_u8; frame_len(a.frame_size())];
        a.render_rgba(t, &mut buf).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(buf.len(), usize::try_from(w * h * 4).unwrap_or(0));
        prop_assert!(buf.as_chunks::<4>().0.iter().all(|px| px.get(3).copied() == Some(255)));
    }
}

#[test]
fn grid_line_visible_on_cell_edges() -> TestResult {
    let (cols, rows) = (96_usize, 40_usize);
    let mut a = aurora_at(960, 600)?;
    let text = a.render_text(12.0);
    let lines: Vec<&str> = text.lines().collect();
    let blank = |r: usize, c: usize| -> bool {
        lines
            .get(r)
            .and_then(|l| l.chars().nth(c))
            .is_some_and(|ch| ch == ' ')
    };
    // An interior cell whose neighbourhood is entirely blank (no glow).
    let mut found = None;
    'search: for r in 8..rows - 8 {
        for c in 14..cols - 14 {
            let quiet = (r - 1..=r + 1).all(|rr| (c - 1..=c + 1).all(|cc| blank(rr, cc)));
            if quiet {
                found = Some((r, c));
                break 'search;
            }
        }
    }
    let (r, c) = found.ok_or("no quiet interior cell")?;
    let mut buf = vec![0_u8; frame_len(a.frame_size())];
    a.render_rgba(12.0, &mut buf)?;
    let sum = |x: usize, y: usize| -> u32 {
        let i = (y * 960 + x) * 4;
        buf.get(i..i + 3)
            .map_or(0, |p| p.iter().map(|&v| u32::from(v)).sum())
    };
    let interior = sum(c * 10 + 4, r * 15 + 7);
    let right_edge = sum(c * 10 + 9, r * 15 + 7);
    let bottom_edge = sum(c * 10 + 4, r * 15 + 14);
    assert!(
        right_edge >= interior + 12,
        "right {right_edge} vs interior {interior}"
    );
    assert!(
        bottom_edge >= interior + 12,
        "bottom {bottom_edge} vs interior {interior}"
    );
    // The line is faint (about 3 % white), not a bright rule.
    assert!(right_edge < 60, "grid too strong: {right_edge}");
    Ok(())
}

fn text_96x40() -> Result<String, AuroraError> {
    let mut a = aurora_at(960, 600)?;
    assert_eq!((a.frame_size().cols, a.frame_size().rows), (96, 40));
    Ok(a.render_text(12.0))
}

#[test]
fn render_text_blank_ratio() -> TestResult {
    let text = text_96x40()?;
    assert_eq!(text.lines().count(), 40);
    assert!(text.lines().all(|l| l.chars().count() == 96));
    let total = 96.0_f64 * 40.0;
    let blanks = f64::from(u32::try_from(text.chars().filter(|&c| c == ' ').count())?);
    let ratio = blanks / total;
    assert!((0.60..=0.75).contains(&ratio), "blank ratio {ratio}");
    // Only a few cells reach the inverted top band ('8' and '@' are the densest glyphs).
    let dense = u32::try_from(text.chars().filter(|&c| c == '8' || c == '@').count())?;
    assert!(dense < 100, "too many dense glyphs: {dense}");
    Ok(())
}

#[test]
fn render_text_golden_snapshot() -> TestResult {
    let text = text_96x40()?;
    insta::assert_snapshot!(text);
    Ok(())
}

#[test]
#[ignore = "visual check: writes PPM files under @prime/tmp"]
fn write_preview_ppm() -> TestResult {
    let dir = r"C:\mDev\Polaris\@prime\tmp\20261028-ascii";
    std::fs::create_dir_all(dir)?;
    let mut a = aurora_at(1440, 900)?;
    let size = a.frame_size();
    let mut buf = vec![0_u8; frame_len(size)];
    for (t, name) in [
        (12.0_f32, "preview-aurora.ppm"),
        (20.0, "preview-aurora-t20.ppm"),
    ] {
        a.render_rgba(t, &mut buf)?;
        let mut ppm = format!("P6\n{} {}\n255\n", size.width_px, size.height_px).into_bytes();
        for px in buf.as_chunks::<4>().0 {
            ppm.extend_from_slice(px.get(0..3).ok_or("short pixel")?);
        }
        std::fs::write(format!("{dir}\\{name}"), ppm)?;
    }
    Ok(())
}

#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn render_perf_1080p_average_under_bound() -> TestResult {
    let bound_ms: f64 = std::env::var("AURORA_PERF_BOUND_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15.0);
    let mut a = aurora_at(1920, 1080)?;
    let mut buf = vec![0_u8; frame_len(a.frame_size())];
    for i in 0..3_u16 {
        a.render_rgba(f32::from(i), &mut buf)?;
    }
    let frames = 30_u16;
    let start = Instant::now();
    for i in 0..frames {
        a.render_rgba(10.0 + f32::from(i) * 0.033, &mut buf)?;
    }
    let avg_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    assert!(
        avg_ms < bound_ms,
        "measured average {avg_ms:.3} ms/frame at 1920x1080 (bound {bound_ms} ms)"
    );
    Ok(())
}
