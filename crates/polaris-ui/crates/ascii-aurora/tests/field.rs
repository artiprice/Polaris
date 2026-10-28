//! Tests for the pure aurora field (descriptor `ui_ascii_aurora_field.json`).

use std::collections::BTreeSet;

use polaris_ui_ascii_aurora::field::{FieldFrame, edge_mask, sample, star_look};
use polaris_ui_ascii_aurora::params::{
    PALETTE, STAR_LATTICE_COLS, STAR_LATTICE_ROWS, STAR_MAX, STAR_MIN, STAR_PALETTE, TINT_GAIN,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SEED: u64 = 0x5EED_A0E0;

fn rgb(c: [u8; 3]) -> [f32; 3] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
    ]
}

/// Interior lattice cell centres (where the edge mask is 1).
fn interior_cells() -> Vec<(u32, u32, f32, f32)> {
    let mut cells = Vec::new();
    for col in 0..STAR_LATTICE_COLS {
        for row in 0..STAR_LATTICE_ROWS {
            let u = (f32::from(u16::try_from(col).unwrap_or(0)) + 0.5)
                / f32::from(u16::try_from(STAR_LATTICE_COLS).unwrap_or(1));
            let v = (f32::from(u16::try_from(row).unwrap_or(0)) + 0.5)
                / f32::from(u16::try_from(STAR_LATTICE_ROWS).unwrap_or(1));
            if (0.2..=0.8).contains(&u) && (0.2..=0.8).contains(&v) {
                cells.push((col, row, u, v));
            }
        }
    }
    cells
}

#[test]
fn deterministic_for_same_seed_and_time() {
    for &(t, u, v) in &[(0.0, 0.3, 0.4), (12.5, 0.55, 0.6), (333.3, 0.8, 0.2)] {
        let a = sample(SEED, t, u, v);
        let b = sample(SEED, t, u, v);
        assert_eq!(a, b);
        let frame = FieldFrame::new(SEED, t);
        assert_eq!(frame.sample(u, v), a);
    }
}

#[test]
fn different_seed_changes_stars() {
    let cells = interior_cells();
    let frame_a = FieldFrame::new(1, 5.0);
    let frame_b = FieldFrame::new(2, 5.0);
    let stars = |frame: &FieldFrame| -> BTreeSet<(u32, u32)> {
        cells
            .iter()
            .filter(|&&(_, _, u, v)| frame.sample(u, v).star > 0.0)
            .map(|&(c, r, _, _)| (c, r))
            .collect()
    };
    let a = stars(&frame_a);
    let b = stars(&frame_b);
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn prop_values_finite_and_in_range(t in 0.0_f32..=600.0, u in 0.0_f32..=1.0, v in 0.0_f32..=1.0) {
        let s = sample(SEED, t, u, v);
        prop_assert!(s.luminance.is_finite() && (0.0..=1.0).contains(&s.luminance));
        prop_assert!(s.star.is_finite() && (0.0..=1.0).contains(&s.star));
        for ch in s.tint {
            prop_assert!(ch.is_finite() && (0.0..=1.0).contains(&ch));
        }
    }
}

#[test]
fn edge_mask_zero_at_borders() {
    let steps = 40_u16;
    for i in 0..=steps {
        let x = f32::from(i) / f32::from(steps);
        for t in [0.0, 9.0, 31.0] {
            for (u, v) in [(x, 0.0), (x, 1.0), (0.0, x), (1.0, x)] {
                let s = sample(SEED, t, u, v);
                assert!(s.luminance == 0.0, "lum at ({u},{v}) = {}", s.luminance);
                assert!(s.star == 0.0, "star at ({u},{v}) = {}", s.star);
                assert!(edge_mask(u, v) == 0.0);
            }
        }
    }
    assert!((edge_mask(0.5, 0.5) - 1.0).abs() < 1e-6);
    // The mask rises smoothly: monotonic from the border to the interior.
    let mut last = 0.0_f32;
    for i in 0..=20_u16 {
        let u = f32::from(i) / 100.0;
        let m = edge_mask(u, 0.5);
        assert!(m >= last);
        last = m;
    }
}

