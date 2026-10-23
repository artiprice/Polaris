use polaris_tools_math::{
    CalcLimits, Finite, Interp, Knot, Point2, bezier, calc, interpolate, summarize,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

proptest! {
    #[test]
    fn bounded_integer_decimal_addition_matches_decimal(a in -1_000_000_i32..1_000_000_i32, b in -1_000_000_i32..1_000_000_i32) {
        let expression = format!("{a} + {b}");
        let expected = Decimal::from(a) + Decimal::from(b);
        prop_assert_eq!(calc(&expression,&CalcLimits::default()),Ok(expected));
    }

    #[test]
    fn generated_finite_summaries_keep_order(values in prop::collection::vec(-1_000_000_i32..1_000_000_i32,1..128)) {
        let finite: Vec<Finite> = values.iter().map(|v|Finite::new(f64::from(*v)).unwrap_or(Finite::ZERO)).collect();
        let summary = summarize(&finite);
        prop_assert!(summary.is_ok());
        if let Ok(summary) = summary {
            prop_assert!(summary.min.get() <= summary.p50.get());
            prop_assert!(summary.p50.get() <= summary.p95.get());
            prop_assert!(summary.p95.get() <= summary.p99.get());
            prop_assert!(summary.p99.get() <= summary.max.get());
        }
    }

    #[test]
    fn generated_bezier_points_remain_in_control_bounds(
        xs in prop::collection::vec(-100_000_i32..100_000_i32, 2..16),
        ys in prop::collection::vec(-100_000_i32..100_000_i32, 2..16),
        t in 0.0_f64..=1.0_f64,
    ) {
        let count = xs.len().min(ys.len());
        let control: Vec<Point2> = xs.iter().zip(ys.iter()).take(count).map(|(x,y)| Point2 {
            x: Finite::new(f64::from(*x)).unwrap_or(Finite::ZERO),
            y: Finite::new(f64::from(*y)).unwrap_or(Finite::ZERO),
        }).collect();
        let result = bezier(&control,Finite::new(t).unwrap_or(Finite::ZERO));
        prop_assert!(result.is_ok());
        if let Ok(point) = result {
            let min_x = control.iter().map(|p|p.x.get()).fold(f64::INFINITY,f64::min);
            let max_x = control.iter().map(|p|p.x.get()).fold(f64::NEG_INFINITY,f64::max);
            let min_y = control.iter().map(|p|p.y.get()).fold(f64::INFINITY,f64::min);
            let max_y = control.iter().map(|p|p.y.get()).fold(f64::NEG_INFINITY,f64::max);
            prop_assert!(point.x.get() >= min_x - 1e-8 && point.x.get() <= max_x + 1e-8);
            prop_assert!(point.y.get() >= min_y - 1e-8 && point.y.get() <= max_y + 1e-8);
        }
    }

    #[test]
    fn generated_linear_knots_are_reproduced(
        ys in prop::collection::vec(-100_000_i32..100_000_i32, 2..32)
    ) {
        let knots: Vec<Knot> = ys.iter().enumerate().map(|(index,y)| Knot {
            x: Finite::new(f64::from(u32::try_from(index).unwrap_or(0))).unwrap_or(Finite::ZERO),
            y: Finite::new(f64::from(*y)).unwrap_or(Finite::ZERO),
        }).collect();
        for knot in &knots {
            prop_assert_eq!(interpolate(&knots,Interp::Linear,knot.x),Ok(knot.y));
        }
    }
}
