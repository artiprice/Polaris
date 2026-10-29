//! Tests for the frame clock (descriptor `ui_ascii_aurora_clock.json`).

use core::time::Duration;

use polaris_ui_ascii_aurora::clock::{MAX_STEP, MIN_STEP};
use polaris_ui_ascii_aurora::{FrameClock, STATIC_T};

const STEP: Duration = Duration::from_millis(33);

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn first_advance_renders_immediately() {
    let mut clock = FrameClock::new(STEP, false);
    assert_eq!(clock.advance(Duration::ZERO), Some(0.0));
    assert_eq!(clock.advance(Duration::ZERO), None);
}

#[test]
fn no_frame_before_step_elapsed() {
    let mut clock = FrameClock::new(STEP, false);
    let _ = clock.advance(Duration::ZERO);
    assert_eq!(clock.advance(Duration::from_millis(10)), None);
    assert_eq!(clock.advance(Duration::from_millis(10)), None);
    assert!(clock.advance(Duration::from_millis(13)).is_some());
}

#[test]
fn fixed_step_advance() {
    let mut clock = FrameClock::new(STEP, false);
    let _ = clock.advance(Duration::ZERO);
    let step = STEP.as_secs_f32();
    for n in 1..=10_u16 {
        let t = clock.advance(STEP);
        let want = f32::from(n) * step;
        assert!(
            t.is_some_and(|t| close(t, want)),
            "frame {n}: {t:?} vs {want}"
        );
    }
    assert!(close(clock.sim_seconds(), 10.0 * step));
}

#[test]
fn hidden_pauses_time() {
    let mut clock = FrameClock::new(STEP, false);
    let _ = clock.advance(Duration::ZERO);
    let t1 = clock.advance(STEP);
    assert!(t1.is_some());
    clock.set_visible(false);
    let before = clock.sim_seconds();
    for _ in 0..50 {
        assert_eq!(clock.advance(Duration::from_secs(1)), None);
    }
    assert!(close(clock.sim_seconds(), before));
    clock.set_visible(true);
    // Becoming visible repaints once at the same time, then resumes stepping.
    assert!(
        clock
            .advance(Duration::ZERO)
            .is_some_and(|t| close(t, before))
    );
    assert_eq!(clock.advance(Duration::from_millis(1)), None);
    assert!(
        clock
            .advance(STEP)
            .is_some_and(|t| close(t, before + STEP.as_secs_f32()))
    );
}

#[test]
fn no_catch_up_burst_after_stall() {
    let mut clock = FrameClock::new(STEP, false);
    let _ = clock.advance(Duration::ZERO);
    let t = clock.advance(Duration::from_secs(5));
    assert!(t.is_some_and(|t| close(t, STEP.as_secs_f32())), "{t:?}");
    assert_eq!(clock.advance(Duration::from_millis(1)), None);
    assert_eq!(clock.advance(Duration::from_millis(1)), None);
    // Cadence resumes with one step per step of wall time.
    let t2 = clock.advance(STEP);
    assert!(
        t2.is_some_and(|t| close(t, 2.0 * STEP.as_secs_f32())),
        "{t2:?}"
    );
}

#[test]
fn reduced_motion_static_once() {
    let mut clock = FrameClock::new(STEP, true);
    assert_eq!(clock.advance(Duration::ZERO), Some(STATIC_T));
    for _ in 0..20 {
        assert_eq!(clock.advance(Duration::from_secs(1)), None);
    }
    assert!(STATIC_T.is_finite() && STATIC_T > 0.0);
}

#[test]
fn invalidate_forces_next_frame() {
    let mut reduced = FrameClock::new(STEP, true);
    assert_eq!(reduced.advance(Duration::ZERO), Some(STATIC_T));
    assert_eq!(reduced.advance(Duration::ZERO), None);
    reduced.invalidate();
    assert_eq!(reduced.advance(Duration::ZERO), Some(STATIC_T));
    assert_eq!(reduced.advance(Duration::ZERO), None);

    let mut live = FrameClock::new(STEP, false);
    let _ = live.advance(Duration::ZERO);
    let _ = live.advance(STEP);
    let t = live.sim_seconds();
    assert_eq!(live.advance(Duration::from_millis(1)), None);
    live.invalidate();
    // Fewer ms than a step elapsed, yet a frame is due, without moving time.
    assert!(
        live.advance(Duration::from_millis(1))
            .is_some_and(|x| close(x, t))
    );
    assert!(close(live.sim_seconds(), t));
    assert_eq!(live.advance(Duration::from_millis(1)), None);
}

#[test]
fn step_is_bounded() {
    assert_eq!(FrameClock::new(Duration::ZERO, false).step(), MIN_STEP);
    assert_eq!(
        FrameClock::new(Duration::from_secs(3600), false).step(),
        MAX_STEP
    );
    assert_eq!(FrameClock::new(STEP, false).step(), STEP);
    // A zero step must not spin: each call still yields at most one frame.
    let mut clock = FrameClock::new(Duration::ZERO, false);
    let _ = clock.advance(Duration::ZERO);
    assert!(clock.advance(Duration::from_secs(10)).is_some());
    assert!(clock.sim_seconds() <= MIN_STEP.as_secs_f32() * 1.01);
}
