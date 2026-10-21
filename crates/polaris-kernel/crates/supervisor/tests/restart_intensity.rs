//! Restart Intensity (descriptor: specs/descriptors/supervisor_restart_intensity.json).

use std::error::Error;
use std::time::Duration;

use polaris_kernel_supervisor::{RestartIntensity, RestartVerdict, RestartWindow};
use proptest::prelude::*;
use tokio::time::Instant;

type TestResult = Result<(), Box<dyn Error>>;

fn intensity(max_restarts: u32, window_ms: u64) -> RestartIntensity {
    RestartIntensity {
        max_restarts,
        window: Duration::from_millis(window_ms),
    }
}

#[test]
fn default_is_three_per_ten_seconds() -> TestResult {
    let d = RestartIntensity::default();
    assert_eq!(d.max_restarts, 3);
    assert_eq!(d.window, Duration::from_secs(10));
    Ok(())
}

#[test]
fn allows_up_to_max_then_exceeds() -> TestResult {
    let t0 = Instant::now();
    let mut w = RestartWindow::new(intensity(3, 10_000));
    for expected in 1..=3_u32 {
        assert_eq!(
            w.record(t0),
            RestartVerdict::Allowed {
                restarts_in_window: expected
            }
        );
    }
    assert_eq!(w.record(t0), RestartVerdict::Exceeded);
    Ok(())
}

#[test]
fn window_slides() -> TestResult {
    let t0 = Instant::now();
    let mut w = RestartWindow::new(intensity(2, 1_000));
    assert!(matches!(w.record(t0), RestartVerdict::Allowed { .. }));
    assert!(matches!(
        w.record(t0 + Duration::from_millis(400)),
        RestartVerdict::Allowed { .. }
    ));
    assert_eq!(
        w.record(t0 + Duration::from_millis(900)),
        RestartVerdict::Exceeded
    );
    // The first restart (t0) has aged out at t0 + 1000 ms, the second is still inside.
    assert_eq!(
        w.record(t0 + Duration::from_millis(1_000)),
        RestartVerdict::Allowed {
            restarts_in_window: 2
        }
    );
    Ok(())
}

#[test]
fn exact_window_age_is_outside() -> TestResult {
    let t0 = Instant::now();
    let mut w = RestartWindow::new(intensity(1, 1_000));
    assert!(matches!(w.record(t0), RestartVerdict::Allowed { .. }));
    assert_eq!(
        w.record(t0 + Duration::from_millis(999)),
        RestartVerdict::Exceeded
    );
    assert_eq!(
        w.record(t0 + Duration::from_millis(1_000)),
        RestartVerdict::Allowed {
            restarts_in_window: 1
        }
    );
    Ok(())
}

#[test]
fn zero_max_never_restarts() -> TestResult {
    let t0 = Instant::now();
    let mut w = RestartWindow::new(intensity(0, 1_000));
    assert_eq!(w.record(t0), RestartVerdict::Exceeded);
    assert_eq!(
        w.record(t0 + Duration::from_secs(3_600)),
        RestartVerdict::Exceeded
    );
    Ok(())
}

#[test]
fn exceeded_requests_are_not_recorded() -> TestResult {
    let t0 = Instant::now();
    let mut w = RestartWindow::new(intensity(1, 1_000));
    assert!(matches!(w.record(t0), RestartVerdict::Allowed { .. }));
    // Hammering inside the window must not push the lockout further out.
    for ms in [100, 300, 500, 900] {
        assert_eq!(
            w.record(t0 + Duration::from_millis(ms)),
            RestartVerdict::Exceeded
        );
    }
    assert!(matches!(
        w.record(t0 + Duration::from_millis(1_000)),
        RestartVerdict::Allowed { .. }
    ));
    Ok(())
}

proptest! {
    #[test]
    fn prop_restarts_per_window_bounded(
        max in 0_u32..6,
        window_ms in 1_u64..2_000,
        deltas in proptest::collection::vec(0_u64..1_500, 0..60),
    ) {
        let t0 = Instant::now();
        let mut w = RestartWindow::new(intensity(max, window_ms));
        let mut now_ms = 0_u64;
        let mut accepted: Vec<u64> = Vec::new();
        for delta in deltas {
            now_ms += delta;
            if let RestartVerdict::Allowed { restarts_in_window } =
                w.record(t0 + Duration::from_millis(now_ms))
            {
                prop_assert!(restarts_in_window <= max);
                accepted.push(now_ms);
            }
        }
        for start in &accepted {
            let inside = accepted
                .iter()
                .filter(|t| **t >= *start && **t < start + window_ms)
                .count();
            prop_assert!(inside <= max as usize, "{inside} restarts in one window, max {max}");
        }
    }
}
