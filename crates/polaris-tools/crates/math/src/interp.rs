//! Validated interpolation primitives.
use crate::{Finite, MathError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use splines::{Interpolation, Key, Spline};

/// Interpolation method for a knot sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interp {
    Linear,
    Step,
    CatmullRom,
    Bezier,
}
/// A finite sample point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Knot {
    /// x coordinate
    pub x: Finite,
    /// y coordinate
    pub y: Finite,
}
/// Finite point in two dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Point2 {
    /// x coordinate
    pub x: Finite,
    /// y coordinate
    pub y: Finite,
}
/// A rectangular grid: z values are `[z00, z10, z01, z11]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Grid2 {
    /// lower and upper x bounds
    pub x: [Finite; 2],
    /// lower and upper y bounds
    pub y: [Finite; 2],
    /// row-major values at lower/lower, upper/lower, lower/upper, upper/upper
    pub z: [Finite; 4],
}

/// Linear interpolation with finite input/output checks.
pub fn lerp(a: Finite, b: Finite, t: Finite) -> Result<Finite, MathError> {
    Finite::new(a.get() + (b.get() - a.get()) * t.get())
}

/// Interpolates a strictly ascending knot sequence. Out-of-range queries are errors.
pub fn interpolate(knots: &[Knot], kind: Interp, x: Finite) -> Result<Finite, MathError> {
    validate_knots(knots)?;
    let first = knots
        .first()
        .ok_or(MathError::BadKnots("at least two knots are required"))?;
    let last = knots
        .last()
        .ok_or(MathError::BadKnots("at least two knots are required"))?;
    if knots.len() < 2 || x.get() < first.x.get() || x.get() > last.x.get() {
        return Err(MathError::BadKnots("query outside knot range"));
    }
    if kind == Interp::Bezier {
        if knots.len() > 64 {
            return Err(MathError::BadKnots(
                "Bezier interpolation supports at most 64 knots",
            ));
        }
        let t = Finite::new((x.get() - first.x.get()) / (last.x.get() - first.x.get()))?;
        let controls: Vec<Point2> = knots
            .iter()
            .map(|knot| Point2 {
                x: knot.y,
                y: knot.y,
            })
            .collect();
        return Ok(bezier(&controls, t)?.y);
    }
    if let Some(knot) = knots
        .iter()
        .find(|knot| knot.x.get().total_cmp(&x.get()).is_eq())
    {
        return Ok(knot.y);
    }
    let mode = match kind {
        Interp::Linear => Interpolation::Linear,
        Interp::Step => Interpolation::Step(0.0),
        Interp::CatmullRom => Interpolation::CatmullRom,
        Interp::Bezier => Interpolation::Linear,
    };
    if kind == Interp::CatmullRom {
        if knots.len() < 4 {
            return Err(MathError::BadKnots(
                "Catmull-Rom requires at least four uniformly spaced knots",
            ));
        }
        let step = knots
            .get(1)
            .zip(knots.first())
            .map(|(b, a)| b.x.get() - a.x.get())
            .ok_or(MathError::BadKnots("missing knot"))?;
        if !step.is_finite() || step <= 0.0 {
            return Err(MathError::BadKnots(
                "Catmull-Rom requires finite positive uniform spacing",
            ));
        }
        let coordinate_scale = knots
            .iter()
            .map(|knot| knot.x.get().abs())
            .fold(step.abs(), f64::max);
        let spacing_tolerance = (step.abs() * 1e-12).max(coordinate_scale * f64::EPSILON * 2.0);
        for pair in knots.windows(2) {
            let delta = pair
                .get(1)
                .zip(pair.first())
                .map(|(b, a)| b.x.get() - a.x.get())
                .ok_or(MathError::BadKnots("missing knot"))?;
            if !delta.is_finite() || (delta - step).abs() > spacing_tolerance {
                return Err(MathError::BadKnots("Catmull-Rom requires uniform spacing"));
            }
        }
    }
    let keys: Vec<Key<f64, f64>> = knots
        .iter()
        .enumerate()
        .map(|(index, k)| {
            Key::new(
                k.x.get(),
                k.y.get(),
                if index + 1 < knots.len() {
                    mode
                } else {
                    Interpolation::Linear
                },
            )
        })
        .collect();
    let spline = Spline::from_vec(keys);
    let value = spline.sample(x.get()).ok_or(MathError::BadKnots(
        "query is outside interpolation support",
    ))?;
    Finite::new(value)
}