#[test]
fn ribbon_brightest_near_curve() {
    let frame = FieldFrame::new(SEED, 0.0);
    let line = frame.ribbon_polyline(0);
    let mut on = Vec::new();
    let mut off = Vec::new();
    for i in 8..56_usize {
        let (Some(p), Some(a), Some(b)) = (line.get(i), line.get(i - 1), line.get(i + 1)) else {
            continue;
        };
        let (dx, dy) = ((b[0] - a[0]) * 1.6, b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 1e-6 {
            continue;
        }
        // Unit normal in aspect space, converted back to (u, v).
        let (nx, ny) = (-dy / len, dx / len);
        let inside = |u: f32, v: f32| (0.2..=0.8).contains(&u) && (0.2..=0.8).contains(&v);
        let q1 = (p[0] + 0.2 * nx / 1.6, p[1] + 0.2 * ny);
        let q2 = (p[0] - 0.2 * nx / 1.6, p[1] - 0.2 * ny);
        if inside(p[0], p[1]) && inside(q1.0, q1.1) && inside(q2.0, q2.1) {
            on.push(frame.sample(p[0], p[1]).luminance);
            off.push(frame.sample(q1.0, q1.1).luminance);
            off.push(frame.sample(q2.0, q2.1).luminance);
        }
    }
    assert!(on.len() >= 5, "only {} usable curve points", on.len());
    let mean = |v: &[f32]| v.iter().sum::<f32>() / f32::from(u16::try_from(v.len()).unwrap_or(1));
    let (m_on, m_off) = (mean(&on), mean(&off));
    assert!(m_on > 0.3, "on-curve mean {m_on}");
    assert!(m_on > 2.0 * m_off, "on {m_on} vs off {m_off}");
}

#[test]
fn stars_seeded_sparse_and_palette_bound() -> TestResult {
    let cells = interior_cells();
    let frame = FieldFrame::new(SEED, 7.0);
    let palette: Vec<[f32; 3]> = STAR_PALETTE.iter().map(|&c| rgb(c)).collect();
    let mut stars = 0_u32;
    for &(_, _, u, v) in &cells {
        let s = frame.sample(u, v);
        if s.star > 0.0 {
            stars += 1;
            assert!(
                (STAR_MIN - 1e-4..=STAR_MAX + 1e-4).contains(&s.star),
                "star opacity {}",
                s.star
            );
            let look = star_look(SEED, u, v);
            assert!(palette.contains(&look.color), "colour {:?}", look.color);
        }
    }
    let total = u32::try_from(cells.len())?;
    let fraction = f64::from(stars) / f64::from(total);
    assert!(
        fraction > 0.002 && fraction < 0.02,
        "star fraction {fraction} ({stars}/{total})"
    );
    // Twinkle really changes opacity over time for at least one star.
    let first = cells
        .iter()
        .find(|&&(_, _, u, v)| frame.sample(u, v).star > 0.0)
        .ok_or("no star found")?;
    let mut seen = BTreeSet::new();
    for step in 0..40_u16 {
        let t = f32::from(step) * 0.7;
        let s = FieldFrame::new(SEED, t).sample(first.2, first.3).star;
        seen.insert((s * 1000.0).to_bits());
    }
    assert!(seen.len() > 10);
    Ok(())
}

#[test]
fn animation_is_smooth() {
    let n = 48_u16;
    let m = 30_u16;
    for &t in &[0.0_f32, 7.3, 29.0, 60.2, 100.3, 347.1] {
        let a = FieldFrame::new(SEED, t);
        let b = FieldFrame::new(SEED, t + 1.0 / 30.0);
        let mut worst = 0.0_f32;
        let mut worst_tint = 0.0_f32;
        for i in 0..=n {
            for j in 0..=m {
                let u = f32::from(i) / f32::from(n);
                let v = f32::from(j) / f32::from(m);
                let (sa, sb) = (a.sample(u, v), b.sample(u, v));
                worst = worst.max((sa.luminance - sb.luminance).abs());
                worst_tint = worst_tint.max(
                    sa.tint
                        .iter()
                        .zip(sb.tint)
                        .map(|(x, y)| (x - y).abs())
                        .fold(0.0_f32, f32::max),
                );
            }
        }
        assert!(worst < 0.03, "t={t}: max luminance step {worst}");
        assert!(worst_tint < 0.03, "t={t}: max tint step {worst_tint}");
    }
}

#[test]
fn tint_stays_inside_palette_range() {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for &c in &PALETTE {
        for (k, ch) in rgb(c).into_iter().enumerate() {
            if let (Some(l), Some(h)) = (lo.get_mut(k), hi.get_mut(k)) {
                *l = l.min((ch * TINT_GAIN).min(1.0));
                *h = h.max((ch * TINT_GAIN).min(1.0));
            }
        }
    }
    for step in 0..200_u16 {
        let u = f32::from(step % 20) / 19.0;
        let v = f32::from(step / 20) / 9.0;
        let s = sample(SEED, f32::from(step) * 1.7, u, v);
        for (k, ch) in s.tint.into_iter().enumerate() {
            let (Some(&l), Some(&h)) = (lo.get(k), hi.get(k)) else {
                continue;
            };
            assert!(
                ch >= l - 1e-4 && ch <= h + 1e-4,
                "channel {k} = {ch} outside {l}..{h}"
            );
        }
    }
}
