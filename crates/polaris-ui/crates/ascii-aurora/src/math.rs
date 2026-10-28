//! Small pure math helpers: easing, hashing and value noise.

use crate::conv::{finite_or, floor_to_i32};

/// Linear interpolation.
pub(crate) fn lerp(a: f32, b: f32, w: f32) -> f32 {
    a + (b - a) * w
}

/// Hermite smoothstep between `e0` and `e1` (0 below, 1 above).
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let span = e1 - e0;
    if span.abs() <= f32::EPSILON {
        return if finite_or(x, 0.0) < e0 { 0.0 } else { 1.0 };
    }
    let w = (finite_or(x, e0) - e0) / span;
    let w = w.clamp(0.0, 1.0);
    w * w * (3.0 - 2.0 * w)
}

/// Triangle wave 0 -> 1 -> 0 with the given period (CSS `alternate`).
pub(crate) fn ping_pong(t: f32, period: f32) -> f32 {
    let x = (finite_or(t, 0.0) / period).rem_euclid(2.0);
    if x > 1.0 { 2.0 - x } else { x }
}

/// splitmix64 finaliser: a strong 64-bit mixer.
pub(crate) fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash a seed with three lattice coordinates.
pub(crate) fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let hx = u64::from(x.cast_unsigned()).wrapping_mul(0x9E37_79B1_85EB_CA87);
    let hy = u64::from(y.cast_unsigned()).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    let hz = u64::from(z.cast_unsigned()).wrapping_mul(0x1656_67B1_9E37_79F9);
    mix(seed ^ hx ^ hy.rotate_left(21) ^ hz.rotate_left(42))
}

/// 16 bits of `h` starting at `shift`, as a `u16`.
#[allow(
    clippy::manual_unwrap_or_default,
    clippy::manual_unwrap_or,
    reason = "project rule forbids the unwrap_or family"
)]
pub(crate) fn bits16(h: u64, shift: u32) -> u16 {
    match u16::try_from((h >> shift) & 0xFFFF) {
        Ok(v) => v,
        Err(_) => 0,
    }
}

/// 16 bits of `h` as a float in `[0, 1)`.
pub(crate) fn unit_from_bits(h: u64, shift: u32) -> f32 {
    f32::from(bits16(h, shift)) * (1.0 / 65536.0)
}

/// Smooth 3-D value noise in `[0, 1]`.
pub(crate) fn value_noise(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let fx = x.floor();
    let fy = y.floor();
    let fz = z.floor();
    let ix = floor_to_i32(fx);
    let iy = floor_to_i32(fy);
    let iz = floor_to_i32(fz);
    let wx = smooth(x - fx);
    let wy = smooth(y - fy);
    let wz = smooth(z - fz);
    let corner = |dx: i32, dy: i32, dz: i32| -> f32 {
        unit_from_bits(
            hash3(
                seed,
                ix.wrapping_add(dx),
                iy.wrapping_add(dy),
                iz.wrapping_add(dz),
            ),
            24,
        )
    };
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), wx);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), wx);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), wx);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), wx);
    lerp(lerp(x00, x10, wy), lerp(x01, x11, wy), wz)
}

/// Quintic-free cheap fade used by the noise.
fn smooth(w: f32) -> f32 {
    w * w * (3.0 - 2.0 * w)
}

/// `Option` value or a default, without `unwrap_or`.
pub(crate) fn or_val<T>(value: Option<T>, default: T) -> T {
    match value {
        Some(v) => v,
        None => default,
    }
}
