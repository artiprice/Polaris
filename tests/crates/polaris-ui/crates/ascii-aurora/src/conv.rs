// Unit tests of crates/polaris-ui/crates/ascii-aurora/src/conv.rs (`tests`); compiled into that module via #[path].
use super::{finite_or, unit_to_u8};

#[test]
fn unit_to_u8_endpoints_and_clamps() {
    assert_eq!(unit_to_u8(0.0), 0);
    assert_eq!(unit_to_u8(1.0), 255);
    assert_eq!(unit_to_u8(-3.0), 0);
    assert_eq!(unit_to_u8(7.0), 255);
    assert_eq!(unit_to_u8(f32::NAN), 0);
    assert_eq!(unit_to_u8(f32::INFINITY), 255);
    assert_eq!(unit_to_u8(f32::NEG_INFINITY), 0);
    assert_eq!(unit_to_u8(finite_or(f32::NAN, 1.0)), 255);
}

#[test]
fn unit_to_u8_matches_rounding_everywhere() {
    for i in 0..=10_000_u16 {
        let x = f32::from(i) / 10_000.0;
        let want = (x * 255.0).round();
        let got = f32::from(unit_to_u8(x));
        assert!((got - want).abs() <= 1.0, "x={x} got={got} want={want}");
        if (x * 255.0 - (x * 255.0).floor() - 0.5).abs() > 1e-3 {
            assert!(got == want, "x={x} got={got} want={want}");
        }
    }
}