/// Bilinear interpolation over a finite increasing rectangle.
pub fn bilinear(grid: &Grid2, x: Finite, y: Finite) -> Result<Finite, MathError> {
    let x0 = grid
        .x
        .first()
        .ok_or(MathError::BadKnots("missing x axis"))?
        .get();
    let x1 = grid
        .x
        .get(1)
        .ok_or(MathError::BadKnots("missing x axis"))?
        .get();
    let y0 = grid
        .y
        .first()
        .ok_or(MathError::BadKnots("missing y axis"))?
        .get();
    let y1 = grid
        .y
        .get(1)
        .ok_or(MathError::BadKnots("missing y axis"))?
        .get();
    if x0 >= x1 || y0 >= y1 || x.get() < x0 || x.get() > x1 || y.get() < y0 || y.get() > y1 {
        return Err(MathError::BadKnots("invalid grid or query outside grid"));
    }
    let tx = Finite::new((x.get() - x0) / (x1 - x0))?;
    let ty = Finite::new((y.get() - y0) / (y1 - y0))?;
    let z00 = grid
        .z
        .first()
        .copied()
        .ok_or(MathError::BadKnots("missing grid corner"))?;
    let z10 = grid
        .z
        .get(1)
        .copied()
        .ok_or(MathError::BadKnots("missing grid corner"))?;
    let z01 = grid
        .z
        .get(2)
        .copied()
        .ok_or(MathError::BadKnots("missing grid corner"))?;
    let z11 = grid
        .z
        .get(3)
        .copied()
        .ok_or(MathError::BadKnots("missing grid corner"))?;
    let lower = lerp(z00, z10, tx)?;
    let upper = lerp(z01, z11, tx)?;
    lerp(lower, upper, ty)
}

/// De Casteljau evaluation for 1–64 finite control points and `t` in `[0,1]`.
pub fn bezier(control: &[Point2], t: Finite) -> Result<Point2, MathError> {
    if control.is_empty() || control.len() > 64 || !(0.0..=1.0).contains(&t.get()) {
        return Err(MathError::BadKnots(
            "Bezier needs 1-64 points and t in [0,1]",
        ));
    }
    let mut points = control.to_vec();
    while points.len() > 1 {
        let mut next = Vec::with_capacity(points.len().saturating_sub(1));
        for pair in points.windows(2) {
            let a = pair
                .first()
                .ok_or(MathError::BadKnots("invalid point sequence"))?;
            let b = pair
                .get(1)
                .ok_or(MathError::BadKnots("invalid point sequence"))?;
            next.push(Point2 {
                x: lerp(a.x, b.x, t)?,
                y: lerp(a.y, b.y, t)?,
            });
        }
        points = next;
    }
    points
        .first()
        .copied()
        .ok_or(MathError::BadKnots("empty Bezier result"))
}

fn validate_knots(knots: &[Knot]) -> Result<(), MathError> {
    if knots.len() < 2 || knots.len() > 4096 {
        return Err(MathError::BadKnots("expected 2-4096 knots"));
    }
    for pair in knots.windows(2) {
        let a = pair.first().ok_or(MathError::BadKnots("invalid knots"))?;
        let b = pair.get(1).ok_or(MathError::BadKnots("invalid knots"))?;
        if a.x.get() >= b.x.get() {
            return Err(MathError::BadKnots(
                "x coordinates must be strictly increasing",
            ));
        }
    }
    Ok(())
}
